use std::collections::HashMap;

use rise_canon::rise_key::notify;
use rise_canon::rise_request::notify::UpdateProfileNotificationSettings;
use rise_i18n::{FormatArg, format_printf, tr};
use serde_json::{Map, Value};

pub type ProfileNotificationSettingsGatewayDTO = rise_canon::rise_response::notify::GetProfileNotificationSettings;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProfileNotificationSettingsModel {
    pub target_user_id: String,
    pub mute_all_profiles: bool,
    pub has_override: bool,
    pub posts_enabled: bool,
    pub stories_enabled: bool,
    pub threads_enabled: bool,
    pub reels_enabled: bool,
    pub comment_mentions_enabled: bool,
    pub same_post_likes_enabled: bool,
    pub repost_likes_enabled: bool,
    pub comment_likes_enabled: bool,
    pub comment_replies_enabled: bool,
}

impl Default for ProfileNotificationSettingsModel {
    fn default() -> Self {
        Self {
            target_user_id: String::new(),
            mute_all_profiles: false,
            has_override: false,
            posts_enabled: true,
            stories_enabled: true,
            threads_enabled: true,
            reels_enabled: true,
            comment_mentions_enabled: true,
            same_post_likes_enabled: true,
            repost_likes_enabled: true,
            comment_likes_enabled: true,
            comment_replies_enabled: true,
        }
    }
}

impl ProfileNotificationSettingsModel {
    pub fn value(&self, scope: ProfileNotificationScope) -> bool {
        match scope {
            ProfileNotificationScope::Posts => self.posts_enabled,
            ProfileNotificationScope::Stories => self.stories_enabled,
            ProfileNotificationScope::Threads => self.threads_enabled,
            ProfileNotificationScope::Reels => self.reels_enabled,
            ProfileNotificationScope::CommentMentions => self.comment_mentions_enabled,
            ProfileNotificationScope::CommentLikes => self.comment_likes_enabled,
            ProfileNotificationScope::CommentReplies => self.comment_replies_enabled,
            ProfileNotificationScope::SamePostLikes => self.same_post_likes_enabled,
            ProfileNotificationScope::RepostLikes => self.repost_likes_enabled,
        }
    }

    pub fn set(&mut self, scope: ProfileNotificationScope, enabled: bool) {
        match scope {
            ProfileNotificationScope::Posts => self.posts_enabled = enabled,
            ProfileNotificationScope::Stories => self.stories_enabled = enabled,
            ProfileNotificationScope::Threads => self.threads_enabled = enabled,
            ProfileNotificationScope::Reels => self.reels_enabled = enabled,
            ProfileNotificationScope::CommentMentions => self.comment_mentions_enabled = enabled,
            ProfileNotificationScope::CommentLikes => self.comment_likes_enabled = enabled,
            ProfileNotificationScope::CommentReplies => self.comment_replies_enabled = enabled,
            ProfileNotificationScope::SamePostLikes => self.same_post_likes_enabled = enabled,
            ProfileNotificationScope::RepostLikes => self.repost_likes_enabled = enabled,
        }
    }

    pub fn is_all_enabled(&self) -> bool {
        ProfileNotificationScope::ALL_CASES.into_iter().all(|scope| self.value(scope))
    }
}

pub trait ProfileNotificationSettingsGatewayModel {
    fn to_model(&self) -> ProfileNotificationSettingsModel;
}

impl ProfileNotificationSettingsGatewayModel for ProfileNotificationSettingsGatewayDTO {
    fn to_model(&self) -> ProfileNotificationSettingsModel {
        ProfileNotificationSettingsModel {
            target_user_id: self.target_user_id.clone(),
            mute_all_profiles: self.mute_all_profiles,
            has_override: self.has_override,
            posts_enabled: self.posts_enabled,
            stories_enabled: self.stories_enabled,
            threads_enabled: self.threads_enabled,
            reels_enabled: self.reels_enabled,
            comment_mentions_enabled: self.comment_mentions_enabled,
            same_post_likes_enabled: self.same_post_likes_enabled,
            repost_likes_enabled: self.repost_likes_enabled,
            comment_likes_enabled: self.comment_likes_enabled,
            comment_replies_enabled: self.comment_replies_enabled,
        }
    }
}

pub trait UpdateProfileNotificationSettingsPatch: Sized {
    fn from_patch(target_user_id: &str, patch: &Map<String, Value>) -> Self;
}

impl UpdateProfileNotificationSettingsPatch for UpdateProfileNotificationSettings {
    fn from_patch(target_user_id: &str, patch: &Map<String, Value>) -> Self {
        let mut body = Self { target_user_id: Some(target_user_id.to_owned()), ..Self::default() };
        for (key, value) in patch {
            let scope = ProfileNotificationScope::from_wire_key(key);
            let enabled = value.as_bool();
            debug_assert!(
                scope.is_some() && enabled.is_some(),
                "profile notification settings patch: unknown key {key}"
            );
            let (Some(scope), Some(enabled)) = (scope, enabled) else {
                continue;
            };
            let field = match scope {
                ProfileNotificationScope::Posts => &mut body.posts_enabled,
                ProfileNotificationScope::Stories => &mut body.stories_enabled,
                ProfileNotificationScope::Threads => &mut body.threads_enabled,
                ProfileNotificationScope::Reels => &mut body.reels_enabled,
                ProfileNotificationScope::CommentMentions => &mut body.comment_mentions_enabled,
                ProfileNotificationScope::CommentLikes => &mut body.comment_likes_enabled,
                ProfileNotificationScope::CommentReplies => &mut body.comment_replies_enabled,
                ProfileNotificationScope::SamePostLikes => &mut body.same_post_likes_enabled,
                ProfileNotificationScope::RepostLikes => &mut body.repost_likes_enabled,
            };
            *field = Some(enabled);
        }
        body
    }
}

/// wire_key is the canon key of the patch and of the response
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileNotificationScope {
    Posts,
    Stories,
    Threads,
    Reels,
    CommentMentions,
    CommentLikes,
    CommentReplies,
    SamePostLikes,
    RepostLikes,
}

impl ProfileNotificationScope {
    pub const ALL_CASES: [ProfileNotificationScope; 9] = [
        Self::Posts,
        Self::Stories,
        Self::Threads,
        Self::Reels,
        Self::CommentMentions,
        Self::CommentLikes,
        Self::CommentReplies,
        Self::SamePostLikes,
        Self::RepostLikes,
    ];

    pub const REGULAR: [ProfileNotificationScope; 7] = [
        Self::Posts,
        Self::Stories,
        Self::Threads,
        Self::Reels,
        Self::CommentMentions,
        Self::CommentLikes,
        Self::CommentReplies,
    ];

    pub const WATCH: [ProfileNotificationScope; 2] = [Self::SamePostLikes, Self::RepostLikes];

    /// the canon notification flag this row toggles
    pub fn wire_key(self) -> &'static str {
        match self {
            Self::Posts => notify::POSTS_ENABLED,
            Self::Stories => notify::STORIES_ENABLED,
            Self::Threads => notify::THREADS_ENABLED,
            Self::Reels => notify::REELS_ENABLED,
            Self::CommentMentions => notify::COMMENT_MENTIONS_ENABLED,
            Self::CommentLikes => notify::COMMENT_LIKES_ENABLED,
            Self::CommentReplies => notify::COMMENT_REPLIES_ENABLED,
            Self::SamePostLikes => notify::SAME_POST_LIKES_ENABLED,
            Self::RepostLikes => notify::REPOST_LIKES_ENABLED,
        }
    }

    pub fn from_wire_key(wire_key: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|scope| scope.wire_key() == wire_key)
    }

    pub fn id(self) -> &'static str {
        self.wire_key()
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Self::Posts => "profile_notify_scope_posts",
            Self::Stories => "profile_notify_scope_stories",
            Self::Threads => "profile_notify_scope_threads",
            Self::Reels => "riffs",
            Self::CommentMentions => "profile_notify_scope_comment_mentions",
            Self::CommentLikes => "profile_notify_scope_comment_likes",
            Self::CommentReplies => "profile_notify_scope_comment_replies",
            Self::SamePostLikes => "profile_notify_scope_same_post_likes",
            Self::RepostLikes => "profile_notify_scope_repost_likes",
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::RepostLikes => {
                let riffs = tr("riffs");
                format_printf(&tr(self.title_key()), &[FormatArg::from(&riffs)])
            }
            _ => tr(self.title_key()),
        }
    }
}

pub struct ProfileNotificationSettingsMutation {
    pub target_user_id: String,
    pub patch: Map<String, Value>,
    pub rollback_patch: Map<String, Value>,
    pub field_revisions: HashMap<String, u64>,
}

pub enum ProfileNotificationSettingsDebounce {}

impl ProfileNotificationSettingsDebounce {
    pub const GROUP_KEY: &'static str = "profile_notification_settings";
}
