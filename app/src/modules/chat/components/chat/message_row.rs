use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, DispatchPhase, ElementId, Entity, Hsla, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollDelta, SharedString, TouchPhase, Window, canvas, point,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::hold_context_menu_ui::bubble_scroll_passthrough::BubbleScrollPassthrough;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::icon_ui::icon_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::{EnvKey, env};
use rise_ui::swiftui::motion::{interactive_spring, spring};
use rise_ui::swiftui::prelude::*;

use rise_ui::swiftui::text_selection::selectable_text_took_press;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::formatted_text_ui::FormattedTextInlinePreparation;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::components::chat::message_bubble::MessageBubble;
use crate::modules::chat::shared::chat_saved_message_presentation::RiseMessageSenderIdentity;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::message_permissions::ChatMessageAction;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

/// where the sticky chrome of a bubble stops
pub struct ChatStickyBottomLimitKey;

impl EnvKey for ChatStickyBottomLimitKey {
    type Value = f32;

    fn default_value() -> Self::Value {
        f32::MAX
    }

    fn fingerprint(_value: &Self::Value) -> Option<u64> {
        None
    }
}

pub fn chat_sticky_bottom_limit(cx: &App) -> f32 {
    env::<ChatStickyBottomLimitKey>(cx)
}

type MessageRowStringAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;
type MessageRowAction = Rc<dyn Fn(&mut Window, &mut App)>;

const REPLY_TRIGGER_DISTANCE: f32 = 56.0;
const REPLY_MAX_PULL: f32 = 80.0;
const SWIPE_MINIMUM_DISTANCE: f32 = 18.0;

struct MessageRowSwipe {
    drag_offset: Animated<f32>,
    reply_armed: bool,
    began: bool,
    mouse_start: Option<Point<Pixels>>,
    trackpad_start: Option<Point<Pixels>>,
    trackpad_translation: Point<Pixels>,
}

impl MessageRowSwipe {
    fn new() -> Self {
        Self {
            drag_offset: Animated::new(0.0),
            reply_armed: false,
            began: false,
            mouse_start: None,
            trackpad_start: None,
            trackpad_translation: Point::default(),
        }
    }

    fn on_changed(&mut self, start: Point<Pixels>, translation: Point<Pixels>, cx: &mut Context<Self>) {
        let dx = f32::from(translation.x);
        let dy = f32::from(translation.y);
        if !self.began {
            if dx.hypot(dy) < SWIPE_MINIMUM_DISTANCE {
                return;
            }
            self.began = true;
        }
        if BubbleScrollPassthrough::with_shared(|passthrough| passthrough.contains(start)) {
            return;
        }
        if dx.abs() <= dy.abs() {
            return;
        }
        if dx >= 0.0 {
            if *self.drag_offset.target() != 0.0 {
                let spec = interactive_spring(0.15, 0.86, 0.25);
                self.drag_offset.set_in(0.0, spec, RiseClock::monotonic(), cx);
                cx.notify();
            }
            return;
        }
        let pull = (-dx).min(REPLY_MAX_PULL);
        let damped = if pull <= REPLY_TRIGGER_DISTANCE {
            pull
        } else {
            REPLY_TRIGGER_DISTANCE + (pull - REPLY_TRIGGER_DISTANCE) * 0.35
        };
        self.drag_offset.jump(-damped);
        let armed = pull >= REPLY_TRIGGER_DISTANCE;
        if armed != self.reply_armed {
            self.reply_armed = armed;
            if armed {
                let _ = impact(ImpactStyle::Medium);
            }
        }
        cx.notify();
    }

    /// whether the swipe asks for a reply
    fn on_ended(&mut self, cx: &mut Context<Self>) -> bool {
        let fire = self.reply_armed;
        if self.began {
            self.drag_offset.set_in(0.0, spring(0.32, 0.72), RiseClock::monotonic(), cx);
        }
        self.reply_armed = false;
        self.began = false;
        self.mouse_start = None;
        self.trackpad_start = None;
        cx.notify();
        fire
    }
}

#[derive(IntoElement)]
pub struct MessageRow {
    id: ElementId,
    message: Arc<GetMessageMessage>,
    chat: Arc<ChatInfo>,
    is_own: bool,
    is_first_in_group: bool,
    is_last_in_group: bool,
    show_avatar: bool,
    draws_inline_avatar: bool,
    allows_reply: bool,
    interactions_enabled: bool,
    allows_message_selection: bool,
    allows_composer_actions: bool,
    leading_context_actions: Vec<HoldContextMenuAction>,
    on_reaction: Option<MessageRowStringAction>,
    on_action: Option<MessageRowStringAction>,
    on_open_sender_profile: Option<MessageRowAction>,
    can_double_tap: Rc<dyn Fn() -> bool>,
    on_double_tap: Option<MessageRowAction>,
    prepared_text: Option<FormattedTextInlinePreparation>,
}

impl MessageRow {
    pub fn new(
        message: impl Into<Arc<GetMessageMessage>>,
        chat: impl Into<Arc<ChatInfo>>,
        is_own: bool,
        is_first_in_group: bool,
        is_last_in_group: bool,
        show_avatar: bool,
    ) -> Self {
        let message = message.into();
        Self {
            id: ElementId::from(SharedString::from(format!("message-row-{}", message.id))),
            message,
            chat: chat.into(),
            is_own,
            is_first_in_group,
            is_last_in_group,
            show_avatar,
            draws_inline_avatar: false,
            allows_reply: true,
            interactions_enabled: true,
            allows_message_selection: false,
            allows_composer_actions: true,
            leading_context_actions: Vec::new(),
            on_reaction: None,
            on_action: None,
            on_open_sender_profile: None,
            can_double_tap: Rc::new(|| true),
            on_double_tap: None,
            prepared_text: None,
        }
    }

    /// keys the swipe state by identity, default key is the message id
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    pub fn draws_inline_avatar(mut self, draws_inline_avatar: bool) -> Self {
        self.draws_inline_avatar = draws_inline_avatar;
        self
    }

    pub fn allows_reply(mut self, allows_reply: bool) -> Self {
        self.allows_reply = allows_reply;
        self
    }

    pub fn interactions_enabled(mut self, interactions_enabled: bool) -> Self {
        self.interactions_enabled = interactions_enabled;
        self
    }

    pub fn allows_message_selection(mut self, allows_message_selection: bool) -> Self {
        self.allows_message_selection = allows_message_selection;
        self
    }

    pub fn allows_composer_actions(mut self, allows_composer_actions: bool) -> Self {
        self.allows_composer_actions = allows_composer_actions;
        self
    }

    pub fn leading_context_actions(mut self, leading_context_actions: Vec<HoldContextMenuAction>) -> Self {
        self.leading_context_actions = leading_context_actions;
        self
    }

    pub fn on_reaction(mut self, on_reaction: Option<MessageRowStringAction>) -> Self {
        self.on_reaction = on_reaction;
        self
    }

    pub fn on_action(mut self, on_action: Option<MessageRowStringAction>) -> Self {
        self.on_action = on_action;
        self
    }

    pub fn on_open_sender_profile(mut self, on_open_sender_profile: Option<MessageRowAction>) -> Self {
        self.on_open_sender_profile = on_open_sender_profile;
        self
    }

    pub fn can_double_tap(mut self, can_double_tap: Rc<dyn Fn() -> bool>) -> Self {
        self.can_double_tap = can_double_tap;
        self
    }

    pub fn on_double_tap(mut self, on_double_tap: Option<MessageRowAction>) -> Self {
        self.on_double_tap = on_double_tap;
        self
    }

    pub fn prepared_text(mut self, prepared_text: Option<FormattedTextInlinePreparation>) -> Self {
        self.prepared_text = prepared_text;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn reserves_avatar_space(&self) -> bool {
        self.show_avatar && MessageSizes::reserves_avatar_space(&self.chat, self.is_own)
    }

    fn avatar_slot(&self, cx: &App) -> AnyElement {
        let side = MessageSizes::avatar_size();
        if !(self.draws_inline_avatar && self.is_last_in_group) {
            return div().w(px(side)).h(px(side)).into_any_element();
        }
        let participant = RiseMessageSenderIdentity::participant(&self.message, &self.chat, cx);
        let source = (!participant.more.logo.is_empty()).then(|| SharedString::from(participant.more.logo.clone()));
        let on_open_sender_profile = self.on_open_sender_profile.clone();
        div()
            .id(self.child_id("avatar"))
            .rounded_full()
            .on_click(move |_, window, cx| {
                if let Some(on_open_sender_profile) = &on_open_sender_profile {
                    on_open_sender_profile(window, cx);
                }
            })
            .child(
                UserLogoUi::new(self.child_id("avatar-logo"))
                    .source(source)
                    .name(Some(SharedString::from(participant.name.clone())))
                    .user_id(Some(SharedString::from(participant.id.clone())))
                    .color_scope_id(Some(SharedString::from(self.chat.id.clone())))
                    .is_deleted(participant.is_deleted)
                    .size(side)
                    .show_story_ring(false)
                    .image_cache_policy(CleverImageCachePolicy::MemoryOnly),
            )
            .into_any_element()
    }

    fn row_content(self, cx: &App) -> impl IntoElement + use<> {
        let reserves_avatar_space = self.reserves_avatar_space();
        let is_own = self.is_own;
        let is_rtl = LayoutDirection::current().is_right_to_left();
        let opposite_side = || div().flex_1().min_w(px(MessageSizes::opposite_side_min_width()));
        let avatar_slot = reserves_avatar_space.then(|| {
            let inset = px((MessageSizes::avatar_leading_inset() - MessageSizes::row_side_inset()).max(0.0));
            div()
                .when(!is_rtl, |slot| slot.pl(inset))
                .when(is_rtl, |slot| slot.pr(inset))
                .child(self.avatar_slot(cx))
        });
        let bubble = MessageBubble::new(self.child_id("bubble"), self.message, self.chat)
            .is_own(self.is_own)
            .is_first_in_group(self.is_first_in_group)
            .is_last_in_group(self.is_last_in_group)
            .interactions_enabled(self.interactions_enabled)
            .allows_message_selection(self.allows_message_selection)
            .allows_composer_actions(self.allows_composer_actions)
            .leading_context_actions(self.leading_context_actions)
            .on_reaction(self.on_reaction)
            .on_action(self.on_action)
            .on_open_sender_profile(self.on_open_sender_profile)
            .can_double_tap(self.can_double_tap)
            .on_double_tap(self.on_double_tap)
            .prepared_text(self.prepared_text);

        div()
            .w_full()
            .flex()
            .when(!is_rtl, |row| row.flex_row())
            .when(is_rtl, |row| row.flex_row_reverse())
            .items_end()
            .when(is_own, |row| row.child(opposite_side()))
            .children(avatar_slot)
            .child(
                div()
                    .min_w_0()
                    .when(reserves_avatar_space && !is_rtl, |slot| slot.pl(px(MessageSizes::avatar_gap())))
                    .when(reserves_avatar_space && is_rtl, |slot| slot.pr(px(MessageSizes::avatar_gap())))
                    .child(bubble),
            )
            .when(!is_own, |row| row.child(opposite_side()))
    }
}

fn reply_hint(drag_offset: f32, reply_armed: bool, tint: Hsla) -> impl IntoElement {
    let progress = (-drag_offset / REPLY_TRIGGER_DISTANCE).min(1.0);
    let scale = 0.6 + 0.4 * progress + if reply_armed { 0.12 } else { 0.0 };
    let side = 15.0;
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .pr(px(6.0))
        .flex()
        .items_center()
        .opacity(progress)
        .child(
            div()
                .w(px(side))
                .h(px(side))
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized("arrowshape.turn.up.left.fill", px(side * scale), tint, true)),
        )
}

impl RenderOnce for MessageRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let swipe: Entity<MessageRowSwipe> = window.use_keyed_state(self.id.clone(), cx, |_, _| MessageRowSwipe::new());
        let now = RiseClock::monotonic();
        let (drag_offset, reply_armed, tracks_mouse) = {
            let state = swipe.read(cx);
            if state.drag_offset.is_running(now) {
                window.request_animation_frame();
            }
            (state.drag_offset.value(now), state.reply_armed, state.mouse_start.is_some())
        };
        let tint = ThemeStore::shared(cx).read(cx).current_theme().primary._100;
        let allows_reply = self.allows_reply;
        let on_action = self.on_action.clone();
        let root_id = self.child_id("swipe");
        let own_trailing_extra = if self.is_own { MessageSizes::own_trailing_extra() } else { 0.0 };
        let is_rtl = LayoutDirection::current().is_right_to_left();

        let mouse_swipe = swipe.clone();
        let wheel_swipe = swipe.clone();
        let wheel_on_action = on_action.clone();
        let tracker_swipe = swipe;
        let tracker_on_action = on_action;

        div()
            .id(root_id)
            .w_full()
            .px(px(MessageSizes::row_side_inset()))
            .when(own_trailing_extra > 0.0 && !is_rtl, |row| {
                row.pr(px(MessageSizes::row_side_inset() + own_trailing_extra))
            })
            .when(own_trailing_extra > 0.0 && is_rtl, |row| {
                row.pl(px(MessageSizes::row_side_inset() + own_trailing_extra))
            })
            .when(allows_reply, |row| {
                row.on_mouse_down(MouseButton::Left, move |event, _, cx| {
                    if selectable_text_took_press(event.position, cx) {
                        return;
                    }
                    let position = event.position;
                    mouse_swipe.update(cx, |state, cx| {
                        state.mouse_start = Some(position);
                        cx.notify();
                    });
                })
                .on_scroll_wheel(move |event, window, cx| {
                    let on_action = wheel_on_action.clone();
                    match event.touch_phase {
                        TouchPhase::Started => {
                            let position = event.position;
                            wheel_swipe.update(cx, |state, _| {
                                state.trackpad_start = Some(position);
                                state.trackpad_translation = Point::default();
                            });
                        }
                        TouchPhase::Moved => {
                            let ScrollDelta::Pixels(delta) = event.delta else {
                                return;
                            };
                            let swallowed = wheel_swipe.update(cx, |state, cx| {
                                let Some(start) = state.trackpad_start else {
                                    return false;
                                };
                                state.trackpad_translation = point(
                                    state.trackpad_translation.x + delta.x,
                                    state.trackpad_translation.y + delta.y,
                                );
                                let translation = state.trackpad_translation;
                                state.on_changed(start, translation, cx);
                                *state.drag_offset.target() != 0.0
                            });
                            if swallowed {
                                cx.stop_propagation();
                            }
                        }
                        TouchPhase::Ended => {
                            let fire = wheel_swipe.update(cx, |state, cx| {
                                state.trackpad_start.is_some() && state.on_ended(cx)
                            });
                            if fire && let Some(on_action) = on_action {
                                on_action(ChatMessageAction::REPLY, window, cx);
                            }
                        }
                        TouchPhase::Cancelled => {
                            wheel_swipe.update(cx, |state, cx| {
                                if state.trackpad_start.is_some() {
                                    state.on_ended(cx);
                                }
                            });
                        }
                    }
                })
            })
            .child(
                div()
                    .relative()
                    .w_full()
                    .when(drag_offset < 0.0, |stack| stack.child(reply_hint(drag_offset, reply_armed, tint)))
                    .child(div().relative().left(px(drag_offset)).w_full().child(self.row_content(cx))),
            )
            .when(allows_reply && tracks_mouse, |row| {
                row.child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, _| {
                            let move_swipe = tracker_swipe.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase != DispatchPhase::Bubble {
                                    return;
                                }
                                let position = event.position;
                                move_swipe.update(cx, |state, cx| {
                                    if let Some(start) = state.mouse_start {
                                        state.on_changed(start, position - start, cx);
                                    }
                                });
                            });
                            let up_swipe = tracker_swipe.clone();
                            let on_action = tracker_on_action.clone();
                            window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                                if phase != DispatchPhase::Bubble {
                                    return;
                                }
                                let fire = up_swipe.update(cx, |state, cx| {
                                    state.mouse_start.is_some() && state.on_ended(cx)
                                });
                                if fire && let Some(on_action) = &on_action {
                                    on_action(ChatMessageAction::REPLY, window, cx);
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
    }
}
