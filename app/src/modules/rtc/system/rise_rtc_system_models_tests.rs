use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCAudioProcessingMode, RiseRTCAudioRoutingDecision, RiseRTCAudioRoutingPolicy, RiseRTCCallMediaMode,
    RiseRTCCallState, RiseRTCConnectionState, RiseRTCRoomMode,
};
use crate::modules::rtc::system::rise_rtc_system_bridges::{
    RiseRTCSystemVoIPAPNSEnvironment, RiseRTCSystemVoIPTokenRegistration,
};
use crate::modules::rtc::system::rise_rtc_system_models::{
    RiseRTCCallActivationGate, RiseRTCCallSystemEventMapper, RiseRTCSystemAccountFence, RiseRTCSystemBackendEndAction,
    RiseRTCSystemCallDirection, RiseRTCSystemCallPhase, RiseRTCSystemCallRacePolicy, RiseRTCSystemVoIPEvent,
    RiseRTCSystemVoIPServerPolicy,
};

#[test]
fn outgoing_connected_at_requires_both_inputs_in_either_order() {
    let accepted_first_before_media =
        RiseRTCCallActivationGate::is_ready(RiseRTCCallState::Accepted, RiseRTCConnectionState::Connecting);
    let accepted_first_after_media =
        RiseRTCCallActivationGate::is_ready(RiseRTCCallState::Accepted, RiseRTCConnectionState::Connected);
    let media_first_before_accepted =
        RiseRTCCallActivationGate::is_ready(RiseRTCCallState::Ringing, RiseRTCConnectionState::Connected);
    let media_first_after_accepted =
        RiseRTCCallActivationGate::is_ready(RiseRTCCallState::Accepted, RiseRTCConnectionState::Connected);

    assert!(!accepted_first_before_media);
    assert!(accepted_first_after_media);
    assert!(!media_first_before_accepted);
    assert!(media_first_after_accepted);
    assert!(!RiseRTCCallActivationGate::is_ready(RiseRTCCallState::Ended, RiseRTCConnectionState::Connected));
}

#[test]
fn audio_routing_protects_receiver_calls_and_defaults_visual_rooms_to_speaker() {
    assert_eq!(
        RiseRTCAudioRoutingPolicy::decision(RiseRTCRoomMode::DirectCall, Some(RiseRTCCallMediaMode::Audio)),
        RiseRTCAudioRoutingDecision { processing_mode: RiseRTCAudioProcessingMode::VoiceChat, prefers_speaker: false }
    );
    assert_eq!(
        RiseRTCAudioRoutingPolicy::decision(RiseRTCRoomMode::DirectCall, Some(RiseRTCCallMediaMode::Video)),
        RiseRTCAudioRoutingDecision { processing_mode: RiseRTCAudioProcessingMode::VideoChat, prefers_speaker: true }
    );
    for mode in [RiseRTCRoomMode::Voice, RiseRTCRoomMode::Stage, RiseRTCRoomMode::GroupVoice] {
        assert_eq!(
            RiseRTCAudioRoutingPolicy::decision(mode, None),
            RiseRTCAudioRoutingDecision { processing_mode: RiseRTCAudioProcessingMode::VideoChat, prefers_speaker: true }
        );
    }
}

#[test]
fn repository_call_states_map_to_call_kit_before_teardown() {
    assert_eq!(RiseRTCCallSystemEventMapper::event(RiseRTCCallState::Ringing), None);
    assert_eq!(RiseRTCCallSystemEventMapper::event(RiseRTCCallState::Accepted), Some(RiseRTCSystemVoIPEvent::Answered));
    assert_eq!(RiseRTCCallSystemEventMapper::event(RiseRTCCallState::Declined), Some(RiseRTCSystemVoIPEvent::Declined));
    assert_eq!(RiseRTCCallSystemEventMapper::event(RiseRTCCallState::Cancelled), Some(RiseRTCSystemVoIPEvent::Cancelled));
    assert_eq!(RiseRTCCallSystemEventMapper::event(RiseRTCCallState::TimedOut), Some(RiseRTCSystemVoIPEvent::TimedOut));
    assert_eq!(RiseRTCCallSystemEventMapper::event(RiseRTCCallState::Ended), Some(RiseRTCSystemVoIPEvent::Ended));
}

#[test]
fn repeated_terminal_snapshots_do_not_reend_managed_call() {
    assert!(RiseRTCSystemCallRacePolicy::should_finish_managed_call(RiseRTCSystemCallPhase::Active));
    assert!(!RiseRTCSystemCallRacePolicy::should_finish_managed_call(RiseRTCSystemCallPhase::Ended));
}

#[test]
fn outgoing_end_before_start_response_defers_cancel_until_server_id_arrives() {
    assert_eq!(
        RiseRTCSystemCallRacePolicy::backend_end_action(
            RiseRTCSystemCallDirection::Outgoing,
            RiseRTCSystemCallPhase::OutgoingStarting,
            false
        ),
        RiseRTCSystemBackendEndAction::WaitForStartThenCancel
    );
    assert_eq!(
        RiseRTCSystemCallRacePolicy::backend_end_action(
            RiseRTCSystemCallDirection::Outgoing,
            RiseRTCSystemCallPhase::OutgoingRinging,
            true
        ),
        RiseRTCSystemBackendEndAction::Cancel
    );
    assert_eq!(
        RiseRTCSystemCallRacePolicy::backend_end_action(
            RiseRTCSystemCallDirection::Outgoing,
            RiseRTCSystemCallPhase::Active,
            true
        ),
        RiseRTCSystemBackendEndAction::End
    );
}

#[test]
fn incoming_end_uses_decline_until_answer_wins_then_uses_end() {
    assert_eq!(
        RiseRTCSystemCallRacePolicy::backend_end_action(
            RiseRTCSystemCallDirection::Incoming,
            RiseRTCSystemCallPhase::IncomingRinging,
            true
        ),
        RiseRTCSystemBackendEndAction::Decline
    );
    assert_eq!(
        RiseRTCSystemCallRacePolicy::backend_end_action(
            RiseRTCSystemCallDirection::Incoming,
            RiseRTCSystemCallPhase::Answering,
            true
        ),
        RiseRTCSystemBackendEndAction::EndThenDecline
    );
    assert_eq!(
        RiseRTCSystemCallRacePolicy::backend_end_action(
            RiseRTCSystemCallDirection::Incoming,
            RiseRTCSystemCallPhase::Active,
            true
        ),
        RiseRTCSystemBackendEndAction::End
    );
}

#[test]
fn stale_async_completion_cannot_resurrect_ended_call() {
    let expected = Uuid::new_v4();

    assert!(RiseRTCSystemCallRacePolicy::accepts_async_completion(
        expected,
        Some(expected),
        RiseRTCSystemCallPhase::Answering
    ));
    assert!(!RiseRTCSystemCallRacePolicy::accepts_async_completion(
        expected,
        Some(Uuid::new_v4()),
        RiseRTCSystemCallPhase::Answering
    ));
    assert!(!RiseRTCSystemCallRacePolicy::accepts_async_completion(
        expected,
        Some(expected),
        RiseRTCSystemCallPhase::Ending
    ));
}

#[test]
fn cold_launch_fence_allows_epoch_restoration_but_never_account_or_generation_change() {
    let cold_fence = RiseRTCSystemAccountFence { account_id: "user-a".to_owned(), account_epoch: None, generation: 11 };
    assert!(cold_fence.matches(Some("user-a"), Some("restored-epoch"), 11));
    assert!(!cold_fence.matches(Some("user-b"), Some("restored-epoch"), 11));
    assert!(!cold_fence.matches(Some("user-a"), Some("restored-epoch"), 12));
}

#[test]
fn established_epoch_rejects_replacement_session() {
    let fence = RiseRTCSystemAccountFence {
        account_id: "user-a".to_owned(),
        account_epoch: Some("epoch-1".to_owned()),
        generation: 11,
    };
    assert!(fence.matches(Some("user-a"), Some("epoch-1"), 11));
    assert!(!fence.matches(Some("user-a"), Some("epoch-2"), 11));
}

#[test]
fn vo_ip_registration_confirmation_key_fences_every_ownership_dimension() {
    let registration = |environment: RiseRTCSystemVoIPAPNSEnvironment, ownership_generation: i64, installation_id: &str| {
        RiseRTCSystemVoIPTokenRegistration {
            token: "aabbccdd".to_owned(),
            environment,
            ownership_generation,
            installation_id: installation_id.to_owned(),
        }
    };
    let base = registration(RiseRTCSystemVoIPAPNSEnvironment::Sandbox, 42, "installation-a");
    let production = registration(RiseRTCSystemVoIPAPNSEnvironment::Production, 42, "installation-a");
    let next_generation = registration(RiseRTCSystemVoIPAPNSEnvironment::Sandbox, 43, "installation-a");
    let other_installation = registration(RiseRTCSystemVoIPAPNSEnvironment::Sandbox, 42, "installation-b");

    assert_ne!(base.confirmation_key(), production.confirmation_key());
    assert_ne!(base.confirmation_key(), next_generation.confirmation_key());
    assert_ne!(base.confirmation_key(), other_installation.confirmation_key());
}

#[test]
fn answered_push_is_an_update_but_still_forbidden_on_vo_ip_transport() {
    assert!(!RiseRTCSystemVoIPEvent::Answered.is_terminal());
    assert!(!RiseRTCSystemVoIPServerPolicy::may_use_vo_ip_transport(&RiseRTCSystemVoIPEvent::Answered));
    assert!(RiseRTCSystemVoIPEvent::AnsweredElsewhere.is_terminal());
}
