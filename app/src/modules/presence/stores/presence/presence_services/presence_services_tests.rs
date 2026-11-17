use gpui::TestAppContext;

use super::presence_services::{PresenceIndicatorProjectionRegistry, PresenceIndicatorValue};
use crate::modules::presence::stores::presence::presence_actions::presence_types::PresenceState;

fn online(is_online: bool) -> PresenceState {
    PresenceState { is_online, ..PresenceState::default() }
}

#[gpui::test]
fn updates_only_the_matching_user_projection(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut registry = PresenceIndicatorProjectionRegistry::new();
        let first = registry.projection("user-a", "chat-a", false, None, cx);
        let second = registry.projection("user-b", "chat-b", false, None, cx);

        registry.update("user-a", Some(&online(true)), cx);

        assert_eq!(first.read(cx).value(), PresenceIndicatorValue::Online);
        assert_eq!(second.read(cx).value(), PresenceIndicatorValue::Hidden);
        assert!(registry.projection("user-b", "chat-b", false, None, cx) == second);
    });
}

#[gpui::test]
fn preserves_in_chat_online_and_fallback_precedence(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut registry = PresenceIndicatorProjectionRegistry::new();
        let matching_chat = registry.projection("user-a", "chat-a", true, None, cx);
        let other_chat = registry.projection("user-a", "chat-b", true, None, cx);

        registry.update(
            "user-a",
            Some(&PresenceState { is_online: false, in_chat_id: Some("chat-a".to_owned()), ..PresenceState::default() }),
            cx,
        );

        assert_eq!(matching_chat.read(cx).value(), PresenceIndicatorValue::InChat);
        assert_eq!(other_chat.read(cx).value(), PresenceIndicatorValue::Hidden);

        registry.update("user-a", Some(&online(true)), cx);

        assert_eq!(matching_chat.read(cx).value(), PresenceIndicatorValue::Online);
        assert_eq!(other_chat.read(cx).value(), PresenceIndicatorValue::Online);
    });
}

#[gpui::test]
fn reset_returns_live_projections_to_their_fallback(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut registry = PresenceIndicatorProjectionRegistry::new();
        let fallback_online = registry.projection("user-a", "chat-a", true, Some(&online(false)), cx);
        let fallback_offline = registry.projection("user-b", "chat-b", false, Some(&online(true)), cx);

        assert_eq!(fallback_online.read(cx).value(), PresenceIndicatorValue::Hidden);
        assert_eq!(fallback_offline.read(cx).value(), PresenceIndicatorValue::Online);

        registry.reset(cx);

        assert_eq!(fallback_online.read(cx).value(), PresenceIndicatorValue::Online);
        assert_eq!(fallback_offline.read(cx).value(), PresenceIndicatorValue::Hidden);
    });
}
