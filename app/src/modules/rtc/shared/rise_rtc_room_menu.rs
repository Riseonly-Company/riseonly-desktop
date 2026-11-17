use gpui::App;
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::shared::try_read;

use crate::modules::chat::pages::chats::chat::chat_roles::forum_member_grouping::ForumMemberGrouping;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatPerm, ChatPermExt};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::rtc::engine::rise_rtc_engine_models::{RiseRTCParticipant, RiseRTCParticipantRole};
use crate::modules::rtc::engine::rise_rtc_listener_volume_presentation::RiseRTCListenerVolume;
use crate::modules::rtc::stores::rtc::rtc_services::rtc_services::RTCServicesStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// single description of what may be done to a live room member
/// so the grid tile and the stage roster cant drift into two menus
/// answers which entries only, permission lives in the capabilities resolved once per screen
pub enum RiseRTCRoomMenu {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseRTCRoomMenuAction {
    ToggleMuteForMe,
    ResetVolume,
    PromoteToSpeaker,
    MoveToAudience,
    ToggleMute,
    ToggleDeafen,
    Kick,
}

impl RiseRTCRoomMenuAction {
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::ToggleMuteForMe => "rtc.mute-for-me",
            Self::ResetVolume => "rtc.volume-reset",
            Self::PromoteToSpeaker => "rtc.promote",
            Self::MoveToAudience => "rtc.audience",
            Self::ToggleMute => "rtc.mute",
            Self::ToggleDeafen => "rtc.deafen",
            Self::Kick => "rtc.kick",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        match raw_value {
            "rtc.mute-for-me" => Some(Self::ToggleMuteForMe),
            "rtc.volume-reset" => Some(Self::ResetVolume),
            "rtc.promote" => Some(Self::PromoteToSpeaker),
            "rtc.audience" => Some(Self::MoveToAudience),
            "rtc.mute" => Some(Self::ToggleMute),
            "rtc.deafen" => Some(Self::ToggleDeafen),
            "rtc.kick" => Some(Self::Kick),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseRTCRoomMenuCapabilities {
    pub can_manage_stage: bool,
    pub can_mute: bool,
    pub can_deafen: bool,
    pub can_remove: bool,
    pub chat_id: Option<String>,
}

impl RiseRTCRoomMenuCapabilities {
    pub fn is_empty(&self) -> bool {
        !self.can_manage_stage && !self.can_mute && !self.can_deafen && !self.can_remove
    }
}

impl RiseRTCRoomMenu {
    fn outranks_viewer(user_id: &str, chat_id: Option<&str>, cx: &mut App) -> bool {
        let Some(chat_id) = chat_id.filter(|chat_id| !chat_id.is_empty()) else {
            return false;
        };
        if try_read::<ChatServicesStore>(cx).is_none() {
            return false;
        }
        let services = ChatServicesStore::shared(cx);
        if let Some(member) = services.update(cx, |services, cx| services.member(chat_id, user_id, cx)) {
            return !ForumMemberGrouping::can_moderate(&member, chat_id, cx);
        }
        let Some(chat) = services.read(cx).chat_info(chat_id, cx) else {
            return false;
        };
        if ChatPerm::is_owner_role(chat.role.as_deref()) {
            return false;
        }
        let Some(creator) = chat.creator_info_decoded() else {
            return false;
        };
        if creator.id != user_id {
            return false;
        }
        creator.role.as_deref().is_none_or(|role| ChatPerm::is_owner_role(Some(role)))
    }

    /// true when actions must drop the moderation rows for this participant
    /// roster cache needs a mutable App which the menu provider cant have
    /// so the row builder answers this first and an observed roster change rebuilds the row
    pub fn outranked(
        participant: &RiseRTCParticipant,
        requested: &RiseRTCRoomMenuCapabilities,
        cx: &mut App,
    ) -> bool {
        !requested.is_empty() && Self::outranks_viewer(&participant.user_id, requested.chat_id.as_deref(), cx)
    }

    /// personal rows come first and always offered, hearing someone isnt a moderator power
    /// capabilities gates whatever acts on the room, not on this listener
    /// stores come from the call screen, read not created
    pub fn actions(
        participant: &RiseRTCParticipant,
        requested: &RiseRTCRoomMenuCapabilities,
        outranked: bool,
        cx: &App,
    ) -> Vec<HoldContextMenuAction> {
        let mut actions: Vec<HoldContextMenuAction> = Vec::new();
        let outranked_capabilities =
            RiseRTCRoomMenuCapabilities { chat_id: requested.chat_id.clone(), ..Default::default() };
        let capabilities = if outranked { &outranked_capabilities } else { requested };

        let is_invented =
            try_read::<GuideSandbox>(cx).is_some_and(|sandbox| sandbox.owns_user_id(&participant.user_id));
        if !is_invented && let Some(service) = try_read::<RTCServicesStore>(cx) {
            let muted_for_me = service.is_muted_for_me(&participant.user_id, cx);
            actions.push(
                HoldContextMenuAction::new(
                    RiseRTCRoomMenuAction::ToggleMuteForMe.raw_value(),
                    if muted_for_me { tr("rtc_volume_unmute_for_me") } else { tr("rtc_volume_mute_for_me") },
                )
                .system_image(if muted_for_me { "speaker.wave.2.fill" } else { "speaker.slash.fill" }),
            );

            if service.has_explicit_listener_volume(&participant.user_id, cx)
                && service.listener_volume_percent(&participant.user_id, cx) != RiseRTCListenerVolume::DEFAULT_PERCENT
                && !try_read::<UserServicesStore>(cx)
                    .is_some_and(|users| users.is_totally_blocked(&participant.user_id, cx))
            {
                actions.push(
                    HoldContextMenuAction::new(RiseRTCRoomMenuAction::ResetVolume.raw_value(), tr("rtc_volume_reset"))
                        .system_image("arrow.counterclockwise"),
                );
            }
        }

        if capabilities.can_manage_stage {
            if participant.role == RiseRTCParticipantRole::Listener
                || participant.role == RiseRTCParticipantRole::Unspecified
            {
                actions.push(
                    HoldContextMenuAction::new(
                        RiseRTCRoomMenuAction::PromoteToSpeaker.raw_value(),
                        tr("rtc_promote_speaker"),
                    )
                    .system_image("person.badge.plus")
                    .section_break(true)
                    .guide_anchor(RiseGuideAnchor::RtcMenuPromoteSpeaker),
                );
            } else {
                actions.push(
                    HoldContextMenuAction::new(RiseRTCRoomMenuAction::MoveToAudience.raw_value(), tr("rtc_move_audience"))
                        .system_image("person.badge.minus")
                        .section_break(true),
                );
            }
        }

        if capabilities.can_mute {
            actions.push(
                HoldContextMenuAction::new(
                    RiseRTCRoomMenuAction::ToggleMute.raw_value(),
                    if participant.server_muted { tr("rtc_allow_unmute") } else { tr("rtc_force_mute") },
                )
                .system_image(if participant.server_muted { "mic.fill" } else { "mic.slash.fill" })
                .section_break(true)
                .guide_anchor((!participant.server_muted).then_some(RiseGuideAnchor::RtcMenuForceMute)),
            );
        }

        if capabilities.can_deafen {
            actions.push(
                HoldContextMenuAction::new(
                    RiseRTCRoomMenuAction::ToggleDeafen.raw_value(),
                    if participant.server_deafened { tr("rtc_allow_undeafen") } else { tr("rtc_force_deafen") },
                )
                .system_image(if participant.server_deafened { "ear.fill" } else { "ear.badge.waveform" })
                .section_break(!capabilities.can_mute),
            );
        }

        if capabilities.can_remove {
            let section_break = !actions.is_empty();
            actions.push(
                HoldContextMenuAction::new(RiseRTCRoomMenuAction::Kick.raw_value(), tr("rtc_remove_participant"))
                    .system_image("person.crop.circle.badge.xmark")
                    .destructive(true)
                    .section_break(section_break),
            );
        }

        actions
    }
}
