use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, BorrowAppContext, Global, Task};
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_platform::app_badge;

use crate::modules::chat::engine::core::rise_conversation_presentation::RiseChatUnreadBadgePresentation;
use crate::modules::notify::engine::rise_notify_unread_presentation::RiseNotifyUnreadPresentation;

#[derive(Default)]
struct RiseAppIconBadgeState {
    last_applied: Option<i64>,
    lifecycle_observers: Vec<RiseSubscription>,
    lifecycle_delivery: Option<Task<()>>,
}

impl Global for RiseAppIconBadgeState {}

pub enum RiseAppIconBadge {}

impl RiseAppIconBadge {
    pub fn start(cx: &mut App) {
        let started = !cx
            .default_global::<RiseAppIconBadgeState>()
            .lifecycle_observers
            .is_empty();
        if started {
            return;
        }
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let lifecycle_observers: Vec<RiseSubscription> = [
            UIApplication::DID_BECOME_ACTIVE_NOTIFICATION,
            UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION,
        ]
        .map(|name| {
            let sender = sender.clone();
            RiseAppEvents::default_center().observe(name, move |_| {
                let _ = sender.unbounded_send(());
            })
        })
        .into_iter()
        .collect();
        let lifecycle_delivery = cx.spawn(async move |cx: &mut AsyncApp| {
            while receiver.next().await.is_some() {
                cx.update(|cx| Self::sync(true, cx));
            }
        });
        cx.update_default_global::<RiseAppIconBadgeState, _>(|state, _| {
            state.lifecycle_observers = lifecycle_observers;
            state.lifecycle_delivery = Some(lifecycle_delivery);
        });
    }

    /// syncs the badge, also in background
    /// no notification extension owns the total on desktop and the socket stays up hidden
    /// so the app keeps counting, only it moves the dock number
    pub fn sync(force: bool, cx: &mut App) {
        let chats = RiseChatUnreadBadgePresentation::shared(cx);
        let bell = RiseNotifyUnreadPresentation::shared(cx);
        let total = chats.read(cx).badge().unread_count + bell.read(cx).unread_count();
        let last_applied = cx.default_global::<RiseAppIconBadgeState>().last_applied;
        if !force && Some(total) == last_applied {
            return;
        }
        Self::apply(total, cx);
    }

    pub fn clear(cx: &mut App) {
        Self::apply(0, cx);
    }

    fn apply(total: i64, cx: &mut App) {
        cx.default_global::<RiseAppIconBadgeState>().last_applied = Some(total);
        let support = app_badge::set_count(u32::try_from(total).unwrap_or(0));
        if !support.is_supported() {
            tracing::debug!(target: "riseonly", total, "the host shows no app icon badge");
        }
    }
}
