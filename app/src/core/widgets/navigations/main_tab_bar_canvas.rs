use std::collections::HashSet;

use gpui::{
    AnyElement, App, AppContext, BoxShadow, Context, Div, Entity, Hsla, IntoElement, ParentElement, Pixels, Render,
    RenderOnce, Size, Styled, WeakEntity, Window, div, hsla, point, prelude::FluentBuilder, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::environment::env_scope;
use rise_ui::swiftui::environment_values::{ColorScheme, ColorSchemeKey};
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};
use rise_widgets::navigations::main_tab_bar_motion::{CFTimeInterval, MainTabBarMotion, MainTabBarTrack};
use rise_widgets::navigations::tab_strip_math::TabStripLayout;

use crate::core::widgets::navigations::main_tab_bar::{MainTabBarItem, MainTabBarMetrics};
use crate::core::widgets::navigations::main_tab_bar_parts::MainTabBarItemLabel;

fn current_media_time() -> CFTimeInterval {
    RiseClock::monotonic().as_secs_f64()
}

/// values sent by one move or chain, none keeps that value's motion
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MainTabBarCanvasTargets {
    pub scroll: Option<f32>,
    pub pill_x: Option<f32>,
    pub lift: Option<f32>,
    pub stretch: Option<f32>,
}

/// pose of the bar, four values each with its own motion
/// the canvas samples the tracks and draws the pose
pub struct MainTabBarCanvasDriver {
    scroll: MainTabBarTrack,
    pill_x: MainTabBarTrack,
    lift: MainTabBarTrack,
    stretch: MainTabBarTrack,
    canvas: Option<Entity<MainTabBarCanvasView>>,
}

impl Default for MainTabBarCanvasDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl MainTabBarCanvasDriver {
    pub fn new() -> Self {
        Self {
            scroll: MainTabBarTrack::new(0.0, 0.05),
            pill_x: MainTabBarTrack::new(0.0, 0.05),
            lift: MainTabBarTrack::new(0.0, 0.001),
            stretch: MainTabBarTrack::new(0.0, 0.0005),
            canvas: None,
        }
    }

    pub fn scroll(&self) -> &MainTabBarTrack {
        &self.scroll
    }

    pub fn pill_x(&self) -> &MainTabBarTrack {
        &self.pill_x
    }

    pub fn lift(&self) -> &MainTabBarTrack {
        &self.lift
    }

    pub fn stretch(&self) -> &MainTabBarTrack {
        &self.stretch
    }

    pub fn attach(&mut self, cx: &mut Context<Self>) -> Entity<MainTabBarCanvasView> {
        if let Some(canvas) = &self.canvas {
            return canvas.clone();
        }
        let driver = cx.entity().downgrade();
        let canvas = cx.new(|_| MainTabBarCanvasView::new(driver));
        self.canvas = Some(canvas.clone());
        canvas
    }

    /// sends the given values to new targets, the others keep their motion
    pub fn r#move(&mut self, motion: MainTabBarMotion, targets: MainTabBarCanvasTargets, cx: &mut Context<Self>) {
        let now = current_media_time();
        if let Some(scroll) = targets.scroll {
            self.scroll.r#move(scroll, motion, now);
        }
        if let Some(pill_x) = targets.pill_x {
            self.pill_x.r#move(pill_x, motion, now);
        }
        if let Some(lift) = targets.lift {
            self.lift.r#move(lift, motion, now);
        }
        if let Some(stretch) = targets.stretch {
            self.stretch.r#move(stretch, motion, now);
        }
        self.render_canvas(cx);
    }

    /// motion that follows the current one after delay, already on the tracks
    /// so it starts on time even if the main thread is busy
    pub fn chain(
        &mut self,
        delay: CFTimeInterval,
        motion: MainTabBarMotion,
        targets: MainTabBarCanvasTargets,
        cx: &mut Context<Self>,
    ) {
        let start = current_media_time() + delay.max(0.0);
        if let Some(scroll) = targets.scroll {
            self.scroll.chain(scroll, motion, start);
        }
        if let Some(pill_x) = targets.pill_x {
            self.pill_x.chain(pill_x, motion, start);
        }
        if let Some(lift) = targets.lift {
            self.lift.chain(lift, motion, start);
        }
        if let Some(stretch) = targets.stretch {
            self.stretch.chain(stretch, motion, start);
        }
        self.render_canvas(cx);
    }

    /// a touch keeps the bar where its going, nothing chained starts after it
    pub fn cancel_chain(&mut self, cx: &mut Context<Self>) {
        let now = current_media_time();
        self.scroll.cancel_chain(now);
        self.pill_x.cancel_chain(now);
        self.lift.cancel_chain(now);
        self.stretch.cancel_chain(now);
        self.render_canvas(cx);
    }

    fn settle_time(&self) -> CFTimeInterval {
        self.scroll
            .settle_time()
            .max(self.pill_x.settle_time())
            .max(self.lift.settle_time())
            .max(self.stretch.settle_time())
    }

    fn render_canvas(&self, cx: &mut Context<Self>) {
        if let Some(canvas) = &self.canvas {
            canvas.update(cx, |_, cx| cx.notify());
        }
    }
}

/// what the bar draws besides its pose, parts that change only with layout or settings or theme
#[derive(Clone, Debug, PartialEq)]
pub struct MainTabBarScene {
    pub width: f32,
    pub layout: TabStripLayout,
    pub mirrored: bool,
    pub uses_glass: bool,
    pub dark: bool,
    pub glass_tint: Hsla,
    pub media_contrast: bool,
    pub reduce_transparency: bool,
    pub items: Vec<MainTabBarItem>,
    pub slots: Vec<isize>,
    pub anchored: HashSet<isize>,
    pub fades_left: bool,
    pub fades_right: bool,
}

impl MainTabBarScene {
    pub fn same_geometry(&self, other: &MainTabBarScene) -> bool {
        self.width == other.width
            && self.layout == other.layout
            && self.mirrored == other.mirrored
            && self.uses_glass == other.uses_glass
    }
}

#[derive(IntoElement)]
pub struct MainTabBarCanvas {
    driver: Entity<MainTabBarCanvasDriver>,
    scene: MainTabBarScene,
}

impl MainTabBarCanvas {
    pub fn new(driver: Entity<MainTabBarCanvasDriver>, scene: MainTabBarScene) -> Self {
        Self { driver, scene }
    }
}

impl RenderOnce for MainTabBarCanvas {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let width = self.scene.width;
        let view = self.driver.update(cx, |driver, cx| driver.attach(cx));
        view.update(cx, |view, _| view.update(self.scene));
        div().flex_none().w(px(width)).h(px(MainTabBarMetrics::HEIGHT)).child(view)
    }
}

/// two end alphas of the edge fade, eased over a quarter second
#[derive(Clone, Copy, Debug)]
struct EdgeFade {
    from: (f32, f32),
    to: (f32, f32),
    begin: CFTimeInterval,
}

impl EdgeFade {
    const DURATION: CFTimeInterval = 0.25;

    fn alphas(scene: &MainTabBarScene) -> (f32, f32) {
        (if scene.fades_left { 0.0 } else { 1.0 }, if scene.fades_right { 0.0 } else { 1.0 })
    }

    fn progress(&self, now: CFTimeInterval) -> f32 {
        ((now - self.begin) / Self::DURATION).clamp(0.0, 1.0) as f32
    }

    fn current(&self, now: CFTimeInterval) -> (f32, f32) {
        let t = self.progress(now);
        (self.from.0 + (self.to.0 - self.from.0) * t, self.from.1 + (self.to.1 - self.from.1) * t)
    }

    fn is_running(&self, now: CFTimeInterval) -> bool {
        self.progress(now) < 1.0 && self.from != self.to
    }
}

#[derive(Clone, Copy, Debug)]
struct Pose {
    scroll: f32,
    pill_x: f32,
    lift: f32,
    stretch: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Frame {
    fn mid_x(&self) -> f32 {
        self.x + self.width / 2.0
    }

    fn mid_y(&self) -> f32 {
        self.y + self.height / 2.0
    }

    fn max_x(&self) -> f32 {
        self.x + self.width
    }

    fn radius(&self) -> f32 {
        self.width.min(self.height) / 2.0
    }

    /// absolutely placed capsule covering this frame
    fn capsule(&self) -> Div {
        div()
            .absolute()
            .left(px(self.x))
            .top(px(self.y))
            .w(px(self.width.max(0.0)))
            .h(px(self.height.max(0.0)))
            .rounded(px(self.radius().max(0.0)))
    }
}

#[derive(Clone, Copy, Debug)]
struct Geometry {
    lens: Frame,
    reach: Frame,
    droplet: Frame,
    shown_lift: f32,
    shift: f32,
    zoom: f32,
}

/// the bar as layers: surface, pill and two copies of the buttons
/// the tinted copy is seen only through the pill
pub struct MainTabBarCanvasView {
    driver: WeakEntity<MainTabBarCanvasDriver>,
    scene: Option<MainTabBarScene>,
    fade: EdgeFade,
}

impl MainTabBarCanvasView {
    const SAMPLE_STEP: CFTimeInterval = 1.0 / 120.0;

    fn new(driver: WeakEntity<MainTabBarCanvasDriver>) -> Self {
        Self { driver, scene: None, fade: EdgeFade { from: (1.0, 1.0), to: (1.0, 1.0), begin: 0.0 } }
    }

    pub fn update(&mut self, scene: MainTabBarScene) {
        if self.scene.as_ref() == Some(&scene) {
            return;
        }
        let animated_fade = self.scene.as_ref().is_some_and(|old| old.width == scene.width);
        let alphas = EdgeFade::alphas(&scene);
        let now = current_media_time();
        self.fade = if animated_fade {
            EdgeFade { from: self.fade.current(now), to: alphas, begin: now }
        } else {
            EdgeFade { from: alphas, to: alphas, begin: now }
        };
        self.scene = Some(scene);
    }

    fn resting_x(slot: isize, scene: &MainTabBarScene) -> f32 {
        let slot_width = scene.layout.slot_width();
        let x = MainTabBarMetrics::INSET + slot as f32 * slot_width;
        if scene.mirrored { scene.width - x - slot_width } else { x }
    }

    fn geometry(pose: Pose, scene: &MainTabBarScene) -> Geometry {
        let width = scene.width;
        let height = MainTabBarMetrics::HEIGHT;
        let outset = if scene.uses_glass { MainTabBarMetrics::DROPLET_OUTSET } else { MainTabBarMetrics::LIFTED_OUTSET };
        let widen = if scene.uses_glass { MainTabBarMetrics::DROPLET_WIDEN } else { MainTabBarMetrics::LIFTED_WIDEN };
        let shown = pose.lift.clamp(0.0, 1.0);

        let rest_width = scene.layout.slot_width();
        let rest_height = MainTabBarMetrics::ITEM_HEIGHT;
        let lens_width = ((rest_width + widen * pose.lift) * (1.0 + pose.stretch)).max(0.0);
        let lens_height =
            ((rest_height + (height + outset * 2.0 - rest_height) * pose.lift) * (1.0 - pose.stretch * 0.35)).max(0.0);
        let leading = MainTabBarMetrics::INSET + pose.pill_x - pose.scroll;
        let mid_x = if scene.mirrored { width - leading } else { leading };
        let lens = Frame { x: mid_x - lens_width / 2.0, y: (height - lens_height) / 2.0, width: lens_width, height: lens_height };

        let inset = MainTabBarMetrics::INSET * 2.0;
        let reach_width = width - inset + (inset + widen + 16.0) * shown;
        let reach_height = height - inset + (inset + outset * 2.0 + 8.0) * shown;
        let reach = Frame {
            x: (width - reach_width) / 2.0,
            y: (height - reach_height) / 2.0,
            width: reach_width,
            height: reach_height,
        };

        let droplet_reach = (MainTabBarMetrics::INSET + MainTabBarMetrics::DROPLET_WIDEN / 2.0 + 8.0) * shown;
        let lower = MainTabBarMetrics::INSET + lens_width / 2.0 - droplet_reach;
        let upper = width - MainTabBarMetrics::INSET - lens_width / 2.0 + droplet_reach;
        let droplet_mid = if lower <= upper { mid_x.clamp(lower, upper) } else { width / 2.0 };
        let droplet = Frame { x: droplet_mid - lens_width / 2.0, y: lens.y, width: lens_width, height: lens_height };

        Geometry {
            lens,
            reach,
            droplet,
            shown_lift: shown,
            shift: if scene.mirrored { pose.scroll } else { -pose.scroll },
            zoom: 1.0 + (MainTabBarMetrics::MAGNIFICATION - 1.0) * pose.lift,
        }
    }

    fn fade_alpha(x: f32, width: f32, alphas: (f32, f32)) -> f32 {
        let fade = MainTabBarMetrics::EDGE_FADE;
        if width <= 0.0 {
            return 1.0;
        }
        if x <= fade {
            let t = (x / fade).clamp(0.0, 1.0);
            alphas.0 + (1.0 - alphas.0) * t
        } else if x >= width - fade {
            let t = ((width - x) / fade).clamp(0.0, 1.0);
            alphas.1 + (1.0 - alphas.1) * t
        } else {
            1.0
        }
    }

    /// rectangle of the bar that shows only what is inside it, child is placed in bar coordinates
    fn clip(frame: Frame, rounded: bool, child: impl IntoElement) -> Div {
        div()
            .absolute()
            .left(px(frame.x))
            .top(px(frame.y))
            .w(px(frame.width.max(0.0)))
            .h(px(frame.height.max(0.0)))
            .when(rounded, |clip| clip.rounded(px(frame.radius().max(0.0))))
            .overflow_hidden()
            .child(div().absolute().left(px(-frame.x)).top(px(-frame.y)).child(child))
    }

    fn row(scene: &MainTabBarScene, tinted: bool, shift: f32, fades: Option<(f32, f32)>) -> Option<Div> {
        let layout = scene.layout;
        let slot_width = layout.slot_width();
        if slot_width <= 0.0 || scene.slots.is_empty() || scene.items.is_empty() {
            return None;
        }
        let positions: Vec<f32> = scene.slots.iter().map(|slot| Self::resting_x(*slot, scene)).collect();
        let origin = positions.iter().copied().fold(f32::INFINITY, f32::min);
        let span = positions.iter().copied().fold(f32::NEG_INFINITY, f32::max) + slot_width - origin;
        let left = origin + shift;
        let alphas = positions
            .iter()
            .map(|x| fades.map_or(1.0, |fades| Self::fade_alpha(left - origin + x + slot_width / 2.0, scene.width, fades)))
            .collect();
        Some(
            div().absolute().left(px(left)).top_0().child(MainTabBarStripRow {
                items: scene.items.clone(),
                slots: scene.slots.clone(),
                positions: positions.iter().map(|x| x - origin).collect(),
                layout,
                size: size(px(span), px(MainTabBarMetrics::HEIGHT)),
                tinted,
                anchored: if tinted { HashSet::new() } else { scene.anchored.clone() },
                alphas,
            }),
        )
    }

    /// lit edge of the lens, bright rim with a soft glow inside
    /// a border takes one color so the rim carries the lit end of the diagonal gradient
    fn rim(lens: Frame, dark: bool, shown: f32) -> Div {
        let rim = if dark { hsla(0.0, 0.0, 1.0, 0.55) } else { hsla(0.0, 0.0, 1.0, 0.95) };
        let glow = hsla(0.0, 0.0, 1.0, if dark { 0.09 } else { 0.2 });
        let radius = px(lens.radius().max(0.0));
        lens.capsule()
            .opacity(shown)
            .border_1()
            .border_color(rim)
            .shadow(vec![BoxShadow {
                color: glow,
                offset: point(px(0.0), px(0.0)),
                blur_radius: px(0.0),
                spread_radius: px(4.0),
                inset: true,
            }])
            .when(!dark, |lens| {
                lens.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded(radius)
                        .border(px(0.5))
                        .border_color(hsla(0.0, 0.0, 0.0, 0.08)),
                )
            })
    }

    fn draw(scene: &MainTabBarScene, geometry: Geometry, fades: (f32, f32)) -> AnyElement {
        let width = scene.width;
        let height = MainTabBarMetrics::HEIGHT;
        let dark = scene.dark;
        let bounds = Frame { x: 0.0, y: 0.0, width, height };
        let shown = geometry.shown_lift;
        let lens = geometry.lens;
        let mut canvas = div().relative().w(px(width)).h(px(height));

        let media = scene.uses_glass && scene.media_contrast;
        if media {
            canvas = canvas.child(
                bounds
                    .capsule()
                    .bg(hsla(0.0, 0.0, 0.0, if scene.reduce_transparency { 1.0 } else { 0.45 }))
                    .shadow(vec![BoxShadow {
                        color: hsla(0.0, 0.0, 0.0, 0.28),
                        offset: point(px(0.0), px(2.0)),
                        blur_radius: px(6.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }]),
            );
        }

        if scene.uses_glass {
            canvas = canvas
                .child(bounds.capsule().bg(scene.glass_tint))
                .child(geometry.droplet.capsule().bg(scene.glass_tint));
        } else {
            canvas = canvas
                .child(bounds.capsule().child(div().size_full().liquid_chrome_capsule_with(0.12, true, Some(0.3), true)))
                .child(
                    lens.capsule()
                        .opacity(shown)
                        .bg(if dark { hsla(0.0, 0.0, 0.16, 0.55) } else { hsla(0.0, 0.0, 1.0, 0.45) })
                        .shadow(vec![BoxShadow {
                            color: hsla(0.0, 0.0, 0.0, 0.22),
                            offset: point(px(0.0), px(5.0)),
                            blur_radius: px(12.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }]),
                );
        }

        let resting = if dark { hsla(0.0, 0.0, 1.0, 0.13) } else { hsla(0.0, 0.0, 0.0, 0.07) };
        let lifted = if scene.uses_glass {
            gpui::transparent_black()
        } else if dark {
            hsla(0.0, 0.0, 1.0, 0.07)
        } else {
            hsla(0.0, 0.0, 1.0, 0.32)
        };
        canvas = canvas.child(Self::clip(
            geometry.reach,
            true,
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(lens.capsule().bg(resting).opacity(1.0 - shown))
                .child(lens.capsule().bg(lifted).opacity(shown)),
        ));

        let draws_rows = scene.layout.slot_width() > 0.0 && !scene.slots.is_empty() && !scene.items.is_empty();
        if draws_rows {
            let left = Frame { x: 0.0, y: 0.0, width: lens.x.clamp(0.0, width), height };
            let right_x = lens.max_x().clamp(0.0, width);
            let right = Frame { x: right_x, y: 0.0, width: width - right_x, height };
            let mut base = div().absolute().top_0().left_0().w(px(width)).h(px(height));
            for side in [left, right] {
                if side.width <= 0.0 {
                    continue;
                }
                if let Some(row) = Self::row(scene, false, geometry.shift, Some(fades)) {
                    base = base.child(Self::clip(side, false, row));
                }
            }
            canvas = canvas.child(Self::clip(bounds, true, base));
        }

        if let Some(row) = Self::row(scene, true, geometry.shift, None) {
            let anchor = UnitPoint::new(
                if width > 0.0 { lens.mid_x() / width } else { 0.5 },
                if height > 0.0 { lens.mid_y() / height } else { 0.5 },
            );
            let zoomed = div()
                .absolute()
                .top_0()
                .left_0()
                .w(px(width))
                .h(px(height))
                .child(row)
                .scale_effect_anchored(geometry.zoom, anchor);
            canvas = canvas.child(Self::clip(geometry.reach, true, Self::clip(lens, true, zoomed)));
        }

        canvas = canvas.child(Self::clip(geometry.reach, true, Self::rim(lens, dark, shown)));

        let scheme = if dark { ColorScheme::Dark } else { ColorScheme::Light };
        env_scope::<ColorSchemeKey>(Some(scheme)).child(canvas).into_any_element()
    }
}

impl Render for MainTabBarCanvasView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(scene) = self.scene.clone() else {
            return div().into_any_element();
        };
        if scene.width <= 0.0 || scene.layout.slot_width() <= 0.0 {
            return div().w(px(scene.width.max(0.0))).h(px(MainTabBarMetrics::HEIGHT)).into_any_element();
        }
        let Some(driver) = self.driver.upgrade() else {
            return div().w(px(scene.width)).h(px(MainTabBarMetrics::HEIGHT)).into_any_element();
        };
        let now = current_media_time();
        let (pose, end) = {
            let driver = driver.read(cx);
            (
                Pose {
                    scroll: driver.scroll.value(now),
                    pill_x: driver.pill_x.value(now),
                    lift: driver.lift.value(now),
                    stretch: driver.stretch.value(now),
                },
                driver.settle_time(),
            )
        };
        if end > now + Self::SAMPLE_STEP / 2.0 || self.fade.is_running(now) {
            window.request_animation_frame();
        }
        let fades = self.fade.current(now);
        Self::draw(&scene, Self::geometry(pose, &scene), fades)
    }
}

#[derive(IntoElement)]
pub struct MainTabBarStripRow {
    pub items: Vec<MainTabBarItem>,
    pub slots: Vec<isize>,
    pub positions: Vec<f32>,
    pub layout: TabStripLayout,
    pub size: Size<Pixels>,
    pub tinted: bool,
    pub anchored: HashSet<isize>,
    pub alphas: Vec<f32>,
}

impl MainTabBarStripRow {
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            slots: Vec::new(),
            positions: Vec::new(),
            layout: TabStripLayout::new(0.0, 0, MainTabBarMetrics::VISIBLE_COUNT, 0.0),
            size: size(px(0.0), px(0.0)),
            tinted: false,
            anchored: HashSet::new(),
            alphas: Vec::new(),
        }
    }
}

impl RenderOnce for MainTabBarStripRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let slot_width = self.layout.slot_width();
        let mut row = div().relative().w(self.size.width).h(self.size.height);
        for (index, slot) in self.slots.iter().copied().enumerate() {
            let Some(item) = self.items.get(self.layout.wrapped(slot)).copied() else {
                continue;
            };
            let x = self.positions.get(index).copied().unwrap_or(0.0);
            let alpha = self.alphas.get(index).copied().unwrap_or(1.0);
            let anchor = if self.anchored.contains(&slot) { item.anchor } else { None };
            row = row.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(MainTabBarMetrics::INSET))
                    .w(px(slot_width))
                    .h(px(MainTabBarMetrics::ITEM_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(alpha)
                    .child(MainTabBarItemLabel::new(item, self.tinted))
                    .guide_anchor(anchor),
            );
        }
        row
    }
}
