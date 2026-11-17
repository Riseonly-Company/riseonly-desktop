use std::cell::RefCell;
use std::f32::consts::PI;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{
    AnyElement, App, Background, Bounds, Context, ElementId, Entity, EntityId, FillOptions, Hsla, IntoElement, Path,
    PathBuilder, PathStyle, Pixels, Point, Render, RenderOnce, SharedString, Size, StyleRefinement, TransformationMatrix,
    WeakEntity, Window, canvas, div, point, prelude::*, px,
};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_theme::AppTheme;
use rise_ui::frame_request::is_painted_on_screen;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::animated::{Animated, Lerp};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::shapes::pass_through;
use rise_ui::swiftui::transition::TransitionModifiers;
use rise_widgets::buttons::composer_send_button::ComposerSendButton;

use crate::modules::chat::components::chat::voice::chat_composer_capture_gesture_plate::{
    ChatComposerCaptureGestureAction, ChatComposerCaptureGestureDrag, ChatComposerCaptureGestureEnd,
    ChatComposerCaptureGesturePlate,
};
use crate::modules::chat::components::chat::voice::chat_composer_capture_mode::ChatComposerCaptureMode;

const HOLD_DELAY_SECONDS: TimeInterval = 0.14;
const TAP_MAX_MOVEMENT: f32 = 12.;
const SEND_HOLD_DELAY_SECONDS: TimeInterval = 0.4;

#[derive(IntoElement)]
pub struct ChatComposerSendMicButton {
    pub id: ElementId,
    pub size: f32,
    pub has_text: bool,
    pub is_editing: bool,
    pub capture_mode: ChatComposerCaptureMode,
    pub is_recording: bool,
    pub theme: AppTheme,
    pub on_send: ChatComposerCaptureGestureAction,
    pub on_toggle_capture_mode: ChatComposerCaptureGestureAction,
    pub on_capture_press_began: ChatComposerCaptureGestureAction,
    pub on_capture_press_ended: ChatComposerCaptureGestureAction,
    pub on_hold_began: ChatComposerCaptureGestureAction,
    pub on_hold_drag: ChatComposerCaptureGestureDrag,
    pub on_hold_ended: ChatComposerCaptureGestureEnd,
}

struct ChatComposerSendMicButtonState {
    morph_progress: Animated<f32>,
    is_pressing: bool,
    last_shows_send: Option<bool>,
    last_is_recording: bool,
    highlight: Animated<f32>,
    scale: Animated<f32>,
    last_is_highlighted: bool,
}

impl ChatComposerSendMicButton {
    fn shows_send(&self) -> bool {
        self.has_text || self.is_editing
    }

    fn state_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }
}

impl RenderOnce for ChatComposerSendMicButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let is_recording = self.is_recording;
        let shows_send = self.shows_send() && !is_recording;
        let state = window.use_keyed_state(self.state_id("send-mic-state"), cx, |_, _| {
            ChatComposerSendMicButtonState {
                morph_progress: Animated::new(0.),
                is_pressing: false,
                last_shows_send: None,
                last_is_recording: is_recording,
                highlight: Animated::new(0.),
                scale: Animated::new(1.),
                last_is_highlighted: false,
            }
        });

        state.update(cx, |state, cx| {
            let target = if shows_send { 1. } else { 0. };
            match state.last_shows_send {
                None => state.morph_progress.jump(target),
                Some(previous) if previous != shows_send => {
                    state.morph_progress.set_in(target, spring(0.34, 0.78), now, cx);
                }
                Some(_) => {}
            }
            state.last_shows_send = Some(shows_send);
            if state.last_is_recording && !is_recording {
                state.is_pressing = false;
            }
            state.last_is_recording = is_recording;
            let is_highlighted = state.is_pressing || is_recording;
            if is_highlighted != state.last_is_highlighted {
                state.last_is_highlighted = is_highlighted;
                let highlight = if is_highlighted { 1. } else { 0. };
                let scale = if is_highlighted { 1.22 } else { 1. };
                state.highlight.set_in(highlight, spring(0.22, 0.7), now, cx);
                state.scale.set_in(scale, spring(0.25, 0.65), now, cx);
            }
        });

        let (morph_progress, is_pressing, highlight, scale) = {
            let state = state.read(cx);
            (
                state.morph_progress.frame_value(now, window),
                state.is_pressing,
                state.highlight.frame_value(now, window),
                state.scale.frame_value(now, window),
            )
        };

        let size = self.size;
        let send_hit_testing = shows_send && morph_progress > 0.92;
        let on_send = self.on_send.clone();
        let send_layer = div()
            .absolute()
            .left_0()
            .top(px((1. - morph_progress) * 10.))
            .size(px(size))
            .opacity(morph_progress)
            .child(
                ComposerSendButton::new(self.state_id("send"), move |window, cx| {
                    if send_hit_testing {
                        on_send(window, cx);
                    }
                })
                .size(size)
                .is_editing(self.is_editing)
                .is_enabled(self.has_text || !self.is_editing),
            )
            .scale_effect(0.62 + 0.38 * morph_progress);

        let capture_layer = div()
            .absolute()
            .left_0()
            .top(px(-morph_progress * 10.))
            .size(px(size))
            .opacity(1. - morph_progress)
            .child(self.capture_layer(is_pressing, highlight, scale, shows_send))
            .scale_effect(0.62 + 0.38 * (1. - morph_progress));

        let plate = self.gesture_plate(&state);
        div()
            .relative()
            .size(px(size))
            .child(send_layer)
            .child(capture_layer)
            .child(div().absolute().inset_0().child(plate))
    }
}

impl ChatComposerSendMicButton {
    /// the button drawn by its own cached view, see the send mic button host
    pub fn hosted(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let size = self.size;
        let host = window.with_global_id(self.state_id("send-mic-host"), |global_id, window| {
            window.with_element_state(global_id, |held: Option<Entity<ChatComposerSendMicButtonHost>>, _| {
                let host = held.unwrap_or_else(|| cx.new(|_| ChatComposerSendMicButtonHost { button: None }));
                (host.clone(), host)
            })
        });
        host.update(cx, |host, cx| host.configure(self, cx));
        div()
            .size(px(size))
            .flex_none()
            .child(host.cached(StyleRefinement::default().size_full()))
            .into_any_element()
    }

    fn same_look(&self, other: &Self) -> bool {
        self.id == other.id
            && self.size == other.size
            && self.has_text == other.has_text
            && self.is_editing == other.is_editing
            && self.capture_mode == other.capture_mode
            && self.is_recording == other.is_recording
            && self.theme.primary._100 == other.theme.primary._100
            && self.theme.text.primary == other.theme.text.primary
    }
}

/// the button as its own cached view
/// the send to mic morph frames rerendered the whole chat screen around it
/// so the host renders only when the button shows something new
pub struct ChatComposerSendMicButtonHost {
    button: Option<ChatComposerSendMicButton>,
}

impl ChatComposerSendMicButtonHost {
    fn configure(&mut self, button: ChatComposerSendMicButton, cx: &mut Context<Self>) {
        let changed = self.button.as_ref().is_none_or(|held| !held.same_look(&button));
        self.button = Some(button);
        if changed {
            cx.notify();
        }
    }

    fn held_action(
        host: &WeakEntity<Self>,
        pick: fn(&ChatComposerSendMicButton) -> &ChatComposerCaptureGestureAction,
    ) -> ChatComposerCaptureGestureAction {
        let host = host.clone();
        Rc::new(move |window: &mut Window, cx: &mut App| {
            let action = host
                .upgrade()
                .and_then(|host| host.read(cx).button.as_ref().map(|button| pick(button).clone()));
            if let Some(action) = action {
                action(window, cx);
            }
        })
    }

    fn held_drag(host: &WeakEntity<Self>) -> ChatComposerCaptureGestureDrag {
        let host = host.clone();
        Rc::new(move |translation: Size<Pixels>, window: &mut Window, cx: &mut App| {
            let action = host
                .upgrade()
                .and_then(|host| host.read(cx).button.as_ref().map(|button| button.on_hold_drag.clone()));
            if let Some(action) = action {
                action(translation, window, cx);
            }
        })
    }

    fn held_end(host: &WeakEntity<Self>) -> ChatComposerCaptureGestureEnd {
        let host = host.clone();
        Rc::new(move |translation: Size<Pixels>, cancelled: bool, window: &mut Window, cx: &mut App| {
            let action = host
                .upgrade()
                .and_then(|host| host.read(cx).button.as_ref().map(|button| button.on_hold_ended.clone()));
            if let Some(action) = action {
                action(translation, cancelled, window, cx);
            }
        })
    }
}

impl Render for ChatComposerSendMicButtonHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(held) = self.button.as_ref() else {
            return div().into_any_element();
        };
        let host = cx.weak_entity();
        ChatComposerSendMicButton {
            id: held.id.clone(),
            size: held.size,
            has_text: held.has_text,
            is_editing: held.is_editing,
            capture_mode: held.capture_mode,
            is_recording: held.is_recording,
            theme: held.theme.clone(),
            on_send: Self::held_action(&host, |button| &button.on_send),
            on_toggle_capture_mode: Self::held_action(&host, |button| &button.on_toggle_capture_mode),
            on_capture_press_began: Self::held_action(&host, |button| &button.on_capture_press_began),
            on_capture_press_ended: Self::held_action(&host, |button| &button.on_capture_press_ended),
            on_hold_began: Self::held_action(&host, |button| &button.on_hold_began),
            on_hold_drag: Self::held_drag(&host),
            on_hold_ended: Self::held_end(&host),
        }
        .into_any_element()
    }
}

impl ChatComposerSendMicButton {
    fn gesture_plate(&self, state: &gpui::Entity<ChatComposerSendMicButtonState>) -> ChatComposerCaptureGesturePlate {
        let sends_on_tap = self.has_text && !self.is_editing;
        let began_state = state.clone();
        let on_capture_press_began = self.on_capture_press_began.clone();
        let ended_state = state.clone();
        let on_capture_press_ended = self.on_capture_press_ended.clone();
        let hold_ended_state = state.clone();
        let on_hold_ended = self.on_hold_ended.clone();

        ChatComposerCaptureGesturePlate {
            id: self.state_id("capture-plate"),
            is_enabled: !self.is_editing,
            hold_delay: if sends_on_tap { SEND_HOLD_DELAY_SECONDS } else { HOLD_DELAY_SECONDS },
            tap_max_movement: TAP_MAX_MOVEMENT,
            on_press_began: Rc::new(move |window: &mut Window, cx: &mut App| {
                began_state.update(cx, |state, cx| {
                    state.is_pressing = true;
                    cx.notify();
                });
                on_capture_press_began(window, cx);
            }),
            on_press_ended: Rc::new(move |window: &mut Window, cx: &mut App| {
                ended_state.update(cx, |state, cx| {
                    state.is_pressing = false;
                    cx.notify();
                });
                on_capture_press_ended(window, cx);
            }),
            on_hold_began: self.on_hold_began.clone(),
            on_hold_drag: self.on_hold_drag.clone(),
            on_hold_ended: Rc::new(move |translation: Size<Pixels>, cancelled: bool, window: &mut Window, cx: &mut App| {
                hold_ended_state.update(cx, |state, cx| {
                    state.is_pressing = false;
                    cx.notify();
                });
                on_hold_ended(translation, cancelled, window, cx);
            }),
            on_tap: if sends_on_tap { self.on_send.clone() } else { self.on_toggle_capture_mode.clone() },
        }
    }

    fn capture_layer(&self, is_pressing: bool, highlight: f32, scale: f32, shows_send: bool) -> impl IntoElement {
        let size = self.size;
        let accent = self.theme.primary._100;
        let is_video_note = self.capture_mode == ChatComposerCaptureMode::VideoNote;
        let symbol = if is_video_note { "video.fill" } else { "mic.fill" };
        let icon_side = if is_video_note { 18. } else { 20. };
        let fill = accent.opacity(highlight);
        let icon_color = self.theme.text.primary.lerp(&gpui::white(), highlight);

        let button = div()
            .size(px(size))
            .child(
                div()
                    .relative()
                    .size(px(size))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().absolute().inset_0().rounded_full().bg(fill))
                    .children(IconUi::sized(symbol, px(icon_side), icon_color, true))
                    .liquid_chrome_circle(None),
            )
            .scale_effect(scale);

        let rings = (self.is_recording || is_pressing).then(|| MicRippleRings {
            id: self.state_id("ripple-rings"),
            size,
            accent,
        });

        let layer = div()
            .relative()
            .size(px(size))
            .children(rings)
            .child(button);
        if shows_send {
            pass_through().relative().size(px(size)).child(layer).into_any_element()
        } else {
            layer.into_any_element()
        }
    }
}

#[derive(IntoElement)]
pub struct MicRippleRings {
    pub id: ElementId,
    pub size: f32,
    pub accent: Hsla,
}

const RIPPLE_PERIOD_SECONDS: f64 = 1.4;
const RIPPLE_COUNT: u64 = 3;
const RIPPLE_STROKE: f32 = 2.;
const RIPPLE_CIRCLE_STEPS: usize = 48;

static NEXT_RIPPLE_SWAP_KEY: AtomicU64 = AtomicU64::new(5 << 61);

/// base swap key of the newest paint of one ripple set
/// element state so nothing observes it and ripples never notify the chat screen
struct MicRippleSwapState {
    base_key: u64,
}

impl RenderOnce for MicRippleRings {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let swap_state = window.with_global_id(self.id.clone(), |global_id, window| {
            window.with_element_state::<Rc<RefCell<MicRippleSwapState>>, _>(global_id, |state, _| {
                let state = state.unwrap_or_else(|| Rc::new(RefCell::new(MicRippleSwapState { base_key: 0 })));
                (state.clone(), state)
            })
        });
        let size = self.size;
        let accent = self.accent;
        pass_through().absolute().inset_0().size(px(size)).child(
            canvas(|_, _: &mut Window, _: &mut App| (), move |bounds, (), window, _| {
                paint_mic_ripples(&swap_state, size, accent, bounds, window)
            })
            .absolute()
            .inset_0()
            .size(px(size)),
        )
    }
}

/// phase of ring index at now, 0 at the button edge and 1 at the widest and faded out
fn ripple_local(index: u64) -> f32 {
    let t = (RiseClock::monotonic().as_secs_f64() % RIPPLE_PERIOD_SECONDS) / RIPPLE_PERIOD_SECONDS;
    ((t + index as f64 * 0.28) % 1.) as f32
}

/// one ring as an annulus centred in a box of side size from origin
/// ring is one point wider than the button on each side and scaled up by its phase
fn ripple_path(origin: Point<Pixels>, size: f32, local: f32) -> Option<Path<Pixels>> {
    let scale = 1. + local * 1.35;
    let outer = (size + 2.) * 0.5 * scale;
    let inner = ((size + 2.) * 0.5 - RIPPLE_STROKE) * scale;
    if outer <= 0. || inner <= 0. {
        return None;
    }
    let center = point(origin.x + px(size * 0.5), origin.y + px(size * 0.5));
    let circle = |radius: f32| {
        (0..RIPPLE_CIRCLE_STEPS)
            .map(|step| {
                let angle = 2. * PI * step as f32 / RIPPLE_CIRCLE_STEPS as f32;
                point(center.x + px(radius * angle.cos()), center.y + px(radius * angle.sin()))
            })
            .collect::<Vec<_>>()
    };
    let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::even_odd()));
    builder.add_polygon(&circle(outer), true);
    builder.add_polygon(&circle(inner), true);
    builder.build().ok()
}

fn ripple_color(accent: Hsla, local: f32) -> Hsla {
    accent.opacity(0.45 * (1. - local))
}

fn paint_mic_ripples(
    swap_state: &Rc<RefCell<MicRippleSwapState>>,
    size: f32,
    accent: Hsla,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let base_key = NEXT_RIPPLE_SWAP_KEY.fetch_add(RIPPLE_COUNT, Ordering::Relaxed);
    swap_state.borrow_mut().base_key = base_key;
    for index in 0..RIPPLE_COUNT {
        let local = ripple_local(index);
        if let Some(path) = ripple_path(bounds.origin, size, local) {
            window.paint_swappable_path(path, ripple_color(accent, local), base_key + index, bounds.origin);
        }
    }
    if !is_painted_on_screen(bounds, window) {
        return;
    }
    if window.transformation() != TransformationMatrix::unit() {
        window.request_animation_frame();
        return;
    }
    MicRippleSwaps { swap_state: swap_state.clone(), size, accent, base_key, view: window.current_view() }.schedule(window);
}

struct MicRippleSwaps {
    swap_state: Rc<RefCell<MicRippleSwapState>>,
    size: f32,
    accent: Hsla,
    base_key: u64,
    view: EntityId,
}

impl MicRippleSwaps {
    fn schedule(self, window: &Window) {
        window.on_next_frame(move |window, cx| self.tick(window, cx));
    }

    fn tick(self, window: &mut Window, cx: &mut App) {
        if self.swap_state.borrow().base_key != self.base_key {
            return;
        }
        let origin = point(px(0.), px(0.));
        let swaps: Option<Vec<(u64, Path<Pixels>, Background)>> = (0..RIPPLE_COUNT)
            .map(|index| {
                let local = ripple_local(index);
                ripple_path(origin, self.size, local)
                    .map(|path| (self.base_key + index, path, ripple_color(self.accent, local).into()))
            })
            .collect();
        let Some(swaps) = swaps else {
            return;
        };
        if window.swap_paths(swaps) {
            self.schedule(window);
            return;
        }
        cx.notify(self.view);
    }
}
