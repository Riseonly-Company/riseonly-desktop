use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Corners, ElementId, Entity, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, SharedString, Task, Window,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_theme::alpha;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::TransitionModifiers;

use crate::modules::chat::shared::config::message_sizes::MessageAccessoryStackGeometry;
use crate::modules::chat::stores::message::message_actions::message_types::MessageBotKeyboardButton;

pub enum BotInlineKeyboardActivationPolicy {}

impl BotInlineKeyboardActivationPolicy {
    pub const MINIMUM_PRESS_DURATION: f64 = 0.28;
    pub const MAXIMUM_MOVEMENT: f32 = 6.0;
}

/// shared with the list engine height estimate, rows are ROW_HEIGHT tall and gaps are SPACING
pub enum BotInlineKeyboardLayout {}

impl BotInlineKeyboardLayout {
    pub const ROW_HEIGHT: f32 = 42.0;
    pub const SPACING: f32 = MessageAccessoryStackGeometry::INLINE_KEYBOARD_SPACING;

    pub fn height(row_count: usize) -> f32 {
        if row_count == 0 {
            return 0.0;
        }
        row_count as f32 * Self::ROW_HEIGHT + (row_count - 1) as f32 * Self::SPACING
    }
}

pub type BotMessageInlineKeyboardPress = Rc<dyn Fn(&MessageBotKeyboardButton, &str, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct BotMessageInlineKeyboard {
    pub id: ElementId,
    pub rows: Vec<Vec<MessageBotKeyboardButton>>,
    pub is_own: bool,
    pub pending_key: Option<String>,
    pub on_press: BotMessageInlineKeyboardPress,
}

impl BotMessageInlineKeyboard {
    pub fn new(
        id: impl Into<ElementId>,
        rows: Vec<Vec<MessageBotKeyboardButton>>,
        is_own: bool,
        on_press: impl Fn(&MessageBotKeyboardButton, &str, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), rows, is_own, pending_key: None, on_press: Rc::new(on_press) }
    }

    pub fn pending_key(mut self, pending_key: Option<String>) -> Self {
        self.pending_key = pending_key;
        self
    }

    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn button_key(button: &MessageBotKeyboardButton, row: usize, column: usize) -> String {
        let action = button
            .callback_data
            .as_deref()
            .or(button.web_app_url.as_deref())
            .or(button.url.as_deref())
            .or(button.switch_inline_query.as_deref())
            .or(button.switch_inline_query_current_chat.as_deref())
            .unwrap_or(&button.text);
        format!("{row}:{column}:{action}")
    }

    fn action_icon(button: &MessageBotKeyboardButton) -> Option<&'static str> {
        if button.web_app_url.is_some() {
            return Some("rectangle.on.rectangle.angled");
        }
        if button.url.is_some() {
            return Some("arrow.up.right");
        }
        if button.switch_inline_query.is_some() || button.switch_inline_query_current_chat.is_some() {
            return Some("text.cursor");
        }
        if button.is_pay {
            return Some("creditcard.fill");
        }
        if button.is_game {
            return Some("gamecontroller.fill");
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn button_view(
        &self,
        button: &MessageBotKeyboardButton,
        key: String,
        row: usize,
        column: usize,
        column_count: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let foreground_color = theme.primary._100;
        let button_background = alpha(theme.bg._300, 0.88);
        let stroke_color = alpha(theme.border._200, 0.35);
        let is_last_row = row + 1 == self.rows.len();
        let corners = Corners {
            top_left: 8.0,
            top_right: 8.0,
            bottom_left: if is_last_row && column == 0 { 16.0 } else { 8.0 },
            bottom_right: if is_last_row && column + 1 == column_count { 16.0 } else { 8.0 },
        };
        let is_pending = self.pending_key.is_some();
        let is_this_pending = self.pending_key.as_deref() == Some(key.as_str());

        let press_id = self.child_id(format!("press-{row}-{column}"));
        let state = window.use_keyed_state(press_id.clone(), cx, |_, _| TelegramInlineKeyboardPressState::new());
        let now = RiseClock::monotonic();
        let progress = {
            let press = state.read(cx);
            if press.highlight.is_running(now) {
                window.request_animation_frame();
            }
            press.highlight.value(now)
        };

        let label = hstack(5.)
            .w_full()
            .h(px(BotInlineKeyboardLayout::ROW_HEIGHT))
            .justify_center()
            .padding_edges(EdgeSet::HORIZONTAL, 8.)
            .when(is_this_pending, |label| {
                label.child(progress_view().control_size(ControlSize::Small).tint(foreground_color))
            })
            .child(
                div().min_w_0().flex_shrink(1.0).child(
                    MainText::new(button.text.clone())
                        .px(16.)
                        .font_weight(FontWeight::MEDIUM)
                        .color(foreground_color)
                        .number_of_lines(1),
                ),
            )
            .when_some(Self::action_icon(button).filter(|_| !is_this_pending), |label, icon| {
                label.children(IconUi::sized(icon, px(10.), alpha(foreground_color, 0.82), false))
            })
            .opacity(TelegramInlineKeyboardButtonStyle::opacity(progress))
            .scale_effect(TelegramInlineKeyboardButtonStyle::scale(progress));

        let on_press = self.on_press.clone();
        let pressed = button.clone();
        let pressed_key = key.clone();
        let surface = plain(self.child_id(format!("button-{row}-{column}")))
            .disabled(is_pending)
            .on_press(move |window, cx| {
                if is_pending {
                    return;
                }
                on_press(&pressed, &pressed_key, window, cx);
            })
            .w_full()
            .bg(button_background)
            .rounded_tl(px(corners.top_left))
            .rounded_tr(px(corners.top_right))
            .rounded_bl(px(corners.bottom_left))
            .rounded_br(px(corners.bottom_right))
            .overflow_hidden()
            .child(label);

        let down_state = state.clone();
        let move_state = state.clone();
        let up_state = state.clone();
        let up_out_state = state.clone();
        let hover_state = state;
        div()
            .id(press_id)
            .relative()
            .w_full()
            .min_w_0()
            .opacity(if is_pending { 0.58 } else { 1.0 })
            .on_hover(move |hovered, _, cx| {
                let reduce_motion = reduce_motion(cx);
                hover_state.update(cx, |press, cx| {
                    press.set_hovered(*hovered, RiseClock::monotonic(), reduce_motion);
                    cx.notify();
                });
            })
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                let reduce_motion = reduce_motion(cx);
                down_state.update(cx, |press, cx| {
                    press.press_began(event.position, RiseClock::monotonic(), reduce_motion);
                    cx.notify();
                });
                if !is_pending {
                    start_hold(&down_state, window, cx);
                }
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) && move_state.read(cx).moved_too_far(event.position) {
                    move_state.update(cx, |press, _| press.cancel_hold());
                }
            })
            .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
                let reduce_motion = reduce_motion(cx);
                up_state.update(cx, |press, cx| {
                    press.press_ended(RiseClock::monotonic(), reduce_motion);
                    cx.notify();
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
                let reduce_motion = reduce_motion(cx);
                up_out_state.update(cx, |press, cx| {
                    press.press_ended(RiseClock::monotonic(), reduce_motion);
                    cx.notify();
                });
            })
            .child(surface)
            .child(uneven_centred_stroke(stroke_color, 0.5, corners))
    }
}

impl RenderOnce for BotMessageInlineKeyboard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut stack = vstack(BotInlineKeyboardLayout::SPACING).w_full();
        for (row_index, row) in self.rows.iter().enumerate() {
            let mut line = TelegramInlineKeyboardRow::layout(BotInlineKeyboardLayout::SPACING, row.len());
            for (column_index, button) in row.iter().enumerate() {
                let key = Self::button_key(button, row_index, column_index);
                line = line.child(self.button_view(button, key, row_index, column_index, row.len(), window, cx));
            }
            stack = stack.child(line);
        }
        stack
    }
}

/// inline keyboard button style, label dips and fades while pressed
/// progress goes 0 at rest to 1 fully pressed
enum TelegramInlineKeyboardButtonStyle {}

impl TelegramInlineKeyboardButtonStyle {
    fn scale(progress: f32) -> f32 {
        1.0 - (1.0 - 0.975) * progress
    }

    fn opacity(progress: f32) -> f32 {
        1.0 - (1.0 - 0.84) * progress
    }

    /// press animation time, 0.08 pressing and 0.16 releasing
    fn duration(is_pressed: bool) -> f64 {
        if is_pressed { 0.08 } else { 0.16 }
    }
}

/// pressed state with its animation, plus the long press that runs next to the tap
struct TelegramInlineKeyboardPressState {
    highlight: Animated<f32>,
    is_pressed: bool,
    is_hovered: bool,
    press_origin: Option<Point<Pixels>>,
    hold_generation: u64,
    hold: Option<Task<()>>,
}

impl TelegramInlineKeyboardPressState {
    fn new() -> Self {
        Self {
            highlight: Animated::new(0.0),
            is_pressed: false,
            is_hovered: false,
            press_origin: None,
            hold_generation: 0,
            hold: None,
        }
    }

    fn retarget(&mut self, now: MotionTime, reduce_motion: bool) {
        let shows_pressed = self.is_pressed || self.is_hovered;
        let target = if shows_pressed { 1.0 } else { 0.0 };
        self.highlight.set(
            target,
            ease_out(TelegramInlineKeyboardButtonStyle::duration(shows_pressed)),
            now,
            reduce_motion,
        );
    }

    fn set_hovered(&mut self, is_hovered: bool, now: MotionTime, reduce_motion: bool) {
        self.is_hovered = is_hovered;
        self.retarget(now, reduce_motion);
    }

    fn press_began(&mut self, origin: Point<Pixels>, now: MotionTime, reduce_motion: bool) {
        self.is_pressed = true;
        self.press_origin = Some(origin);
        self.retarget(now, reduce_motion);
    }

    fn press_ended(&mut self, now: MotionTime, reduce_motion: bool) {
        self.is_pressed = false;
        self.cancel_hold();
        self.retarget(now, reduce_motion);
    }

    fn cancel_hold(&mut self) {
        self.press_origin = None;
        self.hold_generation = self.hold_generation.wrapping_add(1);
        self.hold = None;
    }

    fn moved_too_far(&self, position: Point<Pixels>) -> bool {
        self.press_origin.is_some_and(|origin| {
            let dx = f32::from(position.x - origin.x);
            let dy = f32::from(position.y - origin.y);
            (dx * dx + dy * dy).sqrt() > BotInlineKeyboardActivationPolicy::MAXIMUM_MOVEMENT
        })
    }

    fn hold_elapsed(&mut self, generation: u64) -> bool {
        let fired = generation == self.hold_generation && self.press_origin.is_some();
        self.hold = None;
        fired
    }
}

fn start_hold(state: &Entity<TelegramInlineKeyboardPressState>, window: &mut Window, cx: &mut App) {
    let generation = state.read(cx).hold_generation;
    let weak = state.downgrade();
    let executor = cx.background_executor().clone();
    let duration = Duration::from_secs_f64(BotInlineKeyboardActivationPolicy::MINIMUM_PRESS_DURATION);
    let task = window.spawn(cx, async move |window_cx| {
        executor.timer(duration).await;
        let _ = window_cx.update(|_, cx| {
            let fired = weak.update(cx, |press, _| press.hold_elapsed(generation)).unwrap_or(false);
            if fired {
                let _ = impact(ImpactStyle::Light);
            }
        });
    });
    state.update(cx, |press, _| press.hold = Some(task));
}

fn uneven_centred_stroke(color: Hsla, line_width: f32, corners: Corners<f32>) -> Div {
    let half = line_width / 2.;
    div()
        .absolute()
        .inset(px(-half))
        .border(px(line_width))
        .border_color(color)
        .rounded_tl(px(corners.top_left + half))
        .rounded_tr(px(corners.top_right + half))
        .rounded_bl(px(corners.bottom_left + half))
        .rounded_br(px(corners.bottom_right + half))
}

struct TelegramInlineKeyboardRow;

impl TelegramInlineKeyboardRow {
    fn layout(spacing: f32, column_count: usize) -> Div {
        div()
            .grid()
            .grid_cols(u16::try_from(column_count.max(1)).unwrap_or(u16::MAX))
            .gap_x(px(spacing))
            .w_full()
    }
}
