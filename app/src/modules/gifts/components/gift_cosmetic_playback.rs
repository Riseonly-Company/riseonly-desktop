use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    div, img, px, AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, Element, ElementId, Entity,
    EntityId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, ObjectFit, ParentElement as _, Pixels,
    Point, Render, RenderOnce, Size, Styled as _, StyleRefinement, StyledImage as _, Subscription, Task, WeakEntity,
    Window, WindowId,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_media::video::av_player::{
    AVLayerVideoGravity, AVPlayer, AVPlayerActionAtItemEnd, AVPlayerItem, AVPlayerItemNotification,
    AVPlayerItemStatus, AVPlayerLayer,
};
use rise_ui::animated_media_playback_scope::AnimatedMediaViewportPlaybackPolicy;
use rise_ui::animated_tabs_ui::{AnimatedTabsStripActivity, AnimatedTabsStripDidRest, AnimatedTabsStripMotionDriver};
use rise_ui::swiftui::environment::{env, reduce_motion, EnvKey};
use rise_ui::top_presenter::{InsidePresentedLayerKey, TopPresenter};
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaFileLease};
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::core::ui::animated_media_playback_coordinator::{
    AnimatedMediaParticipantId, AnimatedMediaPlaybackCoordinator, AnimatedMediaPlaybackKind,
};
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListMountEnvelopePolicy;
use crate::modules::gifts::components::gift_cosmetic_files::GiftCosmeticFiles;

/// rectangle in unit coordinates of the whole media
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GiftCosmeticUnitRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl GiftCosmeticUnitRect {
    pub const UNIT: Self = Self { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };

    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }
}

/// part of a cosmetic media one surface shows
/// a frame media is two square layers side by side, left half behind the avatar and right
/// half in front
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GiftCosmeticPart {
    #[default]
    Whole,
    Back,
    Front,
}

impl GiftCosmeticPart {
    pub fn source_rect(self) -> GiftCosmeticUnitRect {
        match self {
            Self::Whole => GiftCosmeticUnitRect::UNIT,
            Self::Back => GiftCosmeticUnitRect::new(0.0, 0.0, 0.5, 1.0),
            Self::Front => GiftCosmeticUnitRect::new(0.5, 0.0, 0.5, 1.0),
        }
    }
}

/// how a surface places its part in its box
/// fill and crop the overflow evenly, whole and centred or full height against the trailing
/// edge like a nameplate
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GiftCosmeticPlacement {
    #[default]
    Fill,
    Fit,
    TrailingFullHeight,
}

pub enum GiftCosmeticGeometry {}

impl GiftCosmeticGeometry {
    pub const FRAME_LAYER_ASPECT: f32 = 1.0;
    pub const PLATE_ASPECT: f32 = 784.0 / 112.0;
    pub const BANNER_ASPECT: f32 = 786.0 / 518.0;

    pub const MEMBER_ROW_HEIGHT: f32 = 56.0;

    /// side of the frame canvas per avatar diameter
    pub fn frame_canvas_scale() -> f32 {
        rise_gifts::FRAME_CANVAS_SCALE_PERMILLE as f32 / 1_000.0
    }
}

/// where a surface draws inside its own box
/// drawn is the clip and media is the whole media placed relative to it, in pixels of the
/// surface box
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GiftCosmeticSurfaceLayout {
    pub drawn: GiftCosmeticUnitRect,
    pub media: GiftCosmeticUnitRect,
}

impl GiftCosmeticSurfaceLayout {
    /// gives the clip and the shifted media for a box of width x height, none for an empty box
    pub fn resolve(
        part: GiftCosmeticPart,
        placement: GiftCosmeticPlacement,
        aspect: f32,
        width: f32,
        height: f32,
    ) -> Option<Self> {
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let aspect = aspect.max(0.01);
        let mut drawn = GiftCosmeticUnitRect::new(0.0, 0.0, width, height);
        let mut visible = GiftCosmeticUnitRect::UNIT;
        match placement {
            GiftCosmeticPlacement::Fill => {
                let box_aspect = width / height;
                if box_aspect > aspect {
                    let visible_height = aspect / box_aspect;
                    visible = GiftCosmeticUnitRect::new(0.0, (1.0 - visible_height) / 2.0, 1.0, visible_height);
                } else {
                    let visible_width = box_aspect / aspect;
                    visible = GiftCosmeticUnitRect::new((1.0 - visible_width) / 2.0, 0.0, visible_width, 1.0);
                }
            }
            GiftCosmeticPlacement::Fit => {
                let box_aspect = width / height;
                if box_aspect > aspect {
                    let drawn_width = height * aspect;
                    drawn = GiftCosmeticUnitRect::new((width - drawn_width) / 2.0, 0.0, drawn_width, height);
                } else {
                    let drawn_height = width / aspect;
                    drawn = GiftCosmeticUnitRect::new(0.0, (height - drawn_height) / 2.0, width, drawn_height);
                }
            }
            GiftCosmeticPlacement::TrailingFullHeight => {
                let full_width = height * aspect;
                if full_width <= width {
                    drawn = GiftCosmeticUnitRect::new(width - full_width, 0.0, full_width, height);
                } else {
                    let visible_width = width / full_width;
                    visible = GiftCosmeticUnitRect::new(1.0 - visible_width, 0.0, visible_width, 1.0);
                }
            }
        }
        let source = part.source_rect();
        let region = GiftCosmeticUnitRect::new(
            source.x + visible.x * source.width,
            source.y + visible.y * source.height,
            visible.width * source.width,
            visible.height * source.height,
        );
        if region.width <= 0.0 || region.height <= 0.0 {
            return None;
        }
        let media_width = drawn.width / region.width;
        let media_height = drawn.height / region.height;
        Some(Self {
            drawn,
            media: GiftCosmeticUnitRect::new(
                -region.x * media_width,
                -region.y * media_height,
                media_width,
                media_height,
            ),
        })
    }
}

/// how a cosmetic movie runs, once by default or in a seamless loop
/// for effect previews that want a loop it rests on its empty last frame between plays
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GiftCosmeticMotion {
    Loop,
    Repeating { rest: Duration },
    Once,
}

impl GiftCosmeticMotion {
    pub const EFFECT_PREVIEW: Self = Self::Repeating { rest: Duration::from_secs(1) };
}

/// whether gifts and cosmetics play once each time they come into view or keep moving
/// they play once everywhere like telegram since looping movies kept a decoder busy for
/// every card and avatar
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GiftMotionStyle {
    Continuous,
    #[default]
    OncePerAppearance,
}

impl GiftMotionStyle {
    /// motion of a movie that otherwise runs as continuous motion
    pub fn motion(self, continuous: GiftCosmeticMotion) -> GiftCosmeticMotion {
        match self {
            Self::Continuous => continuous,
            Self::OncePerAppearance => GiftCosmeticMotion::Once,
        }
    }

    pub fn replays_on_return(self) -> bool {
        self == Self::OncePerAppearance
    }
}

pub struct GiftMotionStyleKey;

impl EnvKey for GiftMotionStyleKey {
    type Value = GiftMotionStyle;

    fn default_value() -> Self::Value {
        GiftMotionStyle::OncePerAppearance
    }
}

/// when gifts and cosmetics move, always wherever they are on screen
/// power saving doesnt stop what a person paid for, reduce motion does
pub enum GiftCosmeticMotionPolicy {}

impl GiftCosmeticMotionPolicy {
    pub fn allows_motion(cx: &App) -> bool {
        !reduce_motion(cx)
    }

    /// why a cosmetic on screen stays still, for the debugger
    pub fn still_reason(cx: &App) -> &'static str {
        if reduce_motion(cx) {
            return "Reduce Motion is on";
        }
        "no playback slot free"
    }
}

/// what one playback shows and how it runs
#[derive(Clone, Debug, PartialEq)]
pub struct GiftCosmeticPlaybackSource {
    pub poster: Option<String>,
    pub movie: Option<Url>,
    pub motion: GiftCosmeticMotion,
    pub animated: bool,
    pub rests_on_poster: bool,
    pub poster_pixels: i64,
    pub replay: i64,
    pub replays_on_return: bool,
    pub fades_poster: bool,
    pub preview_poster: Option<(String, i64)>,
}

impl GiftCosmeticPlaybackSource {
    /// constructor with defaults for the trailing fields
    pub fn new(poster: Option<String>, movie: Option<Url>, motion: GiftCosmeticMotion, animated: bool) -> Self {
        Self {
            poster,
            movie,
            motion,
            animated,
            rests_on_poster: true,
            poster_pixels: 768,
            replay: 0,
            replays_on_return: false,
            fades_poster: false,
            preview_poster: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GiftCosmeticPlaybackSlotState {
    None,
    Waiting,
    Granted,
}

/// one decoder for every surface of one gift or cosmetic, poster decoded once and shared
/// one player backs each surface picture so layers never drift
/// runs only while on screen from disk cache, poster stands in
pub struct GiftCosmeticPlayback {
    participant: AnimatedMediaParticipantId,
    source: Option<GiftCosmeticPlaybackSource>,
    surfaces: HashMap<EntityId, WeakEntity<GiftCosmeticSurfaceView>>,
    poster: Option<CleverImage>,
    poster_task: Option<Task<()>>,
    player: Option<Entity<AVPlayer>>,
    lease: Option<RemoteMediaFileLease>,
    failure_observation: Option<Subscription>,
    did_note_failure: bool,
    did_note_blocked: bool,
    load_task: Option<Task<()>>,
    generation: u64,
    slot: GiftCosmeticPlaybackSlotState,
    is_visible: bool,
    did_finish_once: bool,
    ready_surfaces: HashSet<EntityId>,
    end_observer: Option<Subscription>,
    awaits_replay: bool,
    rest_task: Option<Task<()>>,
    once_deadline_task: Option<Task<()>>,
    awaits_first_frame: bool,
    did_start_once: bool,
}

impl GiftCosmeticPlayback {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            participant: AnimatedMediaParticipantId::next(),
            source: None,
            surfaces: HashMap::new(),
            poster: None,
            poster_task: None,
            player: None,
            lease: None,
            failure_observation: None,
            did_note_failure: false,
            did_note_blocked: false,
            load_task: None,
            generation: 0,
            slot: GiftCosmeticPlaybackSlotState::None,
            is_visible: false,
            did_finish_once: false,
            ready_surfaces: HashSet::new(),
            end_observer: None,
            awaits_replay: false,
            rest_task: None,
            once_deadline_task: None,
            awaits_first_frame: false,
            did_start_once: false,
        }
    }

    pub fn attach(&mut self, surface: &Entity<GiftCosmeticSurfaceView>, cx: &mut Context<Self>) {
        self.surfaces.insert(surface.entity_id(), surface.downgrade());
        let playback = cx.weak_entity();
        let poster = self.poster.clone();
        let player = self.player.clone();
        let tracks = self.tracks_visibility();
        surface.update(cx, |surface, cx| {
            surface.playback = Some(playback);
            surface.set_poster(poster, cx);
            surface.set_player(player, cx);
        });
        self.apply_visibility_of_layers(cx);
        surface.update(cx, |surface, cx| surface.set_tracks_visibility(tracks, cx));
        self.surface_visibility_changed(cx);
    }

    /// surface is none when the surface itself is going away and cant be touched anymore
    pub fn detach(
        &mut self,
        key: EntityId,
        surface: Option<&Entity<GiftCosmeticSurfaceView>>,
        cx: &mut Context<Self>,
    ) {
        self.surfaces.remove(&key);
        self.ready_surfaces.remove(&key);
        if let Some(surface) = surface {
            let me = cx.entity_id();
            surface.update(cx, |surface, cx| {
                surface.set_player(None, cx);
                if surface.playback.as_ref().is_some_and(|playback| playback.entity_id() == me) {
                    surface.playback = None;
                }
            });
        }
        if self.live_surfaces().is_empty() {
            self.teardown(cx);
        } else {
            self.surface_visibility_changed(cx);
        }
    }

    fn live_surfaces(&self) -> Vec<Entity<GiftCosmeticSurfaceView>> {
        self.surfaces.values().filter_map(WeakEntity::upgrade).collect()
    }

    /// only a source that may play makes its surfaces watch the screen
    /// a poster never needs it and a list of avatars would pay for a visibility pulse each
    fn tracks_visibility(&self) -> bool {
        self.source.as_ref().is_some_and(|source| source.animated && source.movie.is_some())
    }

    pub fn surface_visibility_changed(&mut self, cx: &mut Context<Self>) {
        let live = self.live_surfaces();
        if live.iter().any(|surface| {
            let surface = surface.read(cx);
            surface.is_in_moving_strip(cx) || surface.is_in_moving_list()
        }) {
            return;
        }
        if self.did_finish_once
            && self.source.as_ref().is_some_and(|source| source.replays_on_return)
            && !live.iter().any(|surface| surface.read(cx).is_in_viewport())
        {
            self.did_finish_once = false;
        }
        let visible = live.iter().any(|surface| surface.read(cx).is_on_screen());
        if visible == self.is_visible {
            return;
        }
        self.is_visible = visible;
        self.refresh_playback(cx);
    }

    pub fn surface_ready_changed(&mut self, key: EntityId, player: EntityId, ready: bool, cx: &mut Context<Self>) {
        if self.player.as_ref().map(Entity::entity_id) != Some(player) {
            return;
        }
        if ready {
            self.ready_surfaces.insert(key);
        } else {
            self.ready_surfaces.remove(&key);
        }
        if !self.awaits_first_frame || !self.is_video_ready() {
            self.apply_visibility_of_layers(cx);
            return;
        }
        self.awaits_first_frame = false;
        self.apply_visibility_of_layers(cx);
        if self.slot == GiftCosmeticPlaybackSlotState::Granted && self.wants_to_play(cx) {
            self.start_or_resume(cx);
        }
    }

    fn is_video_ready(&self) -> bool {
        let live = self.live_surfaces();
        self.player.is_some()
            && !live.is_empty()
            && live.iter().all(|surface| self.ready_surfaces.contains(&surface.entity_id()))
    }

    pub fn update(&mut self, next: GiftCosmeticPlaybackSource, cx: &mut Context<Self>) {
        if self.source.as_ref() == Some(&next) {
            return;
        }
        let previous = self.source.replace(next.clone());
        let tracked = previous
            .as_ref()
            .is_some_and(|source| source.animated && source.movie.is_some());

        if previous.as_ref().map(|source| (&source.poster, source.poster_pixels))
            != Some((&next.poster, next.poster_pixels))
        {
            self.load_poster(next.poster.clone(), next.poster_pixels, next.preview_poster.clone(), cx);
        }
        if previous.as_ref().map(|source| (&source.movie, source.motion)) != Some((&next.movie, next.motion))
            || !next.animated
        {
            self.stop_player(cx);
            self.did_finish_once = false;
        }
        if let Some(previous) = &previous
            && previous.replay != next.replay
            && next.motion == GiftCosmeticMotion::Once
        {
            self.stop_player(cx);
            self.did_finish_once = false;
        }
        self.apply_visibility_of_layers(cx);
        let tracks = self.tracks_visibility();
        if tracks != tracked {
            for surface in self.live_surfaces() {
                surface.update(cx, |surface, cx| surface.set_tracks_visibility(tracks, cx));
            }
        }
        self.refresh_playback(cx);
    }

    fn load_poster(
        &mut self,
        url: Option<String>,
        pixels: i64,
        preview: Option<(String, i64)>,
        cx: &mut Context<Self>,
    ) {
        self.poster_task = None;
        let Some(url) = url.filter(|url| !url.is_empty()) else {
            self.set_poster(None, cx);
            return;
        };
        let user_id = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        if let Some(cached) = CleverImageBitmapLoader::memory_image(
            &url,
            true,
            user_id.as_deref(),
            epoch.as_deref(),
            Some(pixels),
            CleverImageCachePolicy::Standard,
            None,
            false,
        ) {
            self.set_poster(Some(cached), cx);
            return;
        }
        let preview = preview.filter(|(preview, _)| !preview.is_empty() && *preview != url).and_then(|(preview, pixels)| {
            CleverImageBitmapLoader::memory_image(
                &preview,
                true,
                user_id.as_deref(),
                epoch.as_deref(),
                Some(pixels),
                CleverImageCachePolicy::Standard,
                None,
                false,
            )
        });
        self.set_poster(preview, cx);
        self.poster_task = Some(cx.spawn(async move |this, cx| {
            let image = CleverImageBitmapLoader::load(
                &url,
                true,
                true,
                user_id,
                epoch,
                Some(pixels),
                CleverImageCachePolicy::Standard,
                CleverImageWorkLane::Visible,
                cx,
            )
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.source.as_ref().and_then(|source| source.poster.as_deref()) != Some(url.as_str()) {
                    return;
                }
                if image.is_some() || this.poster.is_none() {
                    this.set_poster(image, cx);
                }
            });
        }));
    }

    fn set_poster(&mut self, image: Option<CleverImage>, cx: &mut Context<Self>) {
        self.poster = image.clone();
        for surface in self.live_surfaces() {
            let image = image.clone();
            surface.update(cx, |surface, cx| surface.set_poster(image, cx));
        }
    }

    fn wants_to_play(&self, cx: &App) -> bool {
        let Some(source) = &self.source else { return false };
        if !source.animated || source.movie.is_none() || !self.is_visible {
            return false;
        }
        match source.motion {
            GiftCosmeticMotion::Loop | GiftCosmeticMotion::Repeating { .. } => {
                GiftCosmeticMotionPolicy::allows_motion(cx)
            }
            GiftCosmeticMotion::Once => !self.did_finish_once && GiftCosmeticMotionPolicy::allows_motion(cx),
        }
    }

    fn refresh_playback(&mut self, cx: &mut Context<Self>) {
        if !self.wants_to_play(cx) {
            self.note_if_held_still(cx);
            self.pause(cx);
            self.release_slot(cx);
            if self.source.as_ref().is_some_and(|source| source.motion == GiftCosmeticMotion::Once)
                && !self.did_finish_once
            {
                self.stop_player(cx);
            }
            self.apply_visibility_of_layers(cx);
            return;
        }
        self.request_slot(cx);
    }

    fn request_slot(&mut self, cx: &mut Context<Self>) {
        match self.slot {
            GiftCosmeticPlaybackSlotState::Granted => self.start_or_resume(cx),
            GiftCosmeticPlaybackSlotState::Waiting => {}
            GiftCosmeticPlaybackSlotState::None => {
                self.slot = GiftCosmeticPlaybackSlotState::Waiting;
                let id = self.participant;
                let granted = cx.weak_entity();
                let revoked = cx.weak_entity();
                AnimatedMediaPlaybackCoordinator::shared(cx).update(cx, |coordinator, cx| {
                    coordinator.request(
                        id,
                        AnimatedMediaPlaybackKind::Gift,
                        move |_, cx| {
                            if granted.update(cx, |this, cx| this.slot_granted(cx)).is_err() {
                                AnimatedMediaPlaybackCoordinator::shared(cx)
                                    .update(cx, |coordinator, cx| coordinator.release(id, None, cx));
                            }
                        },
                        move |_, cx| {
                            let _ = revoked.update(cx, |this, cx| this.slot_revoked(cx));
                        },
                        cx,
                    );
                });
                let this = cx.weak_entity();
                cx.defer(move |cx| {
                    let _ = this.update(cx, |this, cx| {
                        if this.slot == GiftCosmeticPlaybackSlotState::Waiting {
                            let reason = GiftCosmeticMotionPolicy::still_reason(cx);
                            this.note_held_still(reason, cx);
                        }
                    });
                });
            }
        }
    }

    fn note_if_held_still(&mut self, cx: &mut Context<Self>) {
        let Some(source) = &self.source else { return };
        if !source.animated || source.movie.is_none() || !self.is_visible {
            return;
        }
        if source.motion == GiftCosmeticMotion::Once && self.did_finish_once {
            return;
        }
        let reason = GiftCosmeticMotionPolicy::still_reason(cx);
        self.note_held_still(reason, cx);
    }

    fn note_held_still(&mut self, reason: &str, cx: &mut Context<Self>) {
        if self.did_note_blocked {
            return;
        }
        self.did_note_blocked = true;
        let name = self
            .source
            .as_ref()
            .and_then(|source| source.movie.as_ref())
            .map_or_else(|| "?".to_owned(), Url::last_path_component);
        GiftCosmeticFiles::note(format!("held still ({reason}): {name}"), cx);
    }

    fn slot_granted(&mut self, cx: &mut Context<Self>) {
        if !self.wants_to_play(cx) {
            self.release_slot(cx);
            return;
        }
        self.slot = GiftCosmeticPlaybackSlotState::Granted;
        self.start_or_resume(cx);
    }

    fn slot_revoked(&mut self, cx: &mut Context<Self>) {
        self.slot = GiftCosmeticPlaybackSlotState::Waiting;
        self.pause(cx);
    }

    fn release_slot(&mut self, cx: &mut Context<Self>) {
        if self.slot == GiftCosmeticPlaybackSlotState::None {
            return;
        }
        self.slot = GiftCosmeticPlaybackSlotState::None;
        let id = self.participant;
        AnimatedMediaPlaybackCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.release(id, None, cx));
    }

    fn start_or_resume(&mut self, cx: &mut Context<Self>) {
        if let Some(player) = self.player.clone() {
            if self.awaits_first_frame {
                return;
            }
            if self.awaits_replay {
                self.replay_if_due(cx);
            } else {
                if self.source.as_ref().is_some_and(|source| source.motion == GiftCosmeticMotion::Once)
                    && !self.did_start_once
                {
                    self.did_start_once = true;
                    self.begin_once_play(&player, cx);
                }
                player.update(cx, |player, cx| player.play(cx));
            }
            return;
        }
        if self.load_task.is_some() {
            return;
        }
        let Some(source) = self.source.clone() else { return };
        let Some(movie) = source.movie.clone() else { return };
        self.generation = self.generation.wrapping_add(1);
        let expected = self.generation;
        let motion = source.motion;
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let local = GiftCosmeticFiles::local(&movie, cx).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != expected {
                    return;
                }
                if local.is_none() {
                    GiftCosmeticFiles::note(
                        format!("movie not cached, playing from the network: {}", movie.last_path_component()),
                        cx,
                    );
                }
                let weak = cx.weak_entity();
                GiftFramePacer::shared(cx).update(cx, |pacer, cx| {
                    pacer.enqueue_player(
                        Box::new(move |cx: &mut App| {
                            weak.update(cx, |this, cx| this.build_player(expected, movie, local, motion, cx))
                                .unwrap_or(false)
                        }),
                        cx,
                    );
                });
            });
        }));
    }

    fn build_player(
        &mut self,
        expected: u64,
        movie: Url,
        local: Option<Url>,
        motion: GiftCosmeticMotion,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.generation != expected || self.player.is_some() {
            return false;
        }
        self.load_task = None;
        if !self.wants_to_play(cx) {
            return false;
        }
        if !movie.is_file_url()
            && let Some(local) = &local
        {
            self.lease = RemoteMediaDiskCache::lease(local.to_file_path().as_deref());
        }
        self.install(AVPlayerItem::new(local.unwrap_or(movie)), motion, cx);
        true
    }

    fn install(&mut self, mut item: AVPlayerItem, motion: GiftCosmeticMotion, cx: &mut Context<Self>) {
        item.set_preferred_forward_buffer_duration(Duration::from_secs(2));
        let expected = self.generation;
        let action = match motion {
            GiftCosmeticMotion::Loop => AVPlayerActionAtItemEnd::None,
            GiftCosmeticMotion::Repeating { .. } | GiftCosmeticMotion::Once => AVPlayerActionAtItemEnd::Pause,
        };
        let player = cx.new(AVPlayer::new);
        player.update(cx, |player, cx| {
            player.set_action_at_item_end(action, cx);
            player.set_is_muted(true, cx);
            player.set_automatically_waits_to_minimize_stalling(true, cx);
            player.replace_current_item(Some(item), cx);
        });
        if motion == GiftCosmeticMotion::Once {
            self.awaits_first_frame = true;
        }
        self.end_observer = Some(cx.subscribe(&player, move |this, _, _: &AVPlayerItemNotification, cx| {
            match motion {
                GiftCosmeticMotion::Loop => this.loop_from_start(expected, cx),
                GiftCosmeticMotion::Repeating { rest } => this.rest(rest, expected, cx),
                GiftCosmeticMotion::Once => this.finish_once(expected, cx),
            }
        }));
        self.did_note_failure = false;
        self.failure_observation = Some(cx.observe(&player, |this, player, cx| this.note_if_failed(&player, cx)));
        self.player = Some(player.clone());
        self.ready_surfaces.clear();
        for surface in self.live_surfaces() {
            let player = player.clone();
            surface.update(cx, |surface, cx| surface.set_player(Some(player), cx));
        }
        self.apply_visibility_of_layers(cx);
        if !self.awaits_first_frame && self.slot == GiftCosmeticPlaybackSlotState::Granted && self.wants_to_play(cx) {
            player.update(cx, |player, cx| player.play(cx));
        }
    }

    fn note_if_failed(&mut self, player: &Entity<AVPlayer>, cx: &mut Context<Self>) {
        if self.did_note_failure || self.player.as_ref() != Some(player) {
            return;
        }
        let (status, reason, name) = {
            let player = player.read(cx);
            (
                player.current_item_status(),
                player.error().map_or_else(|| "unknown".to_owned(), ToString::to_string),
                player
                    .current_item()
                    .map_or_else(|| "?".to_owned(), |item| item.url().last_path_component()),
            )
        };
        if status != AVPlayerItemStatus::Failed {
            return;
        }
        self.did_note_failure = true;
        GiftCosmeticFiles::note(format!("movie failed to play: {name}: {reason}"), cx);
    }

    fn begin_once_play(&mut self, player: &Entity<AVPlayer>, cx: &mut Context<Self>) {
        let duration = player.read(cx).current_item_duration().unwrap_or(f64::NAN);
        let seconds = if duration.is_finite() && duration > 0.0 {
            duration
        } else {
            f64::from(rise_gifts::PROFILE_EFFECT_DURATION_MS) / 1_000.0
        };
        if self
            .source
            .as_ref()
            .is_some_and(|source| source.fades_poster && source.rests_on_poster)
        {
            for surface in self.live_surfaces() {
                surface.update(cx, |surface, cx| surface.dissolve_poster(seconds, cx));
            }
        }
        let expected = self.generation;
        self.once_deadline_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(seconds + 3.0)).await;
            let _ = this.update(cx, |this, cx| {
                this.finish_once(expected, cx);
            });
        }));
    }

    /// the poster comes back over the held last frame first and the player goes a pass later
    /// a player let go in the same pass could empty its picture a frame before the poster showed
    fn finish_once(&mut self, expected: u64, cx: &mut Context<Self>) {
        if expected != self.generation
            || self.player.is_none()
            || self.source.as_ref().is_none_or(|source| source.motion != GiftCosmeticMotion::Once)
        {
            return;
        }
        self.did_finish_once = true;
        self.release_slot(cx);
        let rests_on_poster = self.source.as_ref().is_none_or(|source| source.rests_on_poster);
        for surface in self.live_surfaces() {
            surface.update(cx, |surface, cx| surface.set_layers(!rests_on_poster, false, cx));
        }
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                if this.generation != expected {
                    return;
                }
                this.stop_player(cx);
                this.apply_visibility_of_layers(cx);
            });
        })
        .detach();
    }

    fn loop_from_start(&mut self, expected: u64, cx: &mut Context<Self>) {
        if expected != self.generation {
            return;
        }
        self.seek_to_start_and_play(cx);
    }

    /// repeating mode, the play hit its empty last frame which stays up for rest
    fn rest(&mut self, rest: Duration, expected: u64, cx: &mut Context<Self>) {
        if expected != self.generation || self.player.is_none() {
            return;
        }
        self.awaits_replay = true;
        self.rest_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(rest).await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != expected {
                    return;
                }
                this.rest_task = None;
                this.replay_if_due(cx);
            });
        }));
    }

    /// starts the next play once the rest is over while the preview may still play
    /// a preview that left the screen starts when its back
    fn replay_if_due(&mut self, cx: &mut Context<Self>) {
        if !self.awaits_replay
            || self.rest_task.is_some()
            || self.slot != GiftCosmeticPlaybackSlotState::Granted
            || !self.wants_to_play(cx)
            || self.player.is_none()
        {
            return;
        }
        self.awaits_replay = false;
        self.seek_to_start_and_play(cx);
    }

    fn seek_to_start_and_play(&mut self, cx: &mut Context<Self>) {
        let Some(player) = self.player.clone() else { return };
        let expected = self.generation;
        let this = cx.weak_entity();
        player.update(cx, |player, cx| {
            player.seek(
                0.0,
                0.0,
                0.0,
                Some(Box::new(move |finished: bool, cx: &mut App| {
                    if !finished {
                        return;
                    }
                    let _ = this.update(cx, |this, cx| {
                        if this.generation != expected
                            || this.slot != GiftCosmeticPlaybackSlotState::Granted
                            || !this.wants_to_play(cx)
                        {
                            return;
                        }
                        if let Some(player) = this.player.clone() {
                            player.update(cx, |player, cx| player.play(cx));
                        }
                    });
                })),
                cx,
            );
        });
    }

    fn pause(&mut self, cx: &mut Context<Self>) {
        if let Some(player) = &self.player {
            player.update(cx, |player, cx| player.pause(cx));
        }
    }

    fn stop_player(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.load_task = None;
        self.rest_task = None;
        self.once_deadline_task = None;
        self.awaits_replay = false;
        self.awaits_first_frame = false;
        self.did_start_once = false;
        if let Some(player) = self.player.take() {
            player.update(cx, |player, cx| player.pause(cx));
        }
        self.end_observer = None;
        self.failure_observation = None;
        self.lease = None;
        self.ready_surfaces.clear();
        for surface in self.live_surfaces() {
            surface.update(cx, |surface, cx| {
                surface.set_player(None, cx);
                surface.end_poster_dissolve(cx);
            });
        }
    }

    fn teardown(&mut self, cx: &mut Context<Self>) {
        self.poster_task = None;
        self.stop_player(cx);
        self.release_slot(cx);
        self.is_visible = false;
        self.source = None;
        self.poster = None;
    }

    /// poster hides only once every surface draws video, both carry alpha and a poster under a
    /// playing picture would ghost through
    /// a once movie picture is shown with its player so its first frame is already on screen
    fn apply_visibility_of_layers(&mut self, cx: &mut Context<Self>) {
        let live = self.live_surfaces();
        let installed = self.player.is_some() && !live.is_empty();
        let video_ready = self.is_video_ready();
        let Some(source) = self.source.as_ref().filter(|source| source.motion == GiftCosmeticMotion::Once) else {
            for surface in live {
                surface.update(cx, |surface, cx| surface.set_layers(video_ready, !video_ready, cx));
            }
            return;
        };
        let shows_poster = source.rests_on_poster && (source.fades_poster || !video_ready || self.awaits_first_frame);
        for surface in live {
            surface.update(cx, |surface, cx| surface.set_layers(!shows_poster, !installed, cx));
        }
    }
}

/// what a surface last told its playback
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GiftCosmeticSurfaceViewVisibility {
    in_viewport: bool,
    on_screen: bool,
}

/// effect poster dissolve in progress
#[derive(Clone, Copy, Debug, PartialEq)]
struct GiftCosmeticPosterDissolve {
    started: Duration,
    duration: Duration,
}

impl GiftCosmeticPosterDissolve {
    const VALUES: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const FLOOR: f32 = 0.002;
    const STEP: f32 = 0.002;
    const FRAME_INTERVAL: Duration = Duration::from_millis(16);
    const KEY_TIMES: [f32; 4] = [0.0, 0.4, 0.6, 1.0];

    /// poster opacity at this progress of the play, linear between keyframes
    fn opacity(progress: f32) -> f32 {
        let progress = progress.clamp(0.0, 1.0);
        for index in 1..Self::KEY_TIMES.len() {
            let (start, end) = (Self::KEY_TIMES[index - 1], Self::KEY_TIMES[index]);
            if progress <= end {
                let span = end - start;
                let local = if span > 0.0 { (progress - start) / span } else { 1.0 };
                let (from, to) = (Self::VALUES[index - 1], Self::VALUES[index]);
                return from + (to - from) * local;
            }
        }
        1.0
    }
}

/// how fast the list under a playing cosmetic moves, from frames the surface gets
/// gpui has no handle on the scroll view so the own frame is the reading
/// above the fast threshold is flying, settled after a quiet moment
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GiftCosmeticScrollSpeed {
    track: Option<GiftCosmeticScrollSpeedTrack>,
    is_fast: bool,
    slow_since: Option<Duration>,
    is_moving: bool,
}

/// the last reading, where the surface was and when
#[derive(Clone, Copy, Debug, PartialEq)]
struct GiftCosmeticScrollSpeedTrack {
    offset: Point<Pixels>,
    time: Duration,
}

impl GiftCosmeticScrollSpeed {
    pub const SAME_FRAME: Duration = Duration::from_micros(4_167);
    pub const STALE_SAMPLE: Duration = Duration::from_millis(250);
    pub const SETTLE_AFTER: Duration = Duration::from_millis(120);
    pub const SLOW_HOLD: Duration = Duration::from_millis(250);

    pub fn is_fast(&self) -> bool {
        self.is_fast
    }

    /// moving at any speed or just appeared, nothing starts or stops until the list rests
    pub fn is_moving(&self) -> bool {
        self.is_moving
    }

    /// new surface position at now, returns whether it is flying past
    pub fn sample(&mut self, origin: Point<Pixels>, now: Duration) -> bool {
        let Some(track) = self.track else {
            self.track = Some(GiftCosmeticScrollSpeedTrack { offset: origin, time: now });
            self.is_fast = false;
            self.slow_since = None;
            self.is_moving = true;
            return false;
        };
        let elapsed = now.saturating_sub(track.time);
        if elapsed < Self::SAME_FRAME {
            return self.is_fast;
        }
        let distance = f32::from(origin.x - track.offset.x)
            .abs()
            .max(f32::from(origin.y - track.offset.y).abs());
        let fresh = elapsed < Self::STALE_SAMPLE;
        if distance > 0.5 {
            self.is_moving = true;
        }
        let flying = fresh
            && distance / elapsed.as_secs_f32() > RiseManualListMountEnvelopePolicy::FAST_SCROLL_VELOCITY_THRESHOLD;
        if flying {
            self.is_fast = true;
            self.slow_since = None;
        } else if self.is_fast && fresh {
            let since = *self.slow_since.get_or_insert(track.time);
            if now.saturating_sub(since) >= Self::SLOW_HOLD {
                self.is_fast = false;
                self.slow_since = None;
            }
        } else {
            self.is_fast = false;
            self.slow_since = None;
        }
        self.track = Some(GiftCosmeticScrollSpeedTrack { offset: origin, time: now });
        self.is_fast
    }

    pub fn settle_if_quiet(&mut self, now: Duration) -> Option<bool> {
        if !self.is_fast && !self.is_moving {
            return Some(false);
        }
        let quiet = self
            .track
            .is_none_or(|track| now.saturating_sub(track.time) >= Self::SETTLE_AFTER);
        if !quiet {
            return None;
        }
        self.is_fast = false;
        self.slow_since = None;
        self.is_moving = false;
        Some(true)
    }
}

/// one surface of a cosmetic, the poster cropped to its part and a clip holding the player
/// picture as big as the whole media shifted so the same part shows
pub struct GiftCosmeticSurfaceView {
    pub playback: Option<WeakEntity<GiftCosmeticPlayback>>,
    reader_id: String,
    poster: Option<CleverImage>,
    poster_aspect: Option<f32>,
    poster_hidden: bool,
    video_hidden: bool,
    player: Option<Entity<AVPlayer>>,
    ready_observation: Option<Subscription>,
    last_ready: Option<bool>,
    strip: Option<WeakEntity<AnimatedTabsStripMotionDriver>>,
    strip_observation: Option<Subscription>,
    is_foreground: bool,
    last_reported_visibility: Option<GiftCosmeticSurfaceViewVisibility>,
    tracks_visibility: bool,
    part: GiftCosmeticPart,
    placement: GiftCosmeticPlacement,
    fallback_aspect: f32,
    placeholder: bool,
    size: Option<Size<Pixels>>,
    size_seed: Option<Size<Pixels>>,
    scroll_speed: GiftCosmeticScrollSpeed,
    settle_task: Option<Task<()>>,
    dissolve: Option<GiftCosmeticPosterDissolve>,
    in_presented_layer: bool,
    window_id: Option<WindowId>,
    is_covered: bool,
    poster_swap_key: u32,
    swap_window: Option<AnyWindowHandle>,
    painted_poster_opacity: Option<f32>,
    dissolve_task: Option<Task<()>>,
    has_rendered: bool,
    painted_video: bool,
    changed_under_cover: bool,
    cover_render_asked: bool,
    _foreground_observation: Subscription,
    _release: Subscription,
}

impl GiftCosmeticSurfaceView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let key = cx.entity_id();
        let release = cx.on_release(move |surface: &mut Self, cx| {
            GiftCosmeticSurface::dismantle_ui_view(key, surface.playback.take(), cx);
        });
        let gate = RiseMediaForegroundGate::shared(cx);
        let is_foreground = gate.read(cx).is_foreground();
        let foreground_observation = cx.observe(&gate, |surface: &mut Self, gate, cx| {
            surface.is_foreground = gate.read(cx).is_foreground();
            surface.report_visibility_if_changed(cx);
        });
        Self {
            playback: None,
            reader_id: format!("gift-cosmetic-surface-{}", key.as_u64()),
            poster: None,
            poster_aspect: None,
            poster_hidden: false,
            video_hidden: true,
            player: None,
            ready_observation: None,
            last_ready: None,
            strip: None,
            strip_observation: None,
            is_foreground,
            last_reported_visibility: None,
            tracks_visibility: false,
            part: GiftCosmeticPart::Whole,
            placement: GiftCosmeticPlacement::Fill,
            fallback_aspect: 1.0,
            placeholder: false,
            size: None,
            size_seed: None,
            scroll_speed: GiftCosmeticScrollSpeed::default(),
            settle_task: None,
            dissolve: None,
            in_presented_layer: false,
            window_id: None,
            is_covered: false,
            poster_swap_key: (key.as_u64() as u32).wrapping_add(1),
            swap_window: None,
            painted_poster_opacity: None,
            dissolve_task: None,
            has_rendered: false,
            painted_video: false,
            changed_under_cover: false,
            cover_render_asked: false,
            _foreground_observation: foreground_observation,
            _release: release,
        }
    }

    pub fn configure(
        &mut self,
        part: GiftCosmeticPart,
        placement: GiftCosmeticPlacement,
        fallback_aspect: f32,
        cx: &mut Context<Self>,
    ) {
        if part == self.part && placement == self.placement && fallback_aspect == self.fallback_aspect {
            return;
        }
        self.part = part;
        self.placement = placement;
        self.fallback_aspect = fallback_aspect;
        self.poster_aspect = self.poster.as_ref().and_then(|image| Self::aspect_of(image, part));
        self.notify_unless_covered(cx);
    }

    fn notify_if_rendered(&self, cx: &mut Context<Self>) {
        if self.has_rendered {
            cx.notify();
        }
    }

    fn aspect_of(image: &CleverImage, part: GiftCosmeticPart) -> Option<f32> {
        let size = image.size(0);
        (size.height.0 > 0).then(|| size.width.0 as f32 * part.source_rect().width / size.height.0 as f32)
    }

    pub fn set_poster(&mut self, image: Option<CleverImage>, cx: &mut Context<Self>) {
        let same = match (&self.poster, &image) {
            (Some(current), Some(next)) => Arc::ptr_eq(current, next),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.poster_aspect = image.as_ref().and_then(|image| Self::aspect_of(image, self.part));
        self.poster = image;
        self.notify_unless_covered(cx);
    }

    pub fn seed_size(&mut self, seed: Option<Size<Pixels>>) {
        if self.size.is_none() {
            self.size = seed;
            self.size_seed = seed;
        }
    }

    fn same_within_snap(a: Size<Pixels>, b: Size<Pixels>) -> bool {
        (f32::from(a.width) - f32::from(b.width)).abs() <= 1.0 && (f32::from(a.height) - f32::from(b.height)).abs() <= 1.0
    }

    pub fn set_placeholder(&mut self, placeholder: bool, cx: &mut Context<Self>) {
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            self.notify_unless_covered(cx);
        }
    }

    pub fn set_player(&mut self, player: Option<Entity<AVPlayer>>, cx: &mut Context<Self>) {
        if self.player.as_ref().map(Entity::entity_id) == player.as_ref().map(Entity::entity_id) {
            return;
        }
        let drawn = self.drawn_layers();
        self.ready_observation = None;
        self.last_ready = None;
        self.player = player.clone();
        if self.drawn_layers() != drawn {
            self.notify_unless_covered(cx);
        }
        let Some(player) = player else { return };
        let expected = player.entity_id();
        self.ready_observation = Some(cx.observe(&player, move |surface: &mut Self, player, cx| {
            surface.player_did_change(expected, &player, cx);
        }));
        self.player_did_change(expected, &player, cx);
    }

    /// is_ready_for_display of the player this surface shows, reported on every change
    fn player_did_change(&mut self, expected: EntityId, player: &Entity<AVPlayer>, cx: &mut Context<Self>) {
        if self.player.as_ref().map(Entity::entity_id) != Some(expected) {
            return;
        }
        let ready = player.read(cx).is_ready_for_display();
        if self.last_ready == Some(ready) {
            return;
        }
        self.last_ready = Some(ready);
        let Some(playback) = self.playback.clone() else { return };
        let key = cx.entity_id();
        cx.defer(move |cx| {
            let _ = playback.update(cx, |playback, cx| playback.surface_ready_changed(key, expected, ready, cx));
        });
    }

    pub fn set_layers(&mut self, poster_hidden: bool, video_hidden: bool, cx: &mut Context<Self>) {
        if self.poster_hidden == poster_hidden && self.video_hidden == video_hidden {
            return;
        }
        let drawn = self.drawn_layers();
        self.poster_hidden = poster_hidden;
        self.video_hidden = video_hidden;
        if self.drawn_layers() != drawn {
            self.notify_unless_covered(cx);
        }
    }

    fn drawn_layers(&self) -> (Option<EntityId>, bool) {
        let video = self.player.as_ref().filter(|_| !self.video_hidden).map(Entity::entity_id);
        (video, self.poster.is_some() && !self.poster_hidden)
    }

    fn notify_unless_covered(&mut self, cx: &mut Context<Self>) {
        if !self.defers_under_cover(cx) {
            self.notify_if_rendered(cx);
        }
    }

    fn defers_under_cover(&mut self, cx: &App) -> bool {
        if !self.tracks_visibility || !self.has_rendered || self.poster.is_none() || !self.covered_now(cx) {
            return false;
        }
        self.changed_under_cover = true;
        if self.cover_render_asked || !self.cover_changes_picture() {
            return true;
        }
        self.cover_render_asked = true;
        false
    }

    /// whether the covered picture, the poster alone at full opacity differs from the frame on screen
    fn cover_changes_picture(&self) -> bool {
        self.poster.is_some()
            && (self.painted_video
                || self
                    .painted_poster_opacity
                    .is_none_or(|opacity| opacity < 1.0 - GiftCosmeticPosterDissolve::STEP))
    }

    fn uncover_changes_picture(&self) -> bool {
        self.poster.is_some()
            && ((self.player.is_some() && !self.video_hidden) || self.poster_hidden || self.dissolve.is_some())
    }

    /// the pulse saw a modal layer come or go over this surface
    /// value is stored at once since a surface out of the drawn tree never renders to take it,
    /// render only when the picture changes
    fn cover_changed(&mut self, covered: bool, cx: &mut Context<Self>) {
        self.is_covered = covered;
        let renders = if covered {
            let renders = self.cover_changes_picture() && !self.cover_render_asked;
            self.cover_render_asked |= renders;
            renders
        } else {
            let renders = self.changed_under_cover || self.uncover_changes_picture();
            self.changed_under_cover = false;
            self.cover_render_asked = false;
            renders
        };
        if renders {
            cx.notify();
        }
    }

    pub fn dissolve_poster(&mut self, seconds: f64, cx: &mut Context<Self>) {
        if !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        self.dissolve = Some(GiftCosmeticPosterDissolve {
            started: RiseClock::monotonic(),
            duration: Duration::from_secs_f64(seconds),
        });
        self.notify_unless_covered(cx);
        self.dissolve_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(GiftCosmeticPosterDissolve::FRAME_INTERVAL).await;
                let goes_on = this.update(cx, |surface, cx| surface.dissolve_tick(cx)).unwrap_or(false);
                if !goes_on {
                    break;
                }
            }
        }));
    }

    fn dissolve_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(dissolve) = self.dissolve else { return false };
        let elapsed = RiseClock::monotonic().saturating_sub(dissolve.started);
        let finished = elapsed >= dissolve.duration;
        if finished {
            self.dissolve = None;
        }
        if self.is_covered || self.painted_poster_opacity.is_none() {
            return !finished;
        }
        let target = if finished {
            1.0
        } else {
            GiftCosmeticPosterDissolve::opacity(elapsed.as_secs_f32() / dissolve.duration.as_secs_f32())
                .max(GiftCosmeticPosterDissolve::FLOOR)
        };
        if !self.fade_poster_to(target, cx) && (finished || self.is_in_viewport()) {
            cx.notify();
        }
        !finished
    }

    fn fade_poster_to(&mut self, target: f32, cx: &mut Context<Self>) -> bool {
        let (Some(window), Some(painted)) = (self.swap_window, self.painted_poster_opacity) else {
            return false;
        };
        if (painted - target).abs() < GiftCosmeticPosterDissolve::STEP {
            return true;
        }
        if painted <= 0.0 {
            return false;
        }
        let key = self.poster_swap_key;
        let faded = window
            .update(cx, |_, window, _| window.fade_swapped_image(key, target / painted))
            .unwrap_or(false);
        if faded {
            self.painted_poster_opacity = Some(target);
        }
        faded
    }

    pub fn end_poster_dissolve(&mut self, cx: &mut Context<Self>) {
        self.dissolve_task = None;
        if self.dissolve.take().is_some() {
            self.notify_unless_covered(cx);
        }
    }

    /// set by the playback, a surface that may play follows the screen and a poster doesnt
    pub fn set_tracks_visibility(&mut self, tracks: bool, cx: &mut Context<Self>) {
        if tracks == self.tracks_visibility {
            return;
        }
        self.tracks_visibility = tracks;
        self.refresh_subscriptions(cx);
        self.report_visibility_if_changed(cx);
    }

    /// the pager the surface is drawn in, read from the environment on every render
    pub fn set_enclosing_strip(
        &mut self,
        strip: Option<WeakEntity<AnimatedTabsStripMotionDriver>>,
        cx: &mut Context<Self>,
    ) {
        if self.strip.as_ref().map(|strip| strip.entity_id()) == strip.as_ref().map(|strip| strip.entity_id()) {
            return;
        }
        self.strip_observation = strip.as_ref().and_then(WeakEntity::upgrade).map(|driver| {
            cx.subscribe(&driver, |surface: &mut Self, _, _: &AnimatedTabsStripDidRest, cx| {
                surface.strip_did_rest(cx);
            })
        });
        self.strip = strip;
    }

    fn is_attached(&self) -> bool {
        MediaCellScreenFrameRegistry::attached_view(&self.reader_id).is_some()
    }

    /// in the viewport of a foreground window however fast its list moves
    /// a play once cosmetic starts over only after leaving it, not when a fling merely holds it still
    pub fn is_in_viewport(&self) -> bool {
        self.tracks_visibility
            && self.is_foreground
            && MediaCellScreenFrameRegistry::attached_view(&self.reader_id).is_some_and(|view| {
                view.is_visible_in_active_clipped_viewport_with(
                    AnimatedMediaViewportPlaybackPolicy::MINIMUM_VISIBLE_HEIGHT_FRACTION,
                    true,
                )
            })
    }

    /// in the viewport and not in a list flying past faster than an eye reads
    /// there the poster stays and players start once the list slows
    pub fn is_on_screen(&self) -> bool {
        self.is_in_viewport() && !self.scroll_speed.is_fast()
    }

    /// inside a pager strip that a pointer or its settle moves
    /// nothing starts or stops there until it rests so no player is built or let go mid swipe
    pub fn is_in_moving_strip(&self, cx: &App) -> bool {
        self.is_attached()
            && self
                .strip
                .as_ref()
                .is_some_and(|strip| AnimatedTabsStripActivity::is_moving(strip, cx))
    }

    pub fn is_in_moving_list(&self) -> bool {
        self.tracks_visibility && self.scroll_speed.is_moving()
    }

    pub fn stop_tracking(&mut self, cx: &mut Context<Self>) {
        let key = cx.entity_id();
        GiftCosmeticVisibilityPulse::shared(cx).update(cx, |pulse, cx| pulse.remove(key, cx));
        self.settle_task = None;
        self.scroll_speed = GiftCosmeticScrollSpeed::default();
        self.last_reported_visibility = None;
    }

    pub fn recheck_visibility(&mut self, cx: &mut Context<Self>) {
        let covered = self.covered_now(cx);
        if covered != self.is_covered {
            self.cover_changed(covered, cx);
        } else if !covered && self.changed_under_cover {
            self.changed_under_cover = false;
            cx.notify();
        }
        self.report_visibility_if_changed(cx);
    }

    fn covered_now(&self, cx: &App) -> bool {
        !self.in_presented_layer
            && self
                .window_id
                .is_some_and(|window_id| TopPresenter::has_presented_in(window_id, cx))
    }

    fn refresh_subscriptions(&mut self, cx: &mut Context<Self>) {
        let key = cx.entity_id();
        let weak = cx.weak_entity();
        let tracks = self.tracks_visibility;
        GiftCosmeticVisibilityPulse::shared(cx).update(cx, |pulse, cx| {
            if tracks {
                pulse.add(key, weak, cx);
            } else {
                pulse.remove(key, cx);
            }
        });
        if !tracks {
            self.settle_task = None;
            self.scroll_speed = GiftCosmeticScrollSpeed::default();
        }
    }

    fn layout_subviews(&mut self, frame: Bounds<Pixels>, cx: &mut Context<Self>) {
        let next = match self.size_seed {
            Some(seed) if Self::same_within_snap(seed, frame.size) => seed,
            _ => frame.size,
        };
        if self.size != Some(next) {
            self.size = Some(next);
            cx.notify();
        }
        if self.tracks_visibility
            && (self.scroll_speed.sample(frame.origin, RiseClock::monotonic()) || self.scroll_speed.is_moving())
        {
            self.watch_for_settle(cx);
        }
        self.report_visibility_if_changed(cx);
    }

    fn watch_for_settle(&mut self, cx: &mut Context<Self>) {
        if self.settle_task.is_some() {
            return;
        }
        self.settle_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(GiftCosmeticScrollSpeed::SETTLE_AFTER).await;
                let settled = this.update(cx, |surface, cx| {
                    match surface.scroll_speed.settle_if_quiet(RiseClock::monotonic()) {
                        None => false,
                        Some(just_settled) => {
                            surface.settle_task = None;
                            if just_settled {
                                surface.report_visibility_if_changed(cx);
                                surface.ask_playback(cx);
                            }
                            true
                        }
                    }
                });
                if settled.unwrap_or(true) {
                    return;
                }
            }
        }));
    }

    fn strip_did_rest(&mut self, cx: &mut Context<Self>) {
        self.report_visibility_if_changed(cx);
        self.ask_playback(cx);
    }

    /// reports a change of either answer, leaving the viewport matters to a play once cosmetic
    /// even while a fast list holds it still
    fn report_visibility_if_changed(&mut self, cx: &mut Context<Self>) {
        if self.is_in_moving_strip(cx) || self.is_in_moving_list() {
            return;
        }
        let in_viewport = self.is_attached() && self.is_in_viewport();
        let visibility = GiftCosmeticSurfaceViewVisibility {
            in_viewport,
            on_screen: in_viewport && !self.scroll_speed.is_fast(),
        };
        if self.last_reported_visibility == Some(visibility) {
            return;
        }
        self.last_reported_visibility = Some(visibility);
        self.ask_playback(cx);
    }

    fn ask_playback(&self, cx: &mut Context<Self>) {
        let Some(playback) = self.playback.clone() else { return };
        cx.defer(move |cx| {
            let _ = playback.update(cx, |playback, cx| playback.surface_visibility_changed(cx));
        });
    }

    fn poster_opacity(&mut self) -> f32 {
        let Some(dissolve) = self.dissolve else { return 1.0 };
        let elapsed = RiseClock::monotonic().saturating_sub(dissolve.started);
        if elapsed >= dissolve.duration {
            self.dissolve = None;
            self.dissolve_task = None;
            return 1.0;
        }
        GiftCosmeticPosterDissolve::opacity(elapsed.as_secs_f32() / dissolve.duration.as_secs_f32())
            .max(GiftCosmeticPosterDissolve::FLOOR)
    }

    const PLACEHOLDER_SCALE: f32 = 0.62;

    fn clip(layout: &GiftCosmeticSurfaceLayout) -> gpui::Div {
        div()
            .absolute()
            .left(px(layout.drawn.x))
            .top(px(layout.drawn.y))
            .w(px(layout.drawn.width))
            .h(px(layout.drawn.height))
            .overflow_hidden()
    }

    fn media(layout: &GiftCosmeticSurfaceLayout) -> gpui::Div {
        div()
            .absolute()
            .left(px(layout.media.x))
            .top(px(layout.media.y))
            .w(px(layout.media.width))
            .h(px(layout.media.height))
    }
}

impl Render for GiftCosmeticSurfaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.weak_entity();
        let reader = WindowScreenFrameReader::new(self.reader_id.clone()).on_change(move |frame, _, cx| {
            let _ = weak.update(cx, |surface, cx| surface.layout_subviews(frame, cx));
        });
        let aspect = self.poster_aspect.unwrap_or(self.fallback_aspect);
        let layout = self.size.and_then(|size| {
            GiftCosmeticSurfaceLayout::resolve(
                self.part,
                self.placement,
                aspect,
                f32::from(size.width),
                f32::from(size.height),
            )
        });
        self.in_presented_layer = env::<InsidePresentedLayerKey>(cx);
        self.window_id = Some(window.window_handle().window_id());
        self.is_covered = self.covered_now(cx);
        self.swap_window = Some(window.window_handle());
        self.painted_poster_opacity = None;
        self.painted_video = false;
        self.has_rendered = true;
        self.cover_render_asked = false;
        if !self.is_covered {
            self.changed_under_cover = false;
        }
        let mut root = div().relative().size_full();
        if let Some(layout) = layout {
            let video_hidden = self.video_hidden || (self.is_covered && self.poster.is_some());
            let poster_hidden = self.poster_hidden && !(self.is_covered && self.poster.is_some());
            let shows_video = self.player.is_some() && !video_hidden;
            self.painted_video = shows_video;
            let shows_poster = self.poster.is_some() && !poster_hidden;
            if self.placeholder && !shows_video && !shows_poster {
                let side = layout.drawn.width.min(layout.drawn.height);
                root = root.child(
                    Self::clip(&layout).flex().items_center().justify_center().child(
                        div()
                            .size(px(side * Self::PLACEHOLDER_SCALE))
                            .rounded(px(side * Self::PLACEHOLDER_SCALE * 0.5))
                            .bg(gpui::hsla(0.0, 0.0, 0.5, 0.14)),
                    ),
                );
            }
            if let Some(poster) = self.poster.clone().filter(|_| !poster_hidden) {
                let opacity = if self.is_covered { 1.0 } else { self.poster_opacity() };
                self.painted_poster_opacity = Some(opacity);
                root = root.child(Self::clip(&layout).opacity(opacity).child(Self::media(&layout).child(
                    GiftPosterSwapKeyed {
                        key: self.poster_swap_key,
                        child: img(poster).size_full().object_fit(ObjectFit::Fill).into_any_element(),
                    },
                )));
            }
            if let Some(player) = self.player.clone().filter(|_| !video_hidden) {
                root = root.child(Self::clip(&layout).child(Self::media(&layout).child(
                    AVPlayerLayer::for_player(Some(player)).video_gravity(AVLayerVideoGravity::Resize),
                )));
            }
        }
        root.child(reader)
    }
}

struct GiftPosterSwapKeyed {
    key: u32,
    child: AnyElement,
}

impl IntoElement for GiftPosterSwapKeyed {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for GiftPosterSwapKeyed {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_image_swap_key(self.key, |window| self.child.paint(window, cx));
    }
}

/// asks every surface that may play a few times a second whether its on screen
/// frame reports alone miss a pager strip offset, a sliding sheet or a pushed page covering a list
/// one timer for all surfaces, runs only while one is registered
pub struct GiftCosmeticVisibilityPulse {
    entries: HashMap<EntityId, WeakEntity<GiftCosmeticSurfaceView>>,
    timer: Option<Task<()>>,
}

impl GiftCosmeticVisibilityPulse {
    const INTERVAL: Duration = Duration::from_millis(300);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { entries: HashMap::new(), timer: None })
    }

    pub fn add(&mut self, key: EntityId, surface: WeakEntity<GiftCosmeticSurfaceView>, cx: &mut Context<Self>) {
        self.entries.insert(key, surface);
        if self.timer.is_some() {
            return;
        }
        self.timer = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Self::INTERVAL).await;
                let Ok(surfaces) = this.update(cx, |pulse, _| pulse.tick()) else { return };
                if surfaces.is_empty() {
                    return;
                }
                cx.update(|cx| {
                    for surface in surfaces {
                        surface.update(cx, |surface, cx| surface.recheck_visibility(cx));
                    }
                });
            }
        }));
    }

    pub fn remove(&mut self, key: EntityId, _cx: &mut Context<Self>) {
        self.entries.remove(&key);
        if self.entries.is_empty() {
            self.timer = None;
        }
    }

    /// the live surfaces to ask, the timer stops when none is left
    fn tick(&mut self) -> Vec<Entity<GiftCosmeticSurfaceView>> {
        self.entries.retain(|_, surface| surface.upgrade().is_some());
        if self.entries.is_empty() {
            self.timer = None;
            return Vec::new();
        }
        self.entries.values().filter_map(WeakEntity::upgrade).collect()
    }
}

/// a player build, answers whether it built anything and a stale one gives its turn away
pub type GiftFramePacerBuild = Box<dyn FnOnce(&mut App) -> bool>;

struct GiftFramePacerCatchUp {
    distance: i64,
    run: Box<dyn FnOnce(&mut App)>,
}

/// main thread work of gifts that comes in bursts, spread over display frames in two lanes
/// players get a few ms per frame and at least one, catching up shelves get a frame no player needs
/// nothing here holds a player, queued work only builds one
pub struct GiftFramePacer {
    players: VecDeque<GiftFramePacerBuild>,
    catch_ups: Vec<GiftFramePacerCatchUp>,
    link: Option<Task<()>>,
}

impl GiftFramePacer {
    const FRAME_BUDGET: Duration = Duration::from_millis(4);
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_667);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { players: VecDeque::new(), catch_ups: Vec::new(), link: None })
    }

    pub fn enqueue_player(&mut self, build: GiftFramePacerBuild, cx: &mut Context<Self>) {
        self.players.push_back(build);
        self.start_link(cx);
    }

    pub fn enqueue_catch_up(&mut self, distance: i64, step: impl FnOnce(&mut App) + 'static, cx: &mut Context<Self>) {
        self.catch_ups.push(GiftFramePacerCatchUp { distance, run: Box::new(step) });
        self.start_link(cx);
    }

    fn start_link(&mut self, cx: &mut Context<Self>) {
        if self.link.is_some() {
            return;
        }
        self.link = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Self::FRAME_INTERVAL).await;
                let keeps_running = cx.update(|cx| Self::drain(&this, cx));
                if !keeps_running {
                    return;
                }
            }
        }));
    }

    /// one frame of work, false once the queues are empty and the clock stopped
    /// each build runs outside the pacer update so it may queue more
    fn drain(this: &WeakEntity<Self>, cx: &mut App) -> bool {
        let start = RiseClock::monotonic();
        let mut built_player = false;
        loop {
            let over_budget =
                built_player && RiseClock::monotonic().saturating_sub(start) >= Self::FRAME_BUDGET;
            if over_budget {
                break;
            }
            let Ok(Some(build)) = this.update(cx, |pacer, _| pacer.players.pop_front()) else {
                break;
            };
            if build(cx) {
                built_player = true;
            }
        }
        if !built_player {
            let step = this
                .update(cx, |pacer, _| {
                    if !pacer.players.is_empty() {
                        return None;
                    }
                    let nearest = pacer.catch_ups.iter().map(|catch_up| catch_up.distance).min()?;
                    let index = pacer.catch_ups.iter().position(|catch_up| catch_up.distance == nearest)?;
                    Some(pacer.catch_ups.remove(index))
                })
                .ok()
                .flatten();
            if let Some(step) = step {
                (step.run)(cx);
            }
        }
        this.update(cx, |pacer, _| {
            let idle = pacer.players.is_empty() && pacer.catch_ups.is_empty();
            if idle {
                pacer.link = None;
            }
            !idle
        })
        .unwrap_or(false)
    }
}

/// the element face of one surface, several surfaces may share one playback
#[derive(IntoElement)]
pub struct GiftCosmeticSurface {
    id: ElementId,
    playback: Entity<GiftCosmeticPlayback>,
    source: GiftCosmeticPlaybackSource,
    part: GiftCosmeticPart,
    placement: GiftCosmeticPlacement,
    fallback_aspect: f32,
    placeholder: bool,
    size_hint: Option<Size<Pixels>>,
}

impl GiftCosmeticSurface {
    /// id keeps the surface own state apart from its siblings
    pub fn new(
        id: impl Into<ElementId>,
        playback: Entity<GiftCosmeticPlayback>,
        source: GiftCosmeticPlaybackSource,
        fallback_aspect: f32,
    ) -> Self {
        Self {
            id: id.into(),
            playback,
            source,
            part: GiftCosmeticPart::Whole,
            placement: GiftCosmeticPlacement::Fill,
            fallback_aspect,
            placeholder: false,
            size_hint: None,
        }
    }

    /// surface size when the parent knows it before layout
    pub fn size_hint(mut self, size_hint: Option<Size<Pixels>>) -> Self {
        self.size_hint = size_hint;
        self
    }

    pub fn part(mut self, part: GiftCosmeticPart) -> Self {
        self.part = part;
        self
    }

    pub fn placement(mut self, placement: GiftCosmeticPlacement) -> Self {
        self.placement = placement;
        self
    }

    /// soft disc until the poster or first frame draws, used in the gift sheet
    pub fn placeholder(mut self, placeholder: bool) -> Self {
        self.placeholder = placeholder;
        self
    }
}

impl GiftCosmeticSurface {
    fn make_ui_view(self, view: &Entity<GiftCosmeticSurfaceView>, cx: &mut App) {
        let strip = AnimatedTabsStripActivity::enclosing(cx);
        view.update(cx, |view, cx| {
            view.seed_size(self.size_hint);
            view.configure(self.part, self.placement, self.fallback_aspect, cx);
            view.set_placeholder(self.placeholder, cx);
            view.set_enclosing_strip(strip, cx);
        });
        self.playback.update(cx, |playback, cx| {
            playback.update(self.source, cx);
            playback.attach(view, cx);
        });
    }

    fn update_ui_view(self, view: &Entity<GiftCosmeticSurfaceView>, current: Entity<GiftCosmeticPlayback>, cx: &mut App) {
        let strip = AnimatedTabsStripActivity::enclosing(cx);
        view.update(cx, |view, cx| {
            view.configure(self.part, self.placement, self.fallback_aspect, cx);
            view.set_placeholder(self.placeholder, cx);
            view.set_enclosing_strip(strip, cx);
        });
        if current.entity_id() != self.playback.entity_id() {
            let key = view.entity_id();
            current.update(cx, |current, cx| current.detach(key, Some(view), cx));
            self.playback.update(cx, |playback, cx| playback.attach(view, cx));
        }
        self.playback.update(cx, |playback, cx| playback.update(self.source, cx));
    }

    fn dismantle_ui_view(key: EntityId, playback: Option<WeakEntity<GiftCosmeticPlayback>>, cx: &mut App) {
        GiftCosmeticVisibilityPulse::shared(cx).update(cx, |pulse, cx| pulse.remove(key, cx));
        if let Some(playback) = playback.and_then(|playback| playback.upgrade()) {
            playback.update(cx, |playback, cx| playback.detach(key, None, cx));
        }
    }
}

impl RenderOnce for GiftCosmeticSurface {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.with_global_id(self.id.clone(), |global_id, window| {
            window.with_element_state(global_id, |state: Option<Entity<GiftCosmeticSurfaceView>>, _| {
                let view = state.unwrap_or_else(|| cx.new(GiftCosmeticSurfaceView::new));
                (view.clone(), view)
            })
        });
        let current = view.read(cx).playback.as_ref().and_then(WeakEntity::upgrade);
        match current {
            Some(current) => self.update_ui_view(&view, current, cx),
            None => self.make_ui_view(&view, cx),
        }
        div().size_full().child(view.cached(StyleRefinement::default().size_full()).replay_when_moved())
    }
}
