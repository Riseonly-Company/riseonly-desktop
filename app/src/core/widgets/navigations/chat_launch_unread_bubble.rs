use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, Bounds, Context, Entity, FontWeight, Hsla, IntoElement, ParentElement, PathBuilder, Pixels,
    RenderOnce, SharedString, Styled, Subscription, Task, Window, canvas, div, point, px,
};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{ease_out, spring};
use rise_ui::swiftui::transition::{Presence, Transition, UnitPoint};

use crate::app::rise_launch_splash::RiseLaunchSplashState;
use crate::app::router::navigation_types::MainTab;
use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseSubscription, UIApplication,
};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatUnreadBadge, RiseChatUnreadBadgePresentation,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

/// arms once on the first authenticated bootstrap and never again until relaunch
/// waits the canon delay, bubble stays down if chats tab opened meanwhile
/// numbers come from the server summary
pub struct ChatLaunchUnreadBubbleStore {
    badge: Option<RiseChatUnreadBadge>,
    presence: Presence<RiseChatUnreadBadge>,
    armed: bool,
    waiting_for_summary: bool,
    task: Option<Task<()>>,
    background_observer: Option<RiseSubscription>,
    background_delivery: Option<Task<()>>,
    summary_observation: Option<Subscription>,
}

impl ChatLaunchUnreadBubbleStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            badge: None,
            presence: Presence::new(None).with_transition(
                Transition::scale_anchored(0.6, UnitPoint::BOTTOM).combined(Transition::opacity()),
            ),
            armed: false,
            waiting_for_summary: false,
            task: None,
            background_observer: None,
            background_delivery: None,
            summary_observation: None,
        }
    }

    const SPLASH_POLL: Duration = Duration::from_millis(100);

    fn delay() -> Duration {
        Self::milliseconds(RiseLimits::client_value("chat_launch_unread_bubble_delay_ms"))
    }

    fn visible() -> Duration {
        Self::milliseconds(RiseLimits::client_value("chat_launch_unread_bubble_visible_ms"))
    }

    fn milliseconds(value: i64) -> Duration {
        Duration::from_millis(u64::try_from(value).unwrap_or(0))
    }

    pub fn badge(&self) -> Option<&RiseChatUnreadBadge> {
        self.badge.as_ref()
    }

    pub fn presence(&self) -> &Presence<RiseChatUnreadBadge> {
        &self.presence
    }

    pub fn arm_for_cold_launch(&mut self, current_tab: MainTab, cx: &mut Context<Self>) {
        if self.armed {
            return;
        }
        self.armed = true;
        if current_tab == MainTab::Chats {
            return;
        }
        self.waiting_for_summary = true;

        let (sender, mut receiver) = mpsc::unbounded::<()>();
        self.background_observer = Some(RiseAppEvents::default_center().observe(
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
            move |_| {
                let _ = sender.unbounded_send(());
            },
        ));
        self.background_delivery = Some(cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                if this.update(cx, |store, cx| store.stop_waiting(cx)).is_err() {
                    break;
                }
            }
        }));

        let delay = Self::delay();
        self.task = Some(cx.spawn(async move |this, cx| {
            while RiseLaunchSplashState::is_visible() {
                cx.background_executor().timer(Self::SPLASH_POLL).await;
            }
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |store, cx| store.show_on_server_summary(cx));
        }));
        cx.notify();
    }

    pub fn note_tab_selected(&mut self, tab: MainTab, cx: &mut Context<Self>) {
        if tab != MainTab::Chats {
            return;
        }
        self.stop_waiting(cx);
        self.task = None;
        if self.badge.is_none() {
            return;
        }
        self.set_badge(None, Some(ease_out(0.18)), cx);
    }

    fn show_on_server_summary(&mut self, cx: &mut Context<Self>) {
        if !self.waiting_for_summary {
            return;
        }
        let Some(summary) = ChatServicesStore::inbox_unread_server_summary(cx) else {
            if self.summary_observation.is_none() {
                let presentation = RiseChatUnreadBadgePresentation::shared(cx);
                self.summary_observation = Some(cx.observe(&presentation, |store, _, cx| {
                    store.summary_observation = None;
                    store.show_on_server_summary(cx);
                }));
            }
            return;
        };
        self.stop_waiting(cx);
        if summary.unread_count <= 0 {
            return;
        }
        self.set_badge(Some(summary), Some(spring(0.38, 0.78)), cx);
        let visible = Self::visible();
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(visible).await;
            let _ = this.update(cx, |store, cx| store.set_badge(None, Some(ease_out(0.22)), cx));
        }));
    }

    fn stop_waiting(&mut self, cx: &mut Context<Self>) {
        self.waiting_for_summary = false;
        self.background_observer = None;
        self.background_delivery = None;
        self.summary_observation = None;
        cx.notify();
    }

    fn set_badge(
        &mut self,
        badge: Option<RiseChatUnreadBadge>,
        spec: Option<rise_ui::swiftui::motion::MotionSpec>,
        cx: &mut Context<Self>,
    ) {
        self.badge = badge.clone();
        self.presence.set_in(badge, spec, RiseClock::monotonic(), cx);
        cx.notify();
    }
}

/// the bubble alone, the tab bar puts it beside the chats button
/// takes no clicks, just a hint
#[derive(IntoElement)]
pub struct ChatLaunchUnreadBubbleUi {
    room_before: f32,
    room_after: f32,
}

impl ChatLaunchUnreadBubbleUi {
    pub const LIFT_ABOVE_BAR: f32 = 8.0;

    pub fn new() -> Self {
        Self { room_before: f32::INFINITY, room_after: f32::INFINITY }
    }

    pub fn room(mut self, room_before: f32, room_after: f32) -> Self {
        self.room_before = room_before;
        self.room_after = room_after;
        self
    }
}

impl Default for ChatLaunchUnreadBubbleUi {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for ChatLaunchUnreadBubbleUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = ChatLaunchUnreadBubbleStore::shared(cx);
        let error = ThemeStore::shared(cx).read(cx).current_theme().semantic.error_100;
        let capsule_length = window.use_keyed_state("chat-launch-unread-bubble-capsule", cx, |_, _| 0.0_f32);
        let shift = ChatLaunchUnreadBubbleContent::shift(*capsule_length.read(cx), self.room_before, self.room_after);
        let now = RiseClock::monotonic();
        let content = store
            .read(cx)
            .presence()
            .render_transitioned(now, window, |badge| {
                ChatLaunchUnreadBubbleContent::body(badge, error, shift, &capsule_length)
            });
        div().flex_none().children(content)
    }
}

/// bar is on the left on desktop so the bubble sits to its right and the pointer faces the bar
struct ChatLaunchUnreadBubbleContent;

impl ChatLaunchUnreadBubbleContent {
    const POINTER_SIDE: f32 = 13.0;
    const POINTER_DEPTH: f32 = 8.0;
    const POINTER_END_INSET: f32 = 20.0;

    /// how far the capsule slides off the pointer
    /// if too long for the room after the button it moves before the pointer, mirrored for the first button
    fn shift(capsule_length: f32, room_before: f32, room_after: f32) -> f32 {
        let half = capsule_length / 2.0;
        let to_end = (half - Self::POINTER_END_INSET).max(0.0);
        let reach = half + Self::POINTER_END_INSET;
        if reach > room_after && room_after <= room_before {
            return -to_end;
        }
        if reach > room_before && room_before < room_after {
            return to_end;
        }
        0.0
    }

    fn paint_pointer(bounds: Bounds<Pixels>, fill: Hsla, window: &mut Window) {
        let mut builder = PathBuilder::fill();
        let origin = bounds.origin;
        builder.move_to(point(origin.x + bounds.size.width, origin.y));
        builder.line_to(point(origin.x, origin.y + bounds.size.height / 2.0));
        builder.line_to(point(origin.x + bounds.size.width, origin.y + bounds.size.height));
        builder.close();
        if let Ok(path) = builder.build() {
            window.paint_path(path, fill);
        }
    }

    fn body(
        badge: &RiseChatUnreadBadge,
        fill: Hsla,
        shift: f32,
        capsule_length: &Entity<f32>,
    ) -> gpui::Div {
        let measured = capsule_length.clone();
        let mut row = div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .px(px(14.0))
            .py(px(9.0))
            .rounded_full()
            .bg(fill)
            .shadow(vec![gpui::BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 0.22),
                offset: gpui::point(px(0.0), px(3.0)),
                blur_radius: px(8.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(Self::segment("bubble.left.and.bubble.right.fill", badge.unread_count));
        if badge.unread_media_count > 0 {
            row = row.child(Self::segment("photo.fill", badge.unread_media_count));
        }
        if badge.unread_replies_count > 0 {
            row = row.child(Self::segment("arrowshape.turn.up.left.fill", badge.unread_replies_count));
        }
        row = row.child(measure("chat-launch-unread-bubble-capsule-size", move |bounds, _, cx| {
            let length = f32::from(bounds.size.height);
            measured.update(cx, |value, cx| {
                if *value != length {
                    *value = length;
                    cx.notify();
                }
            });
        }));
        let pointer = canvas(
            |_, _, _| {},
            move |bounds: Bounds<Pixels>, _, window: &mut Window, _| Self::paint_pointer(bounds, fill, window),
        )
        .flex_none()
        .w(px(Self::POINTER_DEPTH))
        .h(px(Self::POINTER_SIDE))
        .relative()
        .top(px(-shift));
        div()
            .relative()
            .top(px(shift))
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .child(pointer)
            .child(row)
    }

    fn segment(symbol: &'static str, count: i64) -> gpui::Div {
        let text: SharedString = if count > 999 {
            SharedString::new_static("999+")
        } else {
            SharedString::from(count.to_string())
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.0))
            .children(IconUi::sized(symbol, px(15.0), gpui::white(), true))
            .child(
                MainText::new(text)
                    .px(16.0)
                    .font_weight(FontWeight::BOLD)
                    .color(gpui::white())
                    .number_of_lines(1),
            )
    }
}
