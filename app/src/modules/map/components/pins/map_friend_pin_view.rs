use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    AnyElement, App, BoxShadow, Context, Div, FontStyle, FontWeight, Hsla, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled, StyledImage, Task, TextRun, Window,
    black, canvas, div, img, point, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::avatar_palette::AvatarPalette;
use rise_theme::theme::app_theme::readable_ink;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{Appearance, RiseThemeAppearance, ThemeStore};
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::font::{Font, SwiftUiFont};
use rise_ui::swiftui::motion::{CubicBezier, ease_in_out, spring};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::map::components::pins::map_pin_decor::{
    MapPinAngle, MapPinBatteryView, MapPinConeFrame, MapPinConeLayer, MapPinMotion, MapPinReactionBadge, MapPinTileView,
    MapPinTrailLayer, MapPinZoom, autoreversing_progress,
};
use crate::modules::map::components::pins::map_self_pin_layer::{MapSelfPinState, MapSelfProfile};
use crate::modules::map::components::reactions::map_reaction_stage::MapReactionStage;
use crate::modules::map::recorder::map_self_location_feed::MapSelfHeading;
use crate::modules::map::shared::map_friend_status::{MapPinModel, MapPinTile, MapPinTrail};
use crate::modules::map::stores::map_actions::map_types::MapLiveFriend;

/// called by a pin layer when the reader clicks a pin
pub type MapPinSelectHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// pin colors resolved once per theme change and handed down
/// so pins never read the theme store while the map moves
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapPinPalette {
    pub primary: Hsla,
    pub ring: Hsla,
    pub stale: Hsla,
    pub plate: Hsla,
    pub plate_text: Hsla,
    pub letter: Hsla,
    pub low_battery: Hsla,
}

/// round avatar backed by the image cache
/// memory hit installs in the same pass
/// anything else arrives later and is dropped if the view was reassigned
#[derive(Default)]
pub struct MapAvatarView {
    image: Rc<RefCell<Option<CleverImage>>>,
    letter: SharedString,
    letter_color: Option<Hsla>,
    background: Option<Hsla>,
    image_key: Option<String>,
    load_task: Option<Task<()>>,
    load_generation: Rc<Cell<u64>>,
}

impl MapAvatarView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn configure<V: 'static>(
        &mut self,
        source: &str,
        name: &str,
        user_id: &str,
        diameter: f32,
        palette: &MapPinPalette,
        cx: &mut Context<V>,
    ) {
        self.letter = AvatarPalette::display_letter(Some(name)).into();
        let background = AvatarPalette::color_for_user(Some(user_id), Some(name));
        self.letter_color = Some(readable_ink(background, palette.letter));
        self.background = Some(background);

        let Some(url) =
            AvatarPalette::resolved_logo_url(Some(source)).filter(|url| url.to_lowercase().starts_with("http"))
        else {
            self.cancel_load();
            self.image_key = None;
            *self.image.borrow_mut() = None;
            return;
        };

        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let pixels = (diameter * 3.0).ceil() as i64;
        let key = format!(
            "{url}|{pixels}|{}|{}",
            account_user_id.as_deref().unwrap_or("anonymous"),
            account_epoch.as_deref().unwrap_or("none"),
        );
        if self.image_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.cancel_load();
        self.image_key = Some(key);
        *self.image.borrow_mut() = None;

        if let Some(hit) = CleverImageBitmapLoader::memory_image(
            &url,
            true,
            account_user_id.as_deref(),
            account_epoch.as_deref(),
            Some(pixels),
            CleverImageCachePolicy::Standard,
            None,
            false,
        ) {
            *self.image.borrow_mut() = Some(hit);
            return;
        }

        let generation = self.load_generation.get().wrapping_add(1);
        self.load_generation.set(generation);
        let current_generation = self.load_generation.clone();
        let slot = self.image.clone();
        let host = cx.entity_id();
        self.load_task = Some(cx.spawn(async move |_, cx| {
            let image = CleverImageBitmapLoader::load(
                &url,
                true,
                true,
                account_user_id,
                account_epoch,
                Some(pixels),
                CleverImageCachePolicy::Standard,
                CleverImageWorkLane::Visible,
                cx,
            )
            .await;
            let Some(image) = image else { return };
            if current_generation.get() != generation {
                return;
            }
            *slot.borrow_mut() = Some(image);
            cx.update(|cx| cx.notify(host));
        }));
    }

    pub fn reset(&mut self) {
        self.cancel_load();
        self.image_key = None;
        *self.image.borrow_mut() = None;
    }

    fn cancel_load(&mut self) {
        self.load_task = None;
        self.load_generation.set(self.load_generation.get().wrapping_add(1));
    }

    /// lays out the circle of side with the letter under the image
    pub fn element(&self, side: f32, alpha: f32) -> AnyElement {
        let mut circle = div()
            .relative()
            .size(px(side))
            .rounded_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .opacity(alpha);
        if let Some(background) = self.background {
            circle = circle.bg(background);
        }
        if let Some(letter_color) = self.letter_color {
            circle = circle.child(
                div()
                    .swiftui_font(Font::system_weight(side * 0.42, FontWeight::SEMIBOLD))
                    .text_color(letter_color)
                    .child(self.letter.clone()),
            );
        }
        if let Some(image) = self.image.borrow().clone() {
            circle = circle.child(img(image).absolute().inset_0().size_full().object_fit(ObjectFit::Cover));
        }
        circle.into_any_element()
    }
}

/// shared base for avatar pins centered on the fix
/// bounds are exactly the ring so only that takes the pointer
/// tile and wake are outside and never steal a map click
pub struct MapAvatarPinView {
    pub reuse_identifier: Option<SharedString>,
    pub avatar_view: MapAvatarView,
    pub tile_view: MapPinTileView,
    pub battery_view: MapPinBatteryView,
    pub reaction_badge: MapPinReactionBadge,
    pub trail_layer: MapPinTrailLayer,
    pub accessibility_label: SharedString,
    ring_fill: Option<Hsla>,
    avatar_alpha: f32,
    ring_width: f32,
    ring_side: f32,
    breath_phase: f64,
    is_breathing: bool,
    breath_started_at: Option<MotionTime>,
    pulse_started_at: Option<MotionTime>,
    zoom_scale: f32,
    highlight_scale: f32,
    stage_scale: Animated<f32>,
    shows_detail: bool,
    has_tile: bool,
    has_battery: bool,
    tile_hidden: bool,
    battery_hidden: bool,
    tile_alpha: Animated<f32>,
    battery_alpha: Animated<f32>,
    shown_trail: Option<MapPinTrail>,
    map_direction: f64,
    is_in_window: bool,
    is_highlighted: bool,
    is_hovered: bool,
    on_select: Option<MapPinSelectHandler>,
    window_centre: Rc<Cell<Option<Point<Pixels>>>>,
}

impl MapAvatarPinView {
    pub const AVATAR_SIDE: f32 = 31.0;
    const PULSE_DURATION: f64 = 0.2;
    const BREATH_DURATION: f64 = 2.4;
    const HIGHLIGHT_SCALE: f32 = 1.14;

    pub fn new(reuse_identifier: Option<SharedString>, ring_width: f32) -> Self {
        let ring_side = Self::AVATAR_SIDE + ring_width * 2.0;
        Self {
            reuse_identifier,
            avatar_view: MapAvatarView::new(),
            tile_view: MapPinTileView::default(),
            battery_view: MapPinBatteryView::default(),
            reaction_badge: MapPinReactionBadge::default(),
            trail_layer: MapPinTrailLayer::new(ring_side / 2.0 * 0.94),
            accessibility_label: SharedString::default(),
            ring_fill: None,
            avatar_alpha: 1.0,
            ring_width,
            ring_side,
            breath_phase: 0.0,
            is_breathing: false,
            breath_started_at: None,
            pulse_started_at: None,
            zoom_scale: 1.0,
            highlight_scale: 1.0,
            stage_scale: Animated::new(1.0),
            shows_detail: true,
            has_tile: false,
            has_battery: false,
            tile_hidden: true,
            battery_hidden: true,
            tile_alpha: Animated::new(1.0),
            battery_alpha: Animated::new(1.0),
            shown_trail: None,
            map_direction: 0.0,
            is_in_window: true,
            is_highlighted: false,
            is_hovered: false,
            on_select: None,
            window_centre: Rc::new(Cell::new(None)),
        }
    }

    /// box center in window coords from the last layout, none before the first one
    pub fn window_centre(&self) -> Option<Point<Pixels>> {
        self.window_centre.get()
    }

    /// side of the pin's box, the layer centers a box this size on the projected point
    pub fn ring_side(&self) -> f32 {
        self.ring_side
    }

    pub fn map_direction(&self) -> f64 {
        self.map_direction
    }

    pub fn set_on_select(&mut self, on_select: Option<MapPinSelectHandler>) {
        self.on_select = on_select;
    }

    pub fn prepare_for_reuse<V: 'static>(&mut self, cx: &mut Context<V>) {
        let now = RiseClock::monotonic();
        self.avatar_view.reset();
        self.shown_trail = None;
        self.trail_layer.hide(false, now, cx);
        self.reaction_badge.set(0, Default::default(), false, now);
        cx.notify();
    }

    pub fn set_reaction_count<V: 'static>(&mut self, count: i64, animated: bool, cx: &mut Context<V>) {
        let anchor = point(self.ring_side * 0.94, self.ring_side * 0.68);
        self.reaction_badge.set(count, anchor, animated, RiseClock::monotonic());
        cx.notify();
    }

    /// additive so it rides on top of the breathing and a burst doesnt stack a pulse per emoji
    pub fn pulse<V: 'static>(&mut self, cx: &mut Context<V>) {
        let now = RiseClock::monotonic();
        if self.pulse_offset(now).is_some() {
            return;
        }
        self.pulse_started_at = Some(now);
        cx.notify();
    }

    /// pins dont breathe in step, each starts at a different point of the cycle
    pub fn set_breath_seed(&mut self, seed: &str) {
        let mut hash: u32 = 2_166_136_261;
        for byte in seed.bytes() {
            hash = (hash ^ u32::from(byte)).wrapping_mul(16_777_619);
        }
        self.breath_phase = f64::from(hash % 1_000) / 1_000.0 * Self::BREATH_DURATION * 2.0;
    }

    pub fn set_tile(&mut self, tile: Option<&MapPinTile>, palette: &MapPinPalette) {
        self.tile_view.configure(tile, palette);
        self.has_tile = tile.is_some();
        self.apply_detail(false, RiseClock::monotonic());
    }

    pub fn set_battery(&mut self, level: Option<i64>, is_charging: bool, palette: &MapPinPalette) {
        self.has_battery = level.is_some();
        if let Some(level) = level {
            self.battery_view.configure(level, is_charging, palette);
        }
        self.apply_detail(false, RiseClock::monotonic());
    }

    /// wake is detail like the tile
    /// remembered while zoomed out and restored at street zoom
    pub fn set_trail(&mut self, trail: Option<&MapPinTrail>, color: Hsla, animated: bool, cx: &App) {
        let now = RiseClock::monotonic();
        self.trail_layer.set_color(color, now);
        self.shown_trail = trail.cloned();
        match trail {
            Some(trail) if self.shows_detail => {
                let screen = trail.bearing_degrees - self.map_direction;
                self.trail_layer.show(trail, screen, animated, now, cx);
            }
            _ => self.trail_layer.hide(animated, now, cx),
        }
    }

    /// size follows zoom and wake follows map rotation
    /// returns true if the map turned so a pin keeping a bearing turns too
    pub fn apply_camera(&mut self, zoom: f64, direction: f64, animated: bool, cx: &App) -> bool {
        let now = RiseClock::monotonic();
        let next_scale = MapPinZoom::scale(zoom);
        let next_detail = MapPinZoom::shows_detail(zoom);
        let direction_changed = MapPinAngle::shortest(direction - self.map_direction).abs() > 0.5;
        if direction_changed {
            self.map_direction = direction;
            self.camera_turned();
        }
        if next_detail != self.shows_detail {
            self.shows_detail = next_detail;
            self.apply_detail(animated, now);
            match self.shown_trail.as_ref() {
                Some(trail) if next_detail => {
                    let screen = trail.bearing_degrees - self.map_direction;
                    self.trail_layer.show(trail, screen, animated, now, cx);
                }
                _ => self.trail_layer.hide(animated, now, cx),
            }
        }
        if (next_scale - self.zoom_scale).abs() > 0.004 {
            self.zoom_scale = next_scale;
            self.apply_scale(false, now);
        }
        direction_changed
    }

    pub fn camera_turned(&mut self) {
        if let Some(trail) = &self.shown_trail {
            let screen = trail.bearing_degrees - self.map_direction;
            self.trail_layer.turn(screen, RiseClock::monotonic());
        }
    }

    pub fn set_highlighted(&mut self, highlighted: bool, animated: bool) {
        self.is_highlighted = highlighted;
        self.highlight_scale = if highlighted || self.is_hovered { Self::HIGHLIGHT_SCALE } else { 1.0 };
        self.apply_scale(animated, RiseClock::monotonic());
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        if hovered == self.is_hovered {
            return;
        }
        self.is_hovered = hovered;
        self.set_highlighted(self.is_highlighted, true);
    }

    fn apply_scale(&mut self, animated: bool, now: MotionTime) {
        let scale = self.zoom_scale * self.highlight_scale;
        if animated {
            self.stage_scale.set(scale, spring(0.28, 0.7), now, false);
        } else {
            self.stage_scale.jump(scale);
        }
    }

    fn apply_detail(&mut self, animated: bool, now: MotionTime) {
        let tile_hidden = !(self.has_tile && self.shows_detail);
        let battery_hidden = !(self.has_battery && self.shows_detail);
        if !animated {
            self.tile_hidden = tile_hidden;
            self.battery_hidden = battery_hidden;
            self.tile_alpha.jump(1.0);
            self.battery_alpha.jump(1.0);
            return;
        }
        for (is_hidden, alpha, hidden) in [
            (&mut self.tile_hidden, &mut self.tile_alpha, tile_hidden),
            (&mut self.battery_hidden, &mut self.battery_alpha, battery_hidden),
        ] {
            if *is_hidden == hidden {
                continue;
            }
            if !hidden {
                alpha.jump(0.0);
            }
            *is_hidden = hidden;
            alpha.set(if hidden { 0.0 } else { 1.0 }, ease_in_out(0.2), now, false);
        }
    }

    pub fn refresh_motion(&mut self, cx: &App) {
        let now = RiseClock::monotonic();
        self.trail_layer.refresh_motion(now, cx);
        let wanted = self.is_in_window && MapPinMotion::is_allowed(cx);
        if wanted && self.is_breathing && self.breath_started_at.is_some() {
            return;
        }
        self.is_breathing = wanted;
        self.breath_started_at = wanted.then_some(now);
    }

    pub fn did_move_to_window(&mut self, in_window: bool, cx: &App) {
        self.is_in_window = in_window;
        self.refresh_motion(cx);
    }

    fn breath_scale(&self, now: MotionTime) -> f32 {
        let Some(started) = self.breath_started_at.filter(|_| self.is_breathing) else {
            return 1.0;
        };
        let elapsed = now.saturating_sub(started).as_secs_f64() + self.breath_phase;
        1.0 + 0.04 * autoreversing_progress(elapsed, Self::BREATH_DURATION) as f32
    }

    /// pulse's additive scale at now, none once it has run
    fn pulse_offset(&self, now: MotionTime) -> Option<f32> {
        let started = self.pulse_started_at?;
        let elapsed = now.saturating_sub(started).as_secs_f64();
        if elapsed >= Self::PULSE_DURATION {
            return None;
        }
        let progress = CubicBezier::EASE_OUT.progress(elapsed / Self::PULSE_DURATION) as f32;
        Some(if progress < 0.35 { 0.12 * progress / 0.35 } else { 0.12 * (1.0 - (progress - 0.35) / 0.65) })
    }

    fn is_running(&self, now: MotionTime) -> bool {
        self.is_breathing
            || self.pulse_offset(now).is_some()
            || self.stage_scale.is_running(now)
            || self.tile_alpha.is_running(now)
            || self.battery_alpha.is_running(now)
            || self.trail_layer.is_running(now)
            || self.reaction_badge.is_running(now)
    }

    /// the pin's box with everything it draws
    /// cone and inner ring (white ring inset) belong to the reader's own pin
    pub fn stage(
        &mut self,
        cone: Option<MapPinConeFrame>,
        inner_ring: Option<f32>,
        now: MotionTime,
        window: &mut Window,
    ) -> Div {
        if self.is_running(now) {
            window.request_animation_frame();
        }
        let ring_side = self.ring_side;
        let centre = ring_side / 2.0;
        let stage = self.stage_scale.value(now);
        let body = stage * (self.breath_scale(now) + self.pulse_offset(now).unwrap_or(0.0));
        let at = |value: f32| centre + (value - centre) * stage;

        let window_centre = Rc::clone(&self.window_centre);
        let mut root = div().relative().size(px(ring_side)).child(
            canvas(move |bounds, _, _| window_centre.set(Some(bounds.center())), |_, (), _, _| {})
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .size(px(ring_side)),
        );

        let trail = self.trail_layer.frame(now);
        if cone.is_some() || trail.is_some() {
            let side = MapPinTrailLayer::side().max(MapPinConeLayer::side()) * stage;
            root = root.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, _| {
                        let middle = bounds.center();
                        if let Some(cone) = cone {
                            MapPinConeLayer::paint(cone, middle, stage, window);
                        }
                        if let Some(trail) = &trail {
                            MapPinTrailLayer::paint(trail, middle, stage, window);
                        }
                    },
                )
                .absolute()
                .left(px(centre - side / 2.0))
                .top(px(centre - side / 2.0))
                .size(px(side)),
            );
        }

        let body_side = ring_side * body;
        let mut ring = div()
            .absolute()
            .left(px(centre - body_side / 2.0))
            .top(px(centre - body_side / 2.0))
            .size(px(body_side))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .shadow(vec![BoxShadow {
                color: black().opacity(0.3),
                offset: point(px(0.0), px(1.5 * body)),
                blur_radius: px(4.0 * body),
                spread_radius: px(0.0),
                inset: false,
            }]);
        if let Some(fill) = self.ring_fill {
            ring = ring.bg(fill);
        }
        if let Some(inset) = inner_ring {
            let inner_side = (ring_side - inset * 2.0) * body;
            ring = ring.child(
                div()
                    .absolute()
                    .left(px((body_side - inner_side) / 2.0))
                    .top(px((body_side - inner_side) / 2.0))
                    .size(px(inner_side))
                    .rounded_full()
                    .bg(white()),
            );
        }
        let avatar_side = (ring_side - self.ring_width * 2.0) * body;
        ring = ring.child(self.avatar_view.element(avatar_side, self.avatar_alpha));
        root = root.child(ring);

        let tile_alpha = self.tile_alpha.value(now);
        if !self.tile_hidden || self.tile_alpha.is_running(now) {
            if let Some(tile) = self.tile_view.element(stage, tile_alpha) {
                root = root
                    .child(div().absolute().left(px(at(ring_side * 0.66))).bottom(px(ring_side - at(ring_side * 0.34))).child(tile));
            }
        }

        let battery_alpha = self.battery_alpha.value(now);
        if !self.battery_hidden || self.battery_alpha.is_running(now) {
            if let Some(pill) = self.battery_view.element(stage, battery_alpha) {
                root = root.child(
                    div()
                        .absolute()
                        .left(px(centre))
                        .top(px(at(ring_side - 3.0)))
                        .w(px(0.0))
                        .flex()
                        .justify_center()
                        .child(pill),
                );
            }
        }

        if let Some(badge) = self.reaction_badge.element(stage, now) {
            let shift = centre * (1.0 - stage);
            root = root.child(div().absolute().left(px(shift)).top(px(shift)).size(px(0.0)).child(badge));
        }
        root
    }

    /// the box that takes the pointer, hand cursor and hover highlight and selection on click
    pub fn interactive<V: 'static>(
        &self,
        id: impl Into<gpui::ElementId>,
        stage: Div,
        base: fn(&mut V) -> &mut MapAvatarPinView,
        cx: &mut Context<V>,
    ) -> impl IntoElement {
        let on_select = self.on_select.clone();
        stage
            .id(id)
            .rounded_full()
            .cursor_pointer()
            .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                base(view).set_hovered(*hovered);
                cx.notify();
            }))
            .on_click(move |_, window, cx| {
                if let Some(on_select) = &on_select {
                    on_select(window, cx);
                }
            })
    }
}

/// one friend on the map, avatar in a white ring plus dwell time or speed and battery
pub struct MapFriendPinView {
    base: MapAvatarPinView,
    model: Option<MapPinModel>,
    palette: Option<MapPinPalette>,
}

impl std::ops::Deref for MapFriendPinView {
    type Target = MapAvatarPinView;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl std::ops::DerefMut for MapFriendPinView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl MapFriendPinView {
    pub const REUSE_ID: &'static str = "map.friend.pin";

    pub fn new(reuse_identifier: Option<SharedString>, _cx: &mut Context<Self>) -> Self {
        Self { base: MapAvatarPinView::new(reuse_identifier, 2.5), model: None, palette: None }
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.base.prepare_for_reuse(cx);
        self.model = None;
    }

    pub fn apply_camera(&mut self, zoom: f64, direction: f64, animated: bool, cx: &mut Context<Self>) {
        self.base.apply_camera(zoom, direction, animated, cx);
        cx.notify();
    }

    pub fn set_highlighted(&mut self, highlighted: bool, animated: bool, cx: &mut Context<Self>) {
        self.base.set_highlighted(highlighted, animated);
        cx.notify();
    }

    pub fn refresh_motion(&mut self, cx: &mut Context<Self>) {
        self.base.refresh_motion(cx);
        cx.notify();
    }

    pub fn configure(&mut self, model: &MapPinModel, palette: &MapPinPalette, cx: &mut Context<Self>) {
        let palette_changed = self.palette.as_ref() != Some(palette);
        if self.model.as_ref() == Some(model) && !palette_changed {
            return;
        }
        let is_same_friend = self.model.as_ref().is_some_and(|shown| shown.id == model.id);
        self.model = Some(model.clone());
        self.palette = Some(*palette);

        self.base.ring_fill = Some(palette.ring);
        self.base.avatar_view.configure(
            &model.logo,
            &model.name,
            &model.id,
            MapAvatarPinView::AVATAR_SIDE,
            palette,
            cx,
        );
        self.base.avatar_alpha = if model.status.is_live() { 1.0 } else { 0.6 };

        self.base.set_breath_seed(&model.id);
        let appearance = match ThemeStore::shared(cx).read(cx).resolved_appearance() {
            RiseThemeAppearance::Light => Appearance::Light,
            RiseThemeAppearance::Dark => Appearance::Dark,
        };
        self.base.reaction_badge.set_color(UIColor::system_red().resolved_color(appearance));
        if !is_same_friend {
            let count = MapReactionStage::shared(cx).read(cx).count(&model.id);
            self.base.set_reaction_count(count, false, cx);
        }
        self.base.set_tile(model.status.tile.as_ref(), palette);
        self.base.set_battery(model.battery_level, model.is_charging, palette);
        self.base.set_trail(model.trail.as_ref(), palette.primary, is_same_friend, cx);

        self.base.accessibility_label = format!("{}, {}", model.name, model.status.text).into();
        self.base.refresh_motion(cx);
        cx.notify();
    }
}

impl Render for MapFriendPinView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let stage = self.base.stage(None, None, now, window);
        let id = SharedString::from(format!(
            "map-friend-pin-{}",
            self.model.as_ref().map(|model| model.id.as_str()).unwrap_or_default()
        ));
        self.base.interactive(id, stage, Self::base_of, cx)
    }
}

impl MapFriendPinView {
    fn base_of(view: &mut Self) -> &mut MapAvatarPinView {
        &mut view.base
    }
}

/// the reader's own pin, accent ring plus heading beam
/// driven by device fixes so it works offline
pub struct MapSelfPinView {
    base: MapAvatarPinView,
    cone_layer: MapPinConeLayer,
    heading: Option<MapSelfHeading>,
    profile_id: Option<String>,
}

impl std::ops::Deref for MapSelfPinView {
    type Target = MapAvatarPinView;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl std::ops::DerefMut for MapSelfPinView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl MapSelfPinView {
    pub const REUSE_ID: &'static str = "map.self.pin";
    const OUTER_RING: f32 = 3.0;
    const INNER_RING: f32 = 1.5;

    pub const Z_POSITION: i32 = 1;

    pub fn new(reuse_identifier: Option<SharedString>, _cx: &mut Context<Self>) -> Self {
        Self {
            base: MapAvatarPinView::new(reuse_identifier, Self::OUTER_RING + Self::INNER_RING),
            cone_layer: MapPinConeLayer::new(),
            heading: None,
            profile_id: None,
        }
    }

    pub fn configure(&mut self, profile: &MapSelfProfile, palette: &MapPinPalette, cx: &mut Context<Self>) {
        self.base.ring_fill = Some(palette.primary);
        self.cone_layer.set_color(palette.primary);
        self.base.avatar_view.configure(
            &profile.logo,
            &profile.name,
            &profile.id,
            MapAvatarPinView::AVATAR_SIDE,
            palette,
            cx,
        );
        self.base.set_breath_seed(&profile.id);
        self.profile_id = Some(profile.id.clone());
        self.base.accessibility_label = profile.name.clone().into();
        self.base.refresh_motion(cx);
        cx.notify();
    }

    pub fn apply(
        &mut self,
        state: &MapSelfPinState,
        palette: &MapPinPalette,
        animated: bool,
        cx: &mut Context<Self>,
    ) {
        self.base.set_tile(state.tile.as_ref(), palette);
        self.base.set_trail(state.trail.as_ref(), palette.primary, animated, cx);
        cx.notify();
    }

    pub fn apply_heading(&mut self, next: Option<&MapSelfHeading>, animated: bool, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        self.heading = next.cloned();
        match self.heading.as_ref() {
            Some(next) => self.cone_layer.show(
                next.degrees - self.base.map_direction(),
                next.accuracy_degrees,
                animated,
                now,
            ),
            None => self.cone_layer.hide(animated, now),
        }
        cx.notify();
    }

    pub fn apply_camera(&mut self, zoom: f64, direction: f64, animated: bool, cx: &mut Context<Self>) {
        if self.base.apply_camera(zoom, direction, animated, cx) {
            self.turn_cone();
        }
        cx.notify();
    }

    pub fn set_highlighted(&mut self, highlighted: bool, animated: bool, cx: &mut Context<Self>) {
        self.base.set_highlighted(highlighted, animated);
        cx.notify();
    }

    pub fn refresh_motion(&mut self, cx: &mut Context<Self>) {
        self.base.refresh_motion(cx);
        cx.notify();
    }

    pub fn camera_turned(&mut self) {
        self.base.camera_turned();
        self.turn_cone();
    }

    fn turn_cone(&mut self) {
        let Some(heading) = self.heading.as_ref() else { return };
        self.cone_layer.show(
            heading.degrees - self.base.map_direction(),
            heading.accuracy_degrees,
            false,
            RiseClock::monotonic(),
        );
    }
}

impl Render for MapSelfPinView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        if self.cone_layer.is_running(now) {
            window.request_animation_frame();
        }
        let cone = self.cone_layer.frame(now);
        let stage = self.base.stage(cone, Some(Self::OUTER_RING), now, window);
        let id = SharedString::from(format!(
            "map-self-pin-{}",
            self.profile_id.as_deref().unwrap_or_default()
        ));
        self.base.interactive(id, stage, Self::base_of, cx)
    }
}

impl MapSelfPinView {
    fn base_of(view: &mut Self) -> &mut MapAvatarPinView {
        &mut view.base
    }
}

/// friends too close to tell apart at this zoom
/// up to three fanned avatars and a head count
/// click zooms in until they separate
pub struct MapFriendStackPinView {
    pub reuse_identifier: Option<SharedString>,
    pub accessibility_label: SharedString,
    avatars: Vec<MapAvatarView>,
    rings: Vec<Option<Hsla>>,
    badge_text: SharedString,
    badge_colors: Option<(Hsla, Hsla)>,
    member_ids: Vec<String>,
    applied_scale: f32,
    on_select: Option<MapPinSelectHandler>,
}

impl MapFriendStackPinView {
    pub const REUSE_ID: &'static str = "map.friend.stack";
    const AVATAR_SIDE: f32 = 26.0;
    const STEP: f32 = 14.0;
    const RING_WIDTH: f32 = 2.0;
    const BADGE_SIDE: f32 = 16.0;
    const BADGE_FONT_SIZE: f32 = 9.5;

    pub fn new(reuse_identifier: Option<SharedString>, _cx: &mut Context<Self>) -> Self {
        Self {
            reuse_identifier,
            accessibility_label: SharedString::default(),
            avatars: (0..3).map(|_| MapAvatarView::new()).collect(),
            rings: vec![None; 3],
            badge_text: SharedString::default(),
            badge_colors: None,
            member_ids: Vec::new(),
            applied_scale: 1.0,
            on_select: None,
        }
    }

    fn ring_side() -> f32 {
        Self::AVATAR_SIDE + Self::RING_WIDTH * 2.0
    }

    /// stack box width and height before the zoom scale, the layer centers it on the point
    pub fn frame_size() -> (f32, f32) {
        (Self::ring_side() + Self::STEP * 2.0 + 10.0, Self::ring_side() + 8.0)
    }

    pub fn set_on_select(&mut self, on_select: Option<MapPinSelectHandler>) {
        self.on_select = on_select;
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.member_ids.clear();
        for avatar in &mut self.avatars {
            avatar.reset();
        }
        cx.notify();
    }

    /// stack shrinks with the single pins so a far view stays a map view
    pub fn apply_zoom(&mut self, zoom: f64, cx: &mut Context<Self>) {
        let scale = MapPinZoom::scale(zoom);
        if (scale - self.applied_scale).abs() <= 0.004 {
            return;
        }
        self.applied_scale = scale;
        cx.notify();
    }

    pub fn configure(&mut self, members: &[MapLiveFriend], palette: &MapPinPalette, cx: &mut Context<Self>) {
        let ids: Vec<String> = members.iter().map(|member| member.id().to_owned()).collect();
        if ids == self.member_ids {
            return;
        }
        self.member_ids = ids;

        let shown = &members[..members.len().min(3)];
        for (index, (ring, avatar)) in self.rings.iter_mut().zip(self.avatars.iter_mut()).enumerate() {
            let Some(friend) = shown.get(index) else {
                *ring = None;
                continue;
            };
            *ring = Some(palette.ring);
            avatar.configure(
                &friend.user.logo,
                friend.user.display_name(),
                friend.id(),
                Self::AVATAR_SIDE,
                palette,
                cx,
            );
        }

        self.badge_text = members.len().to_string().into();
        self.badge_colors = Some((palette.plate, palette.plate_text));
        self.accessibility_label =
            shown.iter().map(|member| member.user.display_name()).collect::<Vec<_>>().join(", ").into();
        cx.notify();
    }

    fn badge_font() -> Font {
        Font::system_weight(Self::BADGE_FONT_SIZE, FontWeight::BOLD).monospaced_digit()
    }

    fn badge_text_width(&self, window: &Window) -> f32 {
        let font = Self::badge_font();
        let run = TextRun {
            len: self.badge_text.len(),
            font: gpui::Font {
                family: font.family(),
                features: font.features(),
                fallbacks: None,
                weight: font.weight,
                style: FontStyle::Normal,
            },
            color: white(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(self.badge_text.clone(), px(Self::BADGE_FONT_SIZE), &[run], None);
        f32::from(line.width)
    }
}

impl Render for MapFriendStackPinView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (width, height) = Self::frame_size();
        let scale = self.applied_scale;
        let ring_side = Self::ring_side();
        let at_x = |x: f32| width / 2.0 + (x - width / 2.0) * scale;
        let at_y = |y: f32| height / 2.0 + (y - height / 2.0) * scale;

        let mut root = div().relative().w(px(width)).h(px(height));
        let mut shown: usize = 0;
        for (index, (ring, avatar)) in self.rings.iter().zip(&self.avatars).enumerate() {
            let Some(fill) = ring else { continue };
            shown += 1;
            let x = index as f32 * Self::STEP;
            root = root.child(
                div()
                    .absolute()
                    .left(px(at_x(x)))
                    .top(px(at_y(4.0)))
                    .size(px(ring_side * scale))
                    .rounded_full()
                    .bg(*fill)
                    .flex()
                    .items_center()
                    .justify_center()
                    .shadow(vec![BoxShadow {
                        color: black().opacity(0.26),
                        offset: point(px(0.0), px(scale)),
                        blur_radius: px(3.0 * scale),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .child(avatar.element(Self::AVATAR_SIDE * scale, 1.0)),
            );
        }

        if let Some((plate, plate_text)) = self.badge_colors {
            let last_max_x = shown.saturating_sub(1) as f32 * Self::STEP + ring_side;
            let badge_width = Self::BADGE_SIDE.max(self.badge_text_width(window).ceil() + 10.0);
            let x = (width - badge_width).min(last_max_x - badge_width / 2.0);
            root = root.child(
                div()
                    .absolute()
                    .left(px(at_x(x)))
                    .top(px(at_y(0.0)))
                    .w(px(badge_width * scale))
                    .h(px(Self::BADGE_SIDE * scale))
                    .rounded(px(Self::BADGE_SIDE / 2.0 * scale))
                    .overflow_hidden()
                    .bg(plate)
                    .flex()
                    .items_center()
                    .justify_center()
                    .swiftui_font(Font::system_weight(Self::BADGE_FONT_SIZE * scale, FontWeight::BOLD).monospaced_digit())
                    .text_color(plate_text)
                    .child(self.badge_text.clone()),
            );
        }

        let on_select = self.on_select.clone();
        let id = SharedString::from(format!("map-friend-stack-{}", self.member_ids.join("-")));
        root.id(id)
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                if let Some(on_select) = &on_select {
                    on_select(window, cx);
                }
            })
    }
}
