use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Bounds, Context, Entity, IntoElement, ObjectFit, ParentElement, Pixels, Point,
    Render, RenderImage, SharedString, Styled, StyledImage, Subscription, Task, WeakEntity, Window, anchored, canvas,
    deferred, div, img, point, px,
};
use rand::Rng;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::accessibility::reduce_motion;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_platform::power::system_low_power_mode;
use rise_ui::swiftui::rotation_effect::RotationEffectModifiers;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::lottie_sticker::lottie_sticker_loader::{
    LottieStickerContainer, LottieStickerLoader, LottieStickerPayload, LottieStickerRenderSource,
};
use crate::core::ui::lottie_sticker::rise_lottie_frame_renderer::RiseLottieFirstFrameCache;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReaction;
use crate::modules::map::components::globe::map_globe_camera_link::MapGlobeCameraLink;
use crate::modules::map::components::pins::map_pin_layer::MapPinLayer;
use crate::modules::map::components::pins::map_self_pin_layer::MapSelfPinLayer;
use crate::modules::map::components::rise_map_host::{RiseMapHost, WeakRiseMapHost};
use crate::modules::map::engine::rise_map_reactions_repository::{MapReactionBatch, MapReactionInboxEntry};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerAssetType, StickerItem};
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

/// what one reaction flies as, a rendered sticker frame or the emoji itself
/// why: gpui draws a plain emoji as text in the flight so no bitmap is needed
#[derive(Clone)]
enum MapReactionSprite {
    Image(Arc<RenderImage>),
    Emoji(SharedString),
}

/// one flying reaction, keyframe animation values sampled per frame
struct MapReactionFlight {
    sprite: MapReactionSprite,
    path: [Point<f64>; 4],
    lengths: Vec<f64>,
    scale_values: [f64; 4],
    fade_key_times: [f64; 4],
    spin_from: f64,
    spin_to: f64,
    duration: f64,
    begin: Duration,
    quiet: bool,
}

/// one frame of a flight, where and how big and how turned and how opaque
struct MapReactionFlightFrame {
    position: Point<f64>,
    scale: f64,
    rotation: f64,
    opacity: f64,
}

impl MapReactionFlight {
    const SCALE_KEY_TIMES: [f64; 4] = [0.0, 0.18, 0.55, 1.0];
    const FADE_VALUES: [f64; 4] = [0.0, 1.0, 1.0, 0.0];
    const PATH_SAMPLES: usize = 32;

    fn bezier(path: &[Point<f64>; 4], t: f64) -> Point<f64> {
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        point(
            a * path[0].x + b * path[1].x + c * path[2].x + d * path[3].x,
            a * path[0].y + b * path[1].y + c * path[2].y + d * path[3].y,
        )
    }

    fn arc_lengths(path: &[Point<f64>; 4]) -> Vec<f64> {
        let mut lengths = Vec::with_capacity(Self::PATH_SAMPLES + 1);
        let mut total = 0.0;
        let mut previous = path[0];
        lengths.push(0.0);
        for index in 1..=Self::PATH_SAMPLES {
            let next = Self::bezier(path, index as f64 / Self::PATH_SAMPLES as f64);
            total += (next.x - previous.x).hypot(next.y - previous.y);
            lengths.push(total);
            previous = next;
        }
        lengths
    }

    fn paced_position(&self, progress: f64) -> Point<f64> {
        let total = self.lengths.last().copied().unwrap_or(0.0);
        if total <= 0.0 {
            return self.path[3];
        }
        let wanted = progress.clamp(0.0, 1.0) * total;
        let index = self.lengths.partition_point(|length| *length < wanted).clamp(1, Self::PATH_SAMPLES);
        let (from, to) = (self.lengths[index - 1], self.lengths[index]);
        let within = if to > from { (wanted - from) / (to - from) } else { 0.0 };
        Self::bezier(&self.path, (index as f64 - 1.0 + within) / Self::PATH_SAMPLES as f64)
    }

    fn keyframe(values: &[f64; 4], key_times: &[f64; 4], progress: f64) -> f64 {
        for index in 1..4 {
            if progress <= key_times[index] {
                let span = (key_times[index] - key_times[index - 1]).max(f64::EPSILON);
                let f = ((progress - key_times[index - 1]) / span).clamp(0.0, 1.0);
                return values[index - 1] + (values[index] - values[index - 1]) * f;
            }
        }
        values[3]
    }

    fn progress(&self, now: Duration) -> f64 {
        now.saturating_sub(self.begin).as_secs_f64() / self.duration.max(f64::EPSILON)
    }

    fn frame(&self, now: Duration) -> MapReactionFlightFrame {
        let progress = self.progress(now).clamp(0.0, 1.0);
        let opacity = Self::keyframe(&Self::FADE_VALUES, &self.fade_key_times, progress);
        if self.quiet {
            return MapReactionFlightFrame {
                position: self.path[3],
                scale: 1.0,
                rotation: 0.0,
                opacity,
            };
        }
        MapReactionFlightFrame {
            position: self.paced_position(progress),
            scale: Self::keyframe(&self.scale_values, &Self::SCALE_KEY_TIMES, progress),
            rotation: self.spin_from + (self.spin_to - self.spin_from) * progress,
            opacity,
        }
    }
}

pub struct MapReactionStage {
    map_view: Option<WeakRiseMapHost>,
    pins: Option<Weak<RefCell<MapPinLayer>>>,
    self_pin: Option<Weak<RefCell<MapSelfPinLayer>>>,
    overlay: Option<Entity<MapReactionOverlayView>>,
    layers: HashMap<Uuid, MapReactionFlight>,
    unread: HashMap<String, i64>,
    sprites: HashMap<String, MapReactionSprite>,
    generation: u64,
    sending: HashMap<Uuid, Task<()>>,
    haptic: ImpactFeedbackGenerator,
}

impl MapReactionStage {
    const EMOJI_SIDE: f64 = 34.0;

    fn maximum_in_flight() -> usize {
        usize::try_from(RiseLimits::client_value("map_reaction_particles")).unwrap_or(0)
    }

    fn sprite_limit() -> usize {
        usize::try_from(RiseLimits::client_value("map_reaction_sprite_cache")).unwrap_or(0)
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            map_view: None,
            pins: None,
            self_pin: None,
            overlay: None,
            layers: HashMap::new(),
            unread: HashMap::new(),
            sprites: HashMap::new(),
            generation: 0,
            sending: HashMap::new(),
            haptic: ImpactFeedbackGenerator::new(ImpactStyle::Light),
        }
    }

    pub fn attach(
        &mut self,
        map_view: &RiseMapHost,
        pins: Rc<RefCell<MapPinLayer>>,
        self_pin: Rc<RefCell<MapSelfPinLayer>>,
        _cx: &mut Context<Self>,
    ) {
        self.map_view = Some(map_view.downgrade());
        self.pins = Some(Rc::downgrade(&pins));
        self.self_pin = Some(Rc::downgrade(&self_pin));
        self.haptic.prepare();
    }

    pub fn detach(&mut self, map_view: &RiseMapHost, cx: &mut Context<Self>) {
        if self.map_view().is_none_or(|attached| !attached.same_map(map_view)) {
            return;
        }
        self.cancel_flights(cx);
        self.sprites.clear();
        self.map_view = None;
        self.pins = None;
        self.self_pin = None;
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.set_unread(&[], cx);
        self.sprites.clear();
        self.cancel_flights(cx);
    }

    pub fn count(&self, user_id: &str) -> i64 {
        self.unread.get(user_id).copied().unwrap_or(0)
    }

    pub fn set_unread(&mut self, entries: &[MapReactionInboxEntry], cx: &mut Context<Self>) {
        let old: HashSet<String> = self.unread.keys().cloned().collect();
        let mut unread: HashMap<String, i64> = HashMap::new();
        for entry in entries {
            let count = unread.entry(entry.sender_id.clone()).or_insert(entry.count);
            *count = (*count).max(entry.count);
        }
        self.unread = unread;
        let Some(pins) = self.pins() else {
            return;
        };
        let ids: HashSet<String> = old.into_iter().chain(self.unread.keys().cloned()).collect();
        for id in ids {
            let count = self.count(&id);
            let pin = pins.borrow().pin_view(&id);
            if let Some(pin) = pin {
                pin.update(cx, |pin, cx| pin.set_reaction_count(count, true, cx));
            }
        }
    }

    pub fn cancel_flights(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.sending.clear();
        self.layers.clear();
        cx.notify();
    }

    pub fn send(&mut self, emoji: &str, source: Point<Pixels>, user_id: &str, custom: Option<StickerItem>, cx: &mut Context<Self>) {
        if self.sending.len() >= Self::maximum_in_flight() {
            return;
        }
        let (stamp, id) = (self.generation, Uuid::new_v4());
        self.haptic.impact_occurred_with_intensity(0.6);
        let item = custom.or_else(|| {
            strip_custom_id(emoji)
                .and_then(|custom_id| StickersServicesStore::shared(cx).read(cx).custom_emoji_item(custom_id, cx))
        });
        let (emoji, user_id) = (emoji.to_string(), user_id.to_string());
        let sprite = self.sprite(emoji, item, cx);
        let task = cx.spawn(async move |this, cx| {
            let image = sprite.await;
            let _ = this.update(cx, |stage, cx| {
                stage.sending.remove(&id);
                if stage.generation != stamp {
                    return;
                }
                let (Some(image), Some(target)) = (image, stage.pin_centre(&user_id, cx)) else {
                    return;
                };
                stage.fly(image, source, target, false, 0.0, cx);
                if let Some(pin) = stage.pins().and_then(|pins| pins.borrow().pin_view(&user_id)) {
                    pin.update(cx, |pin, cx| pin.pulse(cx));
                }
            });
        });
        self.sending.insert(id, task);
    }

    pub fn play(&mut self, reaction: MapReactionBatch, user_id: String, cx: &mut Context<Self>) -> Task<()> {
        let stamp = self.generation;
        let sprite = self.sprite(reaction.emoji.clone(), reaction.sticker(), cx);
        cx.spawn(async move |this, cx| {
            let Some(image) = sprite.await else {
                return;
            };
            if this.read_with(cx, |stage, _| stage.generation != stamp).unwrap_or(true) {
                return;
            }
            let mut remaining = reaction.count;
            while remaining > 0 {
                let Ok(step) = this.update(cx, |stage, cx| stage.play_volley(&image, &user_id, remaining, stamp, cx)) else {
                    return;
                };
                let Some(volley) = step else {
                    return;
                };
                match volley {
                    MapReactionVolleyStep::Crowded => {
                        cx.background_executor().timer(Duration::from_millis(80)).await;
                        continue;
                    }
                    MapReactionVolleyStep::Launch { volley, particles, quiet, target, source } => {
                        for index in 0..particles {
                            let launched = this.update(cx, |stage, cx| {
                                if stage.generation != stamp {
                                    return false;
                                }
                                stage.launch_particle(&image, source, target, index, volley, cx);
                                true
                            });
                            if !launched.unwrap_or(false) {
                                return;
                            }
                            if particles > 1 {
                                let pause = if volley >= 50 { 5 } else { 25 };
                                cx.background_executor().timer(Duration::from_millis(pause)).await;
                            }
                        }
                        remaining -= volley;
                        let _ = this.update(cx, |stage, cx| {
                            if let Some(pin) = stage.pins().and_then(|pins| pins.borrow().pin_view(&user_id)) {
                                pin.update(cx, |pin, cx| pin.pulse(cx));
                            }
                        });
                        let rest = if quiet { 240 } else { 850 };
                        cx.background_executor().timer(Duration::from_millis(rest)).await;
                        if this.read_with(cx, |stage, _| stage.generation != stamp).unwrap_or(true) {
                            return;
                        }
                    }
                }
            }
        })
    }

    fn play_volley(
        &mut self,
        _image: &MapReactionSprite,
        user_id: &str,
        remaining: i64,
        stamp: u64,
        cx: &mut Context<Self>,
    ) -> Option<MapReactionVolleyStep> {
        if self.generation != stamp {
            return None;
        }
        let overlay = self.ensure_overlay(cx)?;
        let source = self.pin_centre(user_id, cx)?;
        let quiet = reduce_motion() || system_low_power_mode();
        let volley = if quiet { remaining.min(100) } else { MapReactionVolley::size(remaining) };
        let particles = if quiet { 1 } else { volley };
        if self.layers.len() + usize::try_from(particles).unwrap_or(0) > Self::maximum_in_flight() {
            return Some(MapReactionVolleyStep::Crowded);
        }
        let side = px(Self::EMOJI_SIDE as f32);
        let bounds = Bounds::from_corners(overlay.origin + point(side, side), overlay.bottom_right() - point(side, side));
        let visible_self = self.self_centre(cx).filter(|centre| bounds.contains(centre));
        let target = visible_self.unwrap_or_else(|| point(bounds.center().x, bounds.bottom() - px(80.)));
        Some(MapReactionVolleyStep::Launch {
            volley,
            particles,
            quiet,
            target,
            source,
        })
    }

    fn launch_particle(
        &mut self,
        image: &MapReactionSprite,
        source: Point<Pixels>,
        target: Point<Pixels>,
        index: i64,
        volley: i64,
        cx: &mut Context<Self>,
    ) {
        let Some(overlay) = self.ensure_overlay(cx) else {
            return;
        };
        let side = px(Self::EMOJI_SIDE as f32);
        let bounds = Bounds::from_corners(overlay.origin + point(side, side), overlay.bottom_right() - point(side, side));
        let mut random = rand::rng();
        let angle = random.random_range(0.0..=(2.0 * std::f64::consts::PI));
        let radius = random.random_range(70.0..=210.0);
        let x = (f64::from(f32::from(target.x)) + angle.cos() * radius)
            .clamp(f64::from(f32::from(bounds.left())), f64::from(f32::from(bounds.right())));
        let y = (f64::from(f32::from(target.y)) + angle.sin() * radius)
            .clamp(f64::from(f32::from(bounds.top())), f64::from(f32::from(bounds.bottom())));
        self.fly(image.clone(), source, point(px(x as f32), px(y as f32)), true, 0.0, cx);
        if index % 10 == 0 {
            self.haptic.impact_occurred_with_intensity((0.35 + volley as f64 / 200.0).min(0.85));
            self.haptic.prepare();
        }
    }

    fn fly(
        &mut self,
        image: MapReactionSprite,
        source: Point<Pixels>,
        target: Point<Pixels>,
        incoming: bool,
        delay: f64,
        cx: &mut Context<Self>,
    ) {
        if self.layers.len() >= Self::maximum_in_flight() {
            return;
        }
        let Some(overlay) = self.ensure_overlay(cx) else {
            return;
        };
        let quiet = reduce_motion();
        let to_f64 = |point_px: Point<Pixels>| point(f64::from(f32::from(point_px.x)), f64::from(f32::from(point_px.y)));
        let (source, target) = (to_f64(source), to_f64(target));
        let (min_x, max_x) = (f64::from(f32::from(overlay.left())) + 12.0, f64::from(f32::from(overlay.right())) - 12.0);
        let (min_y, max_y) = (f64::from(f32::from(overlay.top())) + 12.0, f64::from(f32::from(overlay.bottom())) - 12.0);
        let clamp = |p: Point<f64>| point(p.x.min(max_x).max(min_x), p.y.min(max_y).max(min_y));
        let mut random = rand::rng();
        let distance = 80.0_f64.max((target.x - source.x).hypot(target.y - source.y));
        let angle = random.random_range(0.0..=(2.0 * std::f64::consts::PI));
        let reach = distance * random.random_range(0.25..=0.9);
        let c1 = clamp(point(source.x + angle.cos() * reach, source.y + angle.sin() * reach));
        let c2 = clamp(point(
            target.x + random.random_range(-distance..=distance) * 0.5,
            target.y + random.random_range(-distance..=distance) * 0.5,
        ));
        let path = [source, c1, c2, target];
        let flight = MapReactionFlight {
            sprite: image,
            lengths: MapReactionFlight::arc_lengths(&path),
            path,
            scale_values: if incoming { [0.45, 1.25, 0.95, 0.3] } else { [0.65, 1.2, 0.9, 0.4] },
            fade_key_times: [0.0, 0.08, if incoming { 0.38 } else { 0.62 }, 1.0],
            spin_from: random.random_range(-0.7..=0.7),
            spin_to: random.random_range(-1.4..=1.4),
            duration: if quiet { 0.24 } else { random.random_range(0.7..=1.08) },
            begin: RiseClock::monotonic() + Duration::from_secs_f64(delay.max(0.0)),
            quiet,
        };
        self.layers.insert(Uuid::new_v4(), flight);
        cx.notify();
    }

    fn retire_finished(&mut self, now: Duration) {
        self.layers.retain(|_, flight| flight.progress(now) < 1.0);
    }

    fn sprite(&mut self, emoji: String, custom: Option<StickerItem>, cx: &mut Context<Self>) -> Task<Option<MapReactionSprite>> {
        if let Some(hit) = self.sprites.get(&emoji) {
            return Task::ready(Some(hit.clone()));
        }
        let (account, epoch) = (SaiAccountCacheScope::active_user_id(), SaiAccountCacheScope::active_epoch());
        let stamp = self.generation;
        let side = gpui::size(px(Self::EMOJI_SIDE as f32), px(Self::EMOJI_SIDE as f32));
        cx.spawn(async move |this, cx| {
            let result = match custom {
                Some(custom) => Self::custom_sprite(custom, side, account.clone(), epoch.clone(), cx).await,
                None if strip_custom_id(&emoji).is_none() => Some(MapReactionSprite::Emoji(emoji.clone().into())),
                None => None,
            };
            this.update(cx, |stage, _| {
                if stage.generation != stamp
                    || !SaiAccountCacheScope::is_current_account_epoch(account.as_deref(), epoch.as_deref())
                {
                    return None;
                }
                let result = result?;
                if stage.sprites.len() >= Self::sprite_limit() {
                    stage.sprites.clear();
                }
                stage.sprites.insert(emoji, result.clone());
                Some(result)
            })
            .ok()
            .flatten()
        })
    }

    async fn custom_sprite(
        custom: StickerItem,
        side: gpui::Size<Pixels>,
        account: Option<String>,
        epoch: Option<String>,
        cx: &mut AsyncApp,
    ) -> Option<MapReactionSprite> {
        let mut result = None;
        if custom.asset_type == StickerAssetType::Lottie
            && let Ok(payload) = LottieStickerLoader::shared().payload(&custom.file_url).await
        {
            let source = match payload {
                LottieStickerPayload::Animation(prepared) => Some(prepared),
                LottieStickerPayload::DotLottie(file) => file.animations.first().map(|entry| LottieStickerRenderSource {
                    json_data: Arc::from(Vec::new()),
                    cache_key: custom.file_url.clone(),
                    container: LottieStickerContainer::DotLottie,
                    animation: entry.animation.clone(),
                }),
            };
            if let Some(source) = source {
                let cache_key = source.cache_key.clone();
                RiseLottieFirstFrameCache::prepare(source, side, account.clone(), epoch.clone(), None).await;
                result = RiseLottieFirstFrameCache::memory_image(&cache_key, side, account.as_deref())
                    .map(MapReactionSprite::Image);
            }
        }
        if result.is_none() {
            let url = if custom.asset_type == StickerAssetType::StaticImage {
                Some(custom.file_url.clone())
            } else {
                custom.thumbnail_url.clone()
            };
            if let Some(url) = url.filter(|url| !url.is_empty()) {
                result = CleverImageBitmapLoader::load(
                    &url,
                    true,
                    true,
                    account,
                    epoch,
                    Some(102),
                    CleverImageCachePolicy::Standard,
                    CleverImageWorkLane::Visible,
                    cx,
                )
                .await
                .map(MapReactionSprite::Image);
            }
        }
        result
    }

    /// the overlay's window frame once the map is in a window
    /// why: a gpui element cant add itself to a window
    /// so the map host mounts the overlay for the map's whole life
    fn ensure_overlay(&mut self, cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        self.map_view()?;
        let bounds = self.overlay.as_ref()?.read(cx).bounds;
        (bounds.size.width > Pixels::ZERO).then_some(bounds)
    }

    fn pin_centre(&self, user_id: &str, cx: &mut App) -> Option<Point<Pixels>> {
        let map_view = self.map_view()?;
        let globe_shown = MapInteractionsStore::shared(cx).read(cx).globe_entry.is_some();
        if globe_shown && let Some(globe) = MapGlobeCameraLink::shared(cx).read(cx).globe() {
            let globe = globe.read(cx);
            return globe.reaction_anchor(user_id).or_else(|| Some(globe.window_centre()));
        }
        let pins = self.pins()?;
        let view = pins.borrow().pin_view(user_id);
        if let Some(centre) = view.and_then(|view| view.read(cx).window_centre()) {
            return Some(centre);
        }
        let coordinate = pins.borrow().coordinate(user_id)?;
        let local = map_view.convert_coordinate_to_point(coordinate)?;
        let origin = map_view.convert_window_point(point(px(0.), px(0.)));
        Some(point(px((local.x - origin.x) as f32), px((local.y - origin.y) as f32)))
    }

    fn self_centre(&self, cx: &mut App) -> Option<Point<Pixels>> {
        self.map_view()?;
        if MapInteractionsStore::shared(cx).read(cx).globe_entry.is_some()
            && let Some(user_id) = SaiAccountCacheScope::active_user_id()
        {
            let globe = MapGlobeCameraLink::shared(cx).read(cx).globe()?;
            return globe.read(cx).reaction_anchor(&user_id);
        }
        let view = self.self_pin.as_ref().and_then(Weak::upgrade)?.borrow().current_view()?;
        view.read(cx).window_centre()
    }

    fn map_view(&self) -> Option<RiseMapHost> {
        self.map_view.as_ref().and_then(WeakRiseMapHost::upgrade)
    }

    fn pins(&self) -> Option<Rc<RefCell<MapPinLayer>>> {
        self.pins.as_ref().and_then(Weak::upgrade)
    }

    /// overlay the map host mounts over the window, built on first use
    pub fn overlay(&mut self, cx: &mut Context<Self>) -> Entity<MapReactionOverlayView> {
        if let Some(overlay) = self.overlay.clone() {
            return overlay;
        }
        let stage = cx.entity();
        let overlay = cx.new(|cx| MapReactionOverlayView::new(stage, cx));
        self.overlay = Some(overlay.clone());
        overlay
    }
}

/// what the main thread decided for the next volley of play
enum MapReactionVolleyStep {
    Crowded,
    Launch {
        volley: i64,
        particles: i64,
        quiet: bool,
        target: Point<Pixels>,
        source: Point<Pixels>,
    },
}

pub enum MapReactionVolley {}

impl MapReactionVolley {
    pub fn size(remaining: i64) -> i64 {
        if remaining >= 500 {
            return 100;
        }
        if remaining >= 100 {
            return 50;
        }
        if remaining >= 10 {
            return 10;
        }
        remaining.max(0)
    }
}

fn strip_custom_id(emoji: &str) -> Option<&str> {
    emoji.strip_prefix(MessageReaction::CUSTOM_EMOJI_PREFIX)
}

/// window wide layer reactions fly in
/// no input and clear and above everything
pub struct MapReactionOverlayView {
    stage: WeakEntity<MapReactionStage>,
    bounds: Bounds<Pixels>,
    _stage_changes: Subscription,
}

impl MapReactionOverlayView {
    fn new(stage: Entity<MapReactionStage>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&stage, |_, _, cx| cx.notify());
        Self {
            stage: stage.downgrade(),
            bounds: Bounds::default(),
            _stage_changes: subscription,
        }
    }

    fn flight_element(flight: &MapReactionFlight, now: Duration) -> AnyElement {
        let frame = flight.frame(now);
        let side = (MapReactionStage::EMOJI_SIDE * frame.scale) as f32;
        let origin = point(px(frame.position.x as f32 - side / 2.0), px(frame.position.y as f32 - side / 2.0));
        let content = match &flight.sprite {
            MapReactionSprite::Image(image) => img(image.clone()).size_full().object_fit(ObjectFit::Contain).into_any_element(),
            MapReactionSprite::Emoji(emoji) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(side * 0.82))
                .child(emoji.clone())
                .into_any_element(),
        };
        div()
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .size(px(side))
            .opacity(frame.opacity as f32)
            .child(content.rotation_effect(frame.rotation.to_degrees() as f32))
            .into_any_element()
    }
}

impl Render for MapReactionOverlayView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let viewport = window.viewport_size();
        let this = cx.entity().downgrade();
        let flights: Vec<AnyElement> = match self.stage.upgrade() {
            Some(stage) => {
                stage.update(cx, |stage, _| {
                    stage.retire_finished(now);
                    stage.layers.values().map(|flight| Self::flight_element(flight, now)).collect()
                })
            }
            None => Vec::new(),
        };
        if !flights.is_empty() {
            window.request_animation_frame();
        }
        let measure = canvas(
            move |bounds, _, cx| {
                let _ = this.update(cx, |overlay, _| overlay.bounds = bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(viewport.width)
                    .h(viewport.height)
                    .relative()
                    .child(measure)
                    .when(!flights.is_empty(), |layer| layer.children(flights)),
            ),
        )
        .with_priority(1)
    }
}
