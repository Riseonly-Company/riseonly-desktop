use std::collections::HashSet;

use gpui::TestAppContext;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_i18n::tr;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;

use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCParticipant, RiseRTCParticipantRole, RiseRTCRaisedHand,
};
use crate::modules::rtc::shared::rise_rtc_raise_hand_cooldown::RiseRTCRaiseHandCooldown;
use crate::modules::rtc::shared::rise_rtc_room_menu::{
    RiseRTCRoomMenu, RiseRTCRoomMenuAction, RiseRTCRoomMenuCapabilities,
};
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;

fn hand(expires_at_ms: Option<i64>) -> RiseRTCRaisedHand {
    RiseRTCRaisedHand { user_id: "u1".to_owned(), requested_at_ms: 1_000, expires_at_ms, note: None, profile: None }
}

fn participant(role: RiseRTCParticipantRole, server_muted: bool, server_deafened: bool) -> RiseRTCParticipant {
    RiseRTCParticipant {
        user_id: "u1".to_owned(),
        role,
        connected: true,
        microphone_enabled: true,
        camera_enabled: false,
        screen_share_enabled: false,
        server_muted,
        server_deafened,
        server_camera_blocked: false,
        server_screen_share_blocked: false,
        deafened: false,
        hand_raised: false,
        joined_at_ms: 0,
        updated_at_ms: 0,
        lease_expires_at_ms: 0,
        order_sequence: 0,
        profile: None,
    }
}

fn personal_action_ids() -> HashSet<&'static str> {
    HashSet::from([RiseRTCRoomMenuAction::ToggleMuteForMe.raw_value(), RiseRTCRoomMenuAction::ResetVolume.raw_value()])
}

fn moderation_ids(actions: &[HoldContextMenuAction]) -> Vec<String> {
    let personal = personal_action_ids();
    actions.iter().filter(|action| !personal.contains(action.id.as_str())).map(|action| action.id.clone()).collect()
}

fn moderation_titles(actions: &[HoldContextMenuAction]) -> Vec<String> {
    let personal = personal_action_ids();
    actions.iter().filter(|action| !personal.contains(action.id.as_str())).map(|action| action.title.clone()).collect()
}

fn menu(
    participant: &RiseRTCParticipant,
    capabilities: &RiseRTCRoomMenuCapabilities,
    cx: &mut TestAppContext,
) -> Vec<HoldContextMenuAction> {
    cx.update(|cx| {
        let _rtc = RTCServicesStore::shared(cx);
        let outranked = RiseRTCRoomMenu::outranked(participant, capabilities, cx);
        RiseRTCRoomMenu::actions(participant, capabilities, outranked, cx)
    })
}

#[test]
fn a_hand_stands_until_its_deadline() {
    let raised = hand(Some(31_000));
    assert!(!raised.has_expired(30_999));
    assert!(raised.has_expired(31_000));
    assert!(raised.has_expired(31_001));
}

#[test]
fn a_hand_with_no_deadline_never_expires_on_its_own() {
    assert!(!hand(None).has_expired(i64::MAX));
}

#[test]
fn the_window_closes_for_its_full_length_and_then_opens() {
    let raised_at = Date::from_time_interval_since_1970(1_000.0);
    let window = 1.5;
    assert!(!RiseRTCRaiseHandCooldown::allows_raise(Some(raised_at), raised_at.adding_time_interval(1.4), window));
    assert!(RiseRTCRaiseHandCooldown::allows_raise(Some(raised_at), raised_at.adding_time_interval(1.5), window));
}

#[test]
fn the_first_raise_is_never_blocked() {
    assert!(RiseRTCRaiseHandCooldown::allows_raise(None, Date::from_time_interval_since_1970(0.0), 1.5));
}

#[test]
fn a_stamp_in_the_future_does_not_lock_the_button() {
    let now = Date::from_time_interval_since_1970(1_000.0);
    assert!(RiseRTCRaiseHandCooldown::allows_raise(Some(now.adding_time_interval(600.0)), now, 1.5));
}

#[test]
fn the_window_comes_from_canon_and_clears_the_gateway_bucket() {
    assert!(RiseRTCRaiseHandCooldown::window_seconds() >= 1.0);
}

#[gpui::test]
fn the_store_gate_is_the_tested_policy(cx: &mut TestAppContext) {
    let services = cx.update(RTCServicesStore::shared);
    let interactions = cx.update(RTCInteractionsStore::shared);
    services.update(cx, |services, cx| services.clear_raise_hand_cooldown(cx));
    interactions.read_with(cx, |interactions, cx| assert!(interactions.can_raise_hand(cx)));

    services.update(cx, |services, cx| {
        services.begin_raise_hand_cooldown(RiseClock::now(), RiseRTCRaiseHandCooldown::window_seconds(), cx);
    });
    interactions.read_with(cx, |interactions, cx| assert!(!interactions.can_raise_hand(cx)));

    services.update(cx, |services, cx| services.clear_raise_hand_cooldown(cx));
    interactions.read_with(cx, |interactions, cx| assert!(interactions.can_raise_hand(cx)));
}

#[gpui::test]
fn a_menu_without_permission_offers_only_personal_audio(cx: &mut TestAppContext) {
    let listener = participant(RiseRTCParticipantRole::Listener, false, false);
    let actions = menu(&listener, &RiseRTCRoomMenuCapabilities::default(), cx);
    let ids: Vec<&str> = actions.iter().map(|action| action.id.as_str()).collect();
    assert_eq!(ids, vec![RiseRTCRoomMenuAction::ToggleMuteForMe.raw_value()]);
    assert!(actions.iter().all(|action| !action.destructive));
}

#[gpui::test]
fn stage_moves_follow_the_target_role(cx: &mut TestAppContext) {
    let capabilities = RiseRTCRoomMenuCapabilities { can_manage_stage: true, ..Default::default() };
    let for_listener = menu(&participant(RiseRTCParticipantRole::Listener, false, false), &capabilities, cx);
    assert_eq!(moderation_ids(&for_listener), vec![RiseRTCRoomMenuAction::PromoteToSpeaker.raw_value().to_owned()]);

    let for_speaker = menu(&participant(RiseRTCParticipantRole::Speaker, false, false), &capabilities, cx);
    assert_eq!(moderation_ids(&for_speaker), vec![RiseRTCRoomMenuAction::MoveToAudience.raw_value().to_owned()]);
}

#[gpui::test]
fn mute_and_deafen_entries_read_the_current_server_state(cx: &mut TestAppContext) {
    let capabilities = RiseRTCRoomMenuCapabilities { can_mute: true, can_deafen: true, ..Default::default() };
    let quiet = menu(&participant(RiseRTCParticipantRole::Speaker, true, true), &capabilities, cx);
    assert_eq!(moderation_titles(&quiet), vec![tr("rtc_allow_unmute"), tr("rtc_allow_undeafen")]);

    let loud = menu(&participant(RiseRTCParticipantRole::Speaker, false, false), &capabilities, cx);
    assert_eq!(moderation_titles(&loud), vec![tr("rtc_force_mute"), tr("rtc_force_deafen")]);
}

#[gpui::test]
fn removal_is_the_only_destructive_entry(cx: &mut TestAppContext) {
    let capabilities = RiseRTCRoomMenuCapabilities {
        can_manage_stage: true,
        can_mute: true,
        can_deafen: true,
        can_block_camera: false,
        can_block_screen_share: false,
        can_remove: true,
        chat_id: None,
    };
    let actions = menu(&participant(RiseRTCParticipantRole::Speaker, false, false), &capabilities, cx);
    let destructive: Vec<&str> =
        actions.iter().filter(|action| action.destructive).map(|action| action.id.as_str()).collect();
    assert_eq!(destructive, vec![RiseRTCRoomMenuAction::Kick.raw_value()]);
    assert_eq!(moderation_ids(&actions).len(), 4);
    assert_eq!(actions.first().map(|action| action.id.as_str()), Some(RiseRTCRoomMenuAction::ToggleMuteForMe.raw_value()));
}
