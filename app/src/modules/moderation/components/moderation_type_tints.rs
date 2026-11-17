use gpui::Hsla;
use rise_theme::AppTheme;
use rise_ui::rise_symbol::RiseSymbol;

use super::moderation_kit_ui::ModerationTint;
use crate::app::router::navigation_types::RootRoute;
use crate::modules::chat::pages::chats::chat::chat_profile::chat_tile_color::ChatTileColor;
use crate::modules::moderation::engine::rise_moderation_wire::ReportRowDTO;
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AppealDecision, AppealStatus, ModerationAuditAction, ReportReason, ReportStatus, ReportTargetType, ResolveAction,
    UserBlockType,
};

pub trait ReportReasonTintExt {
    fn tint(self) -> Hsla;
}

impl ReportReasonTintExt for ReportReason {
    fn tint(self) -> Hsla {
        ModerationTint::reason(self)
    }
}

/// icon name and tint for a report target type
pub trait ReportTargetTypeTintExt {
    fn icon_name(self) -> &'static str;
    fn tint(self) -> Hsla;
}

impl ReportTargetTypeTintExt for ReportTargetType {
    fn icon_name(self) -> &'static str {
        match self {
            Self::Post => "doc.text",
            Self::Comment => "text.bubble",
            Self::User => "person",
            Self::Chat => "bubble.left.and.bubble.right",
            Self::Vacancy => "briefcase",
            Self::Message => "envelope",
            Self::MusicTrack => "music.note",
            Self::Artist => RiseSymbol::ARTIST,
            Self::Story => "circle.dashed.inset.filled",
            Self::Bot => "cpu",
            Self::Other => "ellipsis.circle",
        }
    }

    fn tint(self) -> Hsla {
        ModerationTint::target(self)
    }
}

/// accent color of a user block type
pub trait UserBlockTypeTintExt {
    fn accent_color(self, theme: &AppTheme) -> Hsla;
}

impl UserBlockTypeTintExt for UserBlockType {
    fn accent_color(self, _theme: &AppTheme) -> Hsla {
        ModerationTint::block(self)
    }
}

pub trait AppealStatusTintExt {
    fn color(self, theme: &AppTheme) -> Hsla;
}

impl AppealStatusTintExt for AppealStatus {
    fn color(self, theme: &AppTheme) -> Hsla {
        ModerationTint::appeal(self, theme)
    }
}

pub trait AppealDecisionTintExt {
    fn color(self, theme: &AppTheme) -> Hsla;
}

impl AppealDecisionTintExt for AppealDecision {
    fn color(self, theme: &AppTheme) -> Hsla {
        match self {
            Self::Accept => theme.semantic.success_100,
            Self::Reject => theme.semantic.error_100,
        }
    }
}

pub trait ResolveActionTintExt {
    fn tile_color(self) -> Hsla;
}

impl ResolveActionTintExt for ResolveAction {
    fn tile_color(self) -> Hsla {
        ModerationTint::action(self)
    }
}

pub trait ModerationAuditActionTintExt {
    fn tint(self, theme: &AppTheme) -> Hsla;
}

impl ModerationAuditActionTintExt for ModerationAuditAction {
    fn tint(self, theme: &AppTheme) -> Hsla {
        match self {
            Self::Dismiss
            | Self::AppealAccept
            | Self::RestoreContent
            | Self::AdminUnblock
            | Self::AdminUnblockIp
            | Self::ApproveModerator
            | Self::ApproveSuperModer => ChatTileColor::green(),
            Self::Warn | Self::Mute | Self::CreateAppeal | Self::ReopenReport | Self::UpdatePermissions => {
                ChatTileColor::orange()
            }
            Self::DeleteContent
            | Self::BlockUser
            | Self::Both
            | Self::Block
            | Self::AdminBlock
            | Self::AdminBlockIp
            | Self::AppealReject
            | Self::RejectModerator
            | Self::RemoveModerator => ChatTileColor::red(),
            Self::AddNote | Self::ResolveReport => theme.primary._100,
        }
    }
}

pub enum ReportStatusStyle {}

impl ReportStatusStyle {
    pub fn color(status: ReportStatus, theme: &AppTheme) -> Hsla {
        ModerationTint::status(status, theme)
    }
}

impl ReportRowDTO {
    pub fn open_route(&self) -> Option<RootRoute> {
        match self.resolved_type() {
            ReportTargetType::Post | ReportTargetType::Comment => {
                let post_id = self.resolved_post_id()?;
                Some(RootRoute::PostDetail { post_id: Some(post_id.to_string()), origin: None })
            }
            ReportTargetType::User => {
                Some(RootRoute::UserPage { user_id: self.reported_user_id(), tag: self.reported_user_tag() })
            }
            ReportTargetType::Chat | ReportTargetType::Message | ReportTargetType::Bot => self.chat_route(),
            ReportTargetType::MusicTrack => {
                let track_id = self.track_id.as_ref().filter(|id| !id.is_empty())?;
                Some(RootRoute::StudioTrackDetail { track_id: track_id.clone() })
            }
            ReportTargetType::Artist => {
                let artist_id = self.artist_id.as_ref().filter(|id| !id.is_empty())?;
                Some(RootRoute::MusicArtistProfile { artist_id: artist_id.clone() })
            }
            ReportTargetType::Vacancy => Some(RootRoute::VacancyDetail { vacancy_id: self.vacancy_id? }),
            ReportTargetType::Story => {
                let author = self.reported_user_id().filter(|id| !id.is_empty())?;
                Some(RootRoute::UserPage { user_id: Some(author), tag: self.reported_user_tag() })
            }
            ReportTargetType::Other => None,
        }
    }

    fn chat_route(&self) -> Option<RootRoute> {
        let chat_id = self.chat_id.as_ref().filter(|id| !id.is_empty())?;
        Some(RootRoute::Chat { chat_id: Some(chat_id.clone()) })
    }

    fn reported_user_tag(&self) -> Option<String> {
        self.reported_user.as_ref().and_then(|user| user.tag.clone())
    }
}
