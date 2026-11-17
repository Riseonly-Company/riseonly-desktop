use std::collections::HashSet;

use rise_engine::rise_wire::rise_wire_contracts::RiseWireReplayPolicy;

use crate::modules::chat::engine::core::rise_conversation_rpc::RiseConversationEndpoints;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatChannelManagementPolicy, ChatPerm, ChatVoiceConnectGateDecision, ChatVoiceConnectGatePolicy,
    VoiceChannelConfig, VoiceChannelConfigValidationError,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChatTopic;
use crate::modules::rtc::shared::rise_rtc_room_locator::{RiseRTCRoomLocator, RiseRTCRoomLocatorError};

#[test]
fn legacy_topic_defaults_to_text_without_voice_configuration() -> anyhow::Result<()> {
    let dto = serde_json::from_str::<RiseStoredChatTopic>(r#"{"topic_id":"topic-1","title":"General"}"#)?;

    let topic = dto.to_topic();
    assert_eq!(topic.channel_kind, ChatChannelKind::Text);
    assert!(topic.voice_config.is_none());
    assert!(!topic.is_rtc_channel());
    Ok(())
}

#[test]
fn voice_topic_decodes_full_gateway_configuration() -> anyhow::Result<()> {
    let dto = serde_json::from_str::<RiseStoredChatTopic>(
        r#"{
          "topic_id": "5c9344fc-c046-4b36-b614-9adfc37df2fa",
          "title": "Lounge",
          "channel_kind": "voice",
          "voice_config": {
            "user_limit": 20,
            "speaker_limit": 8,
            "audio_bitrate_kbps": 64,
            "video_enabled": true,
            "screen_share_enabled": false,
            "default_muted": false,
            "region": "KZ-ALA"
          }
        }"#,
    )?;

    let topic = dto.to_topic();
    assert_eq!(topic.channel_kind, ChatChannelKind::Voice);
    assert!(topic.is_rtc_channel());
    let config = topic.voice_config.as_ref().expect("a voice topic carries its configuration");
    assert_eq!(config.user_limit, 20);
    assert_eq!(config.speaker_limit, 8);
    assert_eq!(config.audio_bitrate_kbps, 64);
    assert!(config.video_enabled);
    assert!(!config.screen_share_enabled);
    assert!(!config.default_muted);
    assert_eq!(config.region, "kz-ala");
    Ok(())
}

#[test]
fn stage_defaults_match_backend_contract() {
    let config = VoiceChannelConfig::defaults(ChatChannelKind::Stage).expect("a stage has defaults");
    assert_eq!(config.user_limit, 0);
    assert_eq!(config.speaker_limit, 25);
    assert_eq!(config.audio_bitrate_kbps, 32);
    assert!(config.video_enabled);
    assert!(config.screen_share_enabled);
    assert!(config.default_muted);
    assert!(config.region.is_empty());
}

#[test]
fn voice_configuration_normalizes_bitrate_and_region() {
    let config = VoiceChannelConfig {
        user_limit: 100,
        speaker_limit: 25,
        audio_bitrate_kbps: 512,
        video_enabled: true,
        screen_share_enabled: true,
        default_muted: false,
        region: "  EU-West_1 ".to_owned(),
    };

    let normalized = config.normalized(ChatChannelKind::Voice).expect("the limits are valid");
    assert_eq!(normalized.audio_bitrate_kbps, 128);
    assert_eq!(normalized.region, "eu-west_1");
}

#[test]
fn voice_configuration_rejects_invalid_limits() {
    let mut config = VoiceChannelConfig::defaults(ChatChannelKind::Voice).expect("a voice channel has defaults");
    config.user_limit = 1;
    assert_eq!(
        config.normalized(ChatChannelKind::Voice).err(),
        Some(VoiceChannelConfigValidationError::InvalidUserLimit(1))
    );

    config.user_limit = 10;
    config.speaker_limit = 11;
    assert_eq!(
        config.normalized(ChatChannelKind::Voice).err(),
        Some(VoiceChannelConfigValidationError::SpeakerLimitExceedsUserLimit)
    );
}

#[test]
fn room_locator_matches_backend_room_identifiers() {
    let chat_id = "3C264C89-EF35-47B0-AE7D-C1989B811C31";
    let topic_id = "5C9344FC-C046-4B36-B614-9ADFC37DF2FA";
    let call_id = "09B5272E-F559-44D2-B22F-C4D4CA115C07";

    assert_eq!(
        RiseRTCRoomLocator::topic_room_id(chat_id, topic_id).as_deref(),
        Ok("rtc:chat:3c264c89-ef35-47b0-ae7d-c1989b811c31:channel:5c9344fc-c046-4b36-b614-9adfc37df2fa")
    );
    assert_eq!(
        RiseRTCRoomLocator::group_room_id(chat_id).as_deref(),
        Ok("rtc:chat:3c264c89-ef35-47b0-ae7d-c1989b811c31:group")
    );
    assert_eq!(
        RiseRTCRoomLocator::direct_call_room_id(call_id).as_deref(),
        Ok("rtc:call:09b5272e-f559-44d2-b22f-c4d4ca115c07")
    );
}

#[test]
fn room_locator_rejects_non_uuid_identifiers() {
    assert_eq!(RiseRTCRoomLocator::group_room_id("chat-1"), Err(RiseRTCRoomLocatorError::InvalidChatID));
    assert_eq!(
        RiseRTCRoomLocator::topic_room_id("3c264c89-ef35-47b0-ae7d-c1989b811c31", ""),
        Err(RiseRTCRoomLocatorError::InvalidTopicID)
    );
}

#[test]
fn voice_permission_bits_match_stable_backend_mask() {
    assert_eq!(ChatPerm::CONNECT_VOICE, 1_i64 << 25);
    assert_eq!(ChatPerm::SPEAK, 1_i64 << 26);
    assert_eq!(ChatPerm::STREAM_VIDEO, 1_i64 << 27);
    assert_eq!(ChatPerm::SHARE_SCREEN, 1_i64 << 28);
    assert_eq!(ChatPerm::MUTE_VOICE_MEMBERS, 1_i64 << 29);
    assert_eq!(ChatPerm::DEAFEN_VOICE_MEMBERS, 1_i64 << 30);
    assert_eq!(ChatPerm::MOVE_VOICE_MEMBERS, 1_i64 << 31);
    assert_eq!(ChatPerm::PRIORITY_SPEAKER, 1_i64 << 32);
    assert_eq!(ChatPerm::REQUEST_TO_SPEAK, 1_i64 << 33);
    assert_eq!(ChatPerm::MANAGE_STAGE, 1_i64 << 34);
    assert_eq!(ChatPerm::MANAGE_VOICE_CHANNEL, 1_i64 << 35);

    let editor_bits = ChatPerm::editor_rows().iter().map(|row| row.mask).collect::<HashSet<i64>>();
    assert!(editor_bits.contains(&ChatPerm::CONNECT_VOICE));
    assert!(editor_bits.contains(&ChatPerm::SPEAK));
    assert!(editor_bits.contains(&ChatPerm::STREAM_VIDEO));
    assert!(editor_bits.contains(&ChatPerm::SHARE_SCREEN));
    assert!(editor_bits.contains(&ChatPerm::MUTE_VOICE_MEMBERS));
    assert!(editor_bits.contains(&ChatPerm::DEAFEN_VOICE_MEMBERS));
    assert!(editor_bits.contains(&ChatPerm::MOVE_VOICE_MEMBERS));
    assert!(editor_bits.contains(&ChatPerm::PRIORITY_SPEAKER));
    assert!(editor_bits.contains(&ChatPerm::REQUEST_TO_SPEAK));
    assert!(editor_bits.contains(&ChatPerm::MANAGE_STAGE));
    assert!(editor_bits.contains(&ChatPerm::MANAGE_VOICE_CHANNEL));
}

#[test]
fn voice_connect_gate_is_scoped_and_fails_closed_without_cache() {
    assert_eq!(ChatVoiceConnectGatePolicy::decision(None), ChatVoiceConnectGateDecision::RefreshRequired);
    assert_eq!(ChatVoiceConnectGatePolicy::decision(Some(0)), ChatVoiceConnectGateDecision::Denied);
    assert_eq!(
        ChatVoiceConnectGatePolicy::decision(Some(ChatPerm::CONNECT_VOICE)),
        ChatVoiceConnectGateDecision::Allowed
    );
    assert_eq!(
        ChatVoiceConnectGatePolicy::decision(Some(ChatPerm::ADMINISTRATOR)),
        ChatVoiceConnectGateDecision::Allowed
    );
}

#[test]
fn channel_management_policy_matches_kind_specific_backend_authorization() {
    assert_eq!(
        ChatChannelManagementPolicy::allowed_destination_kinds(None, ChatPerm::MANAGE_TOPICS),
        vec![ChatChannelKind::Text]
    );
    assert_eq!(
        ChatChannelManagementPolicy::allowed_destination_kinds(None, ChatPerm::MANAGE_VOICE_CHANNEL),
        vec![ChatChannelKind::Voice, ChatChannelKind::Stage]
    );
    assert_eq!(
        ChatChannelManagementPolicy::allowed_destination_kinds(None, ChatPerm::ADMINISTRATOR),
        ChatChannelKind::ALL.to_vec()
    );

    for source in ChatChannelKind::ALL {
        assert_eq!(
            ChatChannelManagementPolicy::allowed_destination_kinds(Some(source), ChatPerm::ADMINISTRATOR),
            vec![source]
        );
        for destination in ChatChannelKind::ALL.into_iter().filter(|destination| *destination != source) {
            assert!(!ChatChannelManagementPolicy::can_mutate(Some(source), destination, ChatPerm::ADMINISTRATOR));
        }
    }
    assert!(
        ChatChannelManagementPolicy::allowed_destination_kinds(Some(ChatChannelKind::Voice), ChatPerm::MANAGE_TOPICS)
            .is_empty()
    );
}

#[test]
fn group_voice_settings_endpoints_stay_inside_rise_engine_catalogue() {
    let read = RiseConversationEndpoints::endpoint("chat", "get_group_voice_channel").expect("the read is catalogued");
    let update =
        RiseConversationEndpoints::endpoint("chat", "update_group_voice_channel").expect("the update is catalogued");

    assert_eq!(read.descriptor().replay_policy, RiseWireReplayPolicy::ReadOnly);
    assert_eq!(update.descriptor().replay_policy, RiseWireReplayPolicy::Never);
}
