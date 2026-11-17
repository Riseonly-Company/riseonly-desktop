use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use gpui::{
    AnyElement, App, AsyncWindowContext, Bounds, Context, Element, ElementId, Entity, GLYPH_SHIFT_KEY_BITS,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Render, RenderOnce, SharedString, StyleRefinement, WeakEntity,
    Window, div, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::pressable_ui::PressableUi;
use rise_ui::swiftui::animated::MotionTime;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};

const START_DELAY: f64 = 0.9;
const END_HOLD: f64 = 1.0;
const SLIDE_SPEED: f64 = 20.0;
const MIN_LEG_DURATION: f64 = 3.5;
const SLIDE_FRAME: Duration = Duration::from_millis(16);
const PROBE_INTERVAL: Duration = Duration::from_millis(500);

static NEXT_SLIDE_KEY: AtomicU32 = AtomicU32::new(1);

fn next_slide_key() -> u32 {
    NEXT_SLIDE_KEY.fetch_add(1, Ordering::Relaxed) | GLYPH_SHIFT_KEY_BITS
}

struct MusicMarqueeTitleState {
    text_width: f32,
    container_width: f32,
    width: Option<f32>,
    started_at: Option<MotionTime>,
    is_active: bool,
    held_at: Option<MotionTime>,
    reduce_motion: bool,
    newest_key: u32,
    shown_offset: f32,
    is_copy_menu_presented: bool,
}

impl MusicMarqueeTitleState {
    fn overflow(&self) -> f32 {
        (self.text_width - self.effective_container_width()).max(0.0)
    }

    fn effective_container_width(&self) -> f32 {
        match self.width {
            Some(width) if width > 0.0 => width,
            _ => self.container_width,
        }
    }

    fn set_text_width(&mut self, text_width: f32, cx: &mut Context<Self>) {
        if self.text_width != text_width {
            self.text_width = text_width;
            self.restart_marquee(cx);
        }
    }

    fn set_container_width(&mut self, container_width: f32, cx: &mut Context<Self>) {
        if self.container_width != container_width {
            self.container_width = container_width;
            self.restart_marquee(cx);
        }
    }

    fn stop_marquee(&mut self) {
        self.started_at = None;
        self.held_at = None;
    }

    /// hold freezes the time the offset reads, resume shifts started_at by the held span
    /// so the leg in progress finishes where it stands
    fn set_active(&mut self, is_active: bool, cx: &mut Context<Self>) {
        if self.is_active == is_active {
            return;
        }
        self.is_active = is_active;
        let now = RiseClock::monotonic();
        if !is_active {
            if self.started_at.is_some() && self.held_at.is_none() {
                self.held_at = Some(now);
                cx.notify();
            }
            return;
        }
        match (self.started_at, self.held_at.take()) {
            (Some(started_at), Some(held_at)) => {
                self.started_at = Some(started_at + now.saturating_sub(held_at));
                cx.notify();
            }
            (None, _) if self.overflow() > 1.0 => self.restart_marquee(cx),
            _ => {}
        }
    }

    fn slides(&self) -> bool {
        self.started_at.is_some() && self.held_at.is_none() && self.overflow() > 1.0
    }

    fn restart_marquee(&mut self, cx: &mut Context<Self>) {
        self.stop_marquee();
        cx.notify();
        if self.overflow() <= 1.0 {
            return;
        }
        self.reduce_motion = cx.reduce_motion();
        let now = RiseClock::monotonic();
        self.started_at = Some(now);
        self.held_at = (!self.is_active).then_some(now);
    }

    /// marquee leg as a pure function of time
    /// linear slide every duration + 1 seconds after a 0.9 s wait
    fn leg(&self, now: MotionTime) -> Option<(f64, f64, f64, bool)> {
        let started_at = self.started_at?;
        let now = self.held_at.unwrap_or(now);
        let travel = f64::from(self.overflow());
        if travel <= 1.0 {
            return None;
        }
        let duration = (travel / SLIDE_SPEED).max(MIN_LEG_DURATION);
        let period = duration + END_HOLD;
        let elapsed = now.saturating_sub(started_at).as_secs_f64() - START_DELAY;
        if elapsed < 0.0 {
            return Some((travel, duration, elapsed, true));
        }
        let index = (elapsed / period).floor();
        let within = elapsed - index * period;
        Some((travel, duration, within, index % 2.0 == 0.0))
    }

    fn offset_at(&self, now: MotionTime) -> f32 {
        let Some((travel, duration, within, outward)) = self.leg(now) else {
            return 0.0;
        };
        if within < 0.0 {
            return 0.0;
        }
        let progress = if self.reduce_motion { 1.0 } else { (within / duration).min(1.0) };
        let offset = if outward { -travel * progress } else { -travel * (1.0 - progress) };
        offset as f32
    }

    fn still_for(&self, now: MotionTime) -> Option<Duration> {
        if self.held_at.is_some() {
            return None;
        }
        let (_, duration, within, _) = self.leg(now)?;
        if within < 0.0 {
            return Some(Duration::from_secs_f64(-within));
        }
        if !self.reduce_motion && within < duration {
            return Some(Duration::ZERO);
        }
        Some(Duration::from_secs_f64((duration + END_HOLD - within).max(0.0)))
    }
}

fn device_rounded(offset: f32, scale: f32) -> f32 {
    if scale > 0.0 { (offset * scale).round() / scale } else { offset }
}

struct MusicMarqueeTitleSlideText {
    child: AnyElement,
    state: Entity<MusicMarqueeTitleState>,
    offset: f32,
    slides: bool,
}

impl IntoElement for MusicMarqueeTitleSlideText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MusicMarqueeTitleSlideText {
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
        if !self.slides {
            self.child.paint(window, cx);
            self.state.update(cx, |state, _| state.newest_key = 0);
            return;
        }
        let key = next_slide_key();
        window.with_glyph_swap_key(key, |window| self.child.paint(window, cx));
        let offset = self.offset;
        self.state.update(cx, |state, _| {
            state.newest_key = key;
            state.shown_offset = offset;
        });
        MusicMarqueeTitleSlide { key, state: self.state.downgrade() }.schedule(SLIDE_FRAME, window, cx);
    }
}

struct MusicMarqueeTitleSlide {
    key: u32,
    state: WeakEntity<MusicMarqueeTitleState>,
}

impl MusicMarqueeTitleSlide {
    fn schedule(self, delay: Duration, window: &Window, cx: &App) {
        window
            .spawn(cx, async move |cx: &mut AsyncWindowContext| {
                cx.background_executor().timer(delay).await;
                let _ = cx.update(move |window, _| window.on_next_frame(move |window, cx| self.tick(window, cx)));
            })
            .detach();
    }

    fn tick(self, window: &mut Window, cx: &mut App) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let now = RiseClock::monotonic();
        let (newest_key, target, shown, still) = {
            let state = state.read(cx);
            (state.newest_key, state.offset_at(now), state.shown_offset, state.still_for(now))
        };
        if newest_key != self.key {
            return;
        }
        let Some(still) = still else {
            return;
        };
        let target = device_rounded(target, window.scale_factor());
        let Some(shows) = window.shift_glyphs(self.key, px(target - shown)) else {
            self.schedule(PROBE_INTERVAL, window, cx);
            return;
        };
        state.update(cx, |state, _| state.shown_offset = target);
        let delay = if !shows {
            PROBE_INTERVAL.max(still)
        } else if still > SLIDE_FRAME {
            still
        } else {
            SLIDE_FRAME
        };
        self.schedule(delay, window, cx);
    }
}

#[derive(IntoElement, Clone, PartialEq)]
pub struct MusicMarqueeTitle {
    id: ElementId,
    text: SharedString,
    font: Font,
    foreground: Hsla,
    line_height: f32,
    shows_copy_menu: bool,
    width: Option<f32>,
    hugs_text: bool,
    is_active: bool,
}

impl MusicMarqueeTitle {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            font: Font::system_weight(22.0, gpui::FontWeight::BOLD),
            foreground: white(),
            line_height: 28.0,
            shows_copy_menu: true,
            width: None,
            hugs_text: false,
            is_active: true,
        }
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    pub fn foreground(mut self, foreground: Hsla) -> Self {
        self.foreground = foreground;
        self
    }

    pub fn line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self
    }

    pub fn shows_copy_menu(mut self, shows_copy_menu: bool) -> Self {
        self.shows_copy_menu = shows_copy_menu;
        self
    }

    pub fn width(mut self, width: impl Into<Option<f32>>) -> Self {
        self.width = width.into();
        self
    }

    pub fn hugs_text(mut self, hugs_text: bool) -> Self {
        self.hugs_text = hugs_text;
        self
    }

    /// false holds the title where it is because its track is paused
    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }
}

/// title in its own cached view
/// measure, restart and hold of the slide notify the nearest view
/// that was the host page or the root
struct MusicMarqueeTitleView {
    title: MusicMarqueeTitle,
}

impl Render for MusicMarqueeTitleView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.title.clone().body(window, cx)
    }
}

impl RenderOnce for MusicMarqueeTitle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.hugs_text {
            return self.body(window, cx);
        }
        let style = match self.width.filter(|width| *width > 0.0) {
            Some(fixed_width) => StyleRefinement::default().w(px(fixed_width)),
            None => StyleRefinement::default().w_full().min_w_0(),
        }
        .h(px(self.line_height));
        let view = window.with_global_id(self.id.clone(), |global_id, window| {
            window.with_element_state(global_id, |existing: Option<Entity<MusicMarqueeTitleView>>, _| {
                let view = existing.unwrap_or_else(|| cx.new(|_| MusicMarqueeTitleView { title: self.clone() }));
                (view.clone(), view)
            })
        });
        view.update(cx, |view, cx| {
            if view.title != self {
                view.title = self;
                cx.notify();
            }
        });
        view.cached(style).into_any_element()
    }
}

impl MusicMarqueeTitle {
    fn body(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let id = self.id;
        let text = self.text;
        let font = self.font;
        let line_height = self.line_height;
        let fixed_width = self.width.filter(|width| *width > 0.0);
        let width = self.width;
        let is_active = self.is_active;

        let state = window.use_keyed_state(id.clone(), cx, |_, _| MusicMarqueeTitleState {
            text_width: 0.0,
            container_width: 0.0,
            width,
            started_at: None,
            is_active,
            held_at: None,
            reduce_motion: false,
            newest_key: 0,
            shown_offset: 0.0,
            is_copy_menu_presented: false,
        });

        let changed_state = state.clone();
        on_change(SharedString::from(format!("{id}-text")), text.clone(), false, window, cx, move |_, _, _, cx| {
            changed_state.update(cx, |state, cx| state.restart_marquee(cx));
        });
        let changed_state = state.clone();
        on_change(
            SharedString::from(format!("{id}-width")),
            width.map(f32::to_bits),
            false,
            window,
            cx,
            move |_, _, _, cx| {
                changed_state.update(cx, |state, cx| {
                    state.width = width;
                    state.restart_marquee(cx);
                });
            },
        );

        let changed_state = state.clone();
        on_change(SharedString::from(format!("{id}-active")), is_active, false, window, cx, move |_, _, _, cx| {
            changed_state.update(cx, |state, cx| state.set_active(is_active, cx));
        });

        let (shown_offset, overflow, text_width, slides) = {
            let state = state.read(cx);
            (
                device_rounded(state.offset_at(RiseClock::monotonic()), window.scale_factor()),
                state.overflow(),
                state.text_width,
                state.slides(),
            )
        };

        let text_state = state.clone();
        let measured_text = div()
            .absolute()
            .left_0()
            .top_0()
            .opacity(0.0)
            .flex_none()
            .whitespace_nowrap()
            .swiftui_font(font)
            .child(text.clone())
            .child(measure(SharedString::from(format!("{id}-text-width")), move |bounds, _, cx| {
                text_state.update(cx, |state, cx| state.set_text_width(f32::from(bounds.size.width), cx));
            }));

        let core = div()
            .relative()
            .flex()
            .items_center()
            .overflow_hidden()
            .w_full()
            .h(px(line_height))
            .child(MusicMarqueeTitleSlideText {
                child: div()
                    .relative()
                    .flex_none()
                    .left(px(if overflow > 1.0 { shown_offset } else { 0.0 }))
                    .whitespace_nowrap()
                    .swiftui_font(font)
                    .text_color(self.foreground)
                    .child(text.clone())
                    .into_any_element(),
                state: state.clone(),
                offset: if overflow > 1.0 { shown_offset } else { 0.0 },
                slides,
            })
            .child(measured_text)
            .when(fixed_width.is_none(), |core| {
                let container_state = state.clone();
                core.child(measure(SharedString::from(format!("{id}-container-width")), move |bounds, _, cx| {
                    container_state.update(cx, |state, cx| {
                        state.set_container_width(f32::from(bounds.size.width), cx);
                    });
                }))
            })
            .into_any_element();

        let disappear_state = state.clone();
        let core = core.on_disappear(SharedString::from(format!("{id}-lifecycle")), move |_, cx| {
            disappear_state.update(cx, |state, cx| {
                state.stop_marquee();
                cx.notify();
            });
        });

        let content = if self.shows_copy_menu {
            let is_copy_menu_presented = Binding::entity(
                state.clone(),
                |state: &MusicMarqueeTitleState| state.is_copy_menu_presented,
                |state: &mut MusicMarqueeTitleState, value: bool, cx| {
                    state.is_copy_menu_presented = value;
                    cx.notify();
                },
            );
            let opener = state;
            let actions: HoldContextMenuPresentationActions = Rc::new(|_: &App| {
                vec![HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc")]
            });
            let on_action: HoldContextMenuPresentationOnAction =
                Rc::new(move |_: String, _: &mut Window, cx: &mut App| UIPasteboard::general().set_string(&text, cx));
            let source: HoldContextMenuTapSource = Rc::new(|_: &mut Window, _: &mut App| div().into_any_element());
            PressableUi::new(SharedString::from(format!("{id}-copy-hold")))
                .tap_scale(1.0)
                .long_press_scale(1.0)
                .long_press_duration(HoldContextMenuMetrics::CONFIRM_DURATION as f32)
                .on_long_press(move |_, cx| {
                    let _ = ImpactFeedbackGenerator::new(ImpactStyle::Medium).impact_occurred();
                    opener.update(cx, |state, cx| {
                        state.is_copy_menu_presented = true;
                        cx.notify();
                    });
                })
                .child(core)
                .tap_context_menu(
                    SharedString::from(format!("{id}-menu")),
                    is_copy_menu_presented,
                    HoldContextMenuAlignment::Leading,
                    false,
                    false,
                    false,
                    None,
                    actions,
                    on_action,
                    source,
                )
                .into_any_element()
        } else {
            core.into_any_element()
        };

        let frame = div();
        match fixed_width {
            Some(fixed_width) => frame.w(px(fixed_width)),
            None => {
                let frame = frame.w_full().min_w_0();
                if self.hugs_text && text_width > 0.0 { frame.max_w(px(text_width)) } else { frame }
            }
        }
        .child(content)
        .into_any_element()
    }
}
