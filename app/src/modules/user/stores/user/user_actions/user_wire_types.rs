use gpui::App;
use rise_canon::{RisePrivacyAudience, RisePrivacyRule, rise_proto, rise_request, rise_response, rise_view};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use super::user_types::{GoalItem, GoalSubgoalItem, PlanItem, ProfileModel, ProfileSocialLink, SubscriptionStatus, UserListEntry};
use super::user_wire_types_hooks::user_wire_types_hooks;
use crate::core::lib::canon_privacy::{RisePrivacyAudienceCanon, RisePrivacyParsedAudience, RisePrivacyRuleCanon};
use crate::core::ui::emoji_picker_ui::emoji_picker_catalog::EmojiPickerCatalog;
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, AuthorInfoMore};

pub type UpdateProfileMoreBody = rise_request::user::update_profile::More;
pub type UpdateProfileSocialLinkBody = rise_request::user::update_profile::more::SocialLinksElement;

pub type UserProfileSubscriptionDTO = rise_proto::user::UserProfileWithSubscriptionResponse;
pub type UserMoreWireDTO = rise_proto::user::MoreDataResponse;
pub type UserProfileSocialLinkWireDTO = rise_proto::user::ProfileSocialLink;
pub type UserGoalWireEntry = rise_proto::user::UserGoalEntry;
pub type UserPlanWireEntry = rise_proto::user::UserPlanEntry;
pub type SyncProfileGoalsGatewayDTO = rise_response::user::SyncProfileGoals;
pub type SyncProfilePlansGatewayDTO = rise_response::user::SyncProfilePlans;
pub type UserGetProfileByTagGatewayDTO = rise_response::user::GetProfileByTag;
pub type FriendRequestGatewayDTO = rise_proto::user::FriendRequestDto;
pub type FriendRequestWireDTO = rise_proto::user::FriendRequestDto;
pub type PrivacySettingsWireDTO = rise_proto::user::PrivacySettingsResponse;
pub type PrivacyExceptionWireDTO = rise_proto::user::PrivacyExceptionResponse;
pub type FriendListGatewayDTO = rise_view::user::Friends;
pub type UnfollowUserGatewayDTO = rise_proto::user::UnfollowUserGatewayResponse;
pub type ChatMuteOverridesGatewayDTO = rise_proto::user::GetChatMuteOverridesGatewayResponse;

pub enum ProfileViewerPrivacy {}

impl ProfileViewerPrivacy {
    pub fn closes(rule: Option<&str>) -> bool {
        matches!(
            RisePrivacyAudience::parse(rule),
            RisePrivacyParsedAudience::Audience(RisePrivacyAudience::None)
        )
    }

    pub fn closes_gifts(owner_user_id: &str, cx: &mut App) -> bool {
        let Some(hooks) = user_wire_types_hooks() else {
            return false;
        };
        let Some(payload) = (hooks.foreign_profile_payload)(owner_user_id, cx) else {
            return false;
        };
        let owner_block_is_empty = payload
            .resolved_owner_block_scope()
            .map(|scope| trimming_whitespaces_and_newlines(scope).is_empty())
            .unwrap_or(true);
        if !owner_block_is_empty {
            return false;
        }
        Self::closes(
            payload
                .profile
                .as_ref()
                .and_then(|profile| profile.privacy_settings.as_ref())
                .map(|settings| settings.gifts_rule.as_str()),
        )
    }

    pub fn hides_subs_count(rule: Option<&str>, count_hidden: Option<bool>) -> bool {
        count_hidden == Some(true) && RisePrivacyRule::Subs.audience(rule) != RisePrivacyAudience::All
    }
}

pub trait UserProfileSocialLinkWireDTOExt {
    fn as_profile_social_link(&self) -> ProfileSocialLink;
}

impl UserProfileSocialLinkWireDTOExt for UserProfileSocialLinkWireDTO {
    fn as_profile_social_link(&self) -> ProfileSocialLink {
        ProfileSocialLink { title: self.title.clone(), url: self.url.clone() }
    }
}

pub trait UserMoreWireDTOExt {
    fn profile_social_links(&self) -> Vec<ProfileSocialLink>;
}

impl UserMoreWireDTOExt for UserMoreWireDTO {
    fn profile_social_links(&self) -> Vec<ProfileSocialLink> {
        if !self.social_links.is_empty() {
            return self.social_links.iter().map(|link| link.as_profile_social_link()).collect();
        }
        self.media_links.iter().map(|url| ProfileSocialLink { title: String::new(), url: url.clone() }).collect()
    }
}

pub trait UserGoalWireEntryExt {
    fn as_goal_item(&self) -> GoalItem;
}

impl UserGoalWireEntryExt for UserGoalWireEntry {
    fn as_goal_item(&self) -> GoalItem {
        GoalItem {
            id: self.id.clone(),
            title: self.title.clone(),
            progress: self.progress as f64 / 100.0,
            description: self.description.clone(),
            is_checklist: !self.steps.is_empty(),
            subgoals: self
                .steps
                .iter()
                .map(|step| GoalSubgoalItem { id: step.id.clone(), title: step.title.clone(), done: step.done })
                .collect(),
        }
    }
}

pub trait UserPlanWireEntryExt {
    fn as_plan_item(&self) -> PlanItem;
}

impl UserPlanWireEntryExt for UserPlanWireEntry {
    fn as_plan_item(&self) -> PlanItem {
        PlanItem {
            id: self.id.clone(),
            title: self.title.clone(),
            date_ms: self.date_ms,
            description: self.description.clone(),
        }
    }
}

pub trait RiseUserProfileRow {
    fn id(&self) -> &str;
    fn name(&self) -> Option<&str>;
    fn phone(&self) -> &str;
    fn tag(&self) -> &str;
    fn alias_tags(&self) -> &[String];
    fn is_premium(&self) -> bool;
    fn is_official(&self) -> bool;
    fn gender(&self) -> &str;
    fn role(&self) -> &str;
    fn friends_count(&self) -> i64;
    fn subscribers_count(&self) -> i64;
    fn subs_count(&self) -> i64;
    fn posts_count(&self) -> i64;
    fn premium_badge_emoji(&self) -> Option<&str>;
    fn name_color(&self) -> Option<&str>;
    fn more(&self) -> Option<&rise_proto::user::MoreDataResponse>;
    fn is_friend(&self) -> Option<bool>;
    fn is_subbed(&self) -> Option<bool>;
    fn friend_request_id(&self) -> Option<i64>;
    fn is_bot(&self) -> bool;
    fn bot_owner_id(&self) -> Option<&str>;
    fn bot_is_active(&self) -> bool;
    fn is_deleted(&self) -> bool;
    fn privacy_settings(&self) -> Option<&rise_proto::user::PrivacySettingsResponse>;
    fn row_is_phone_public(&self) -> bool;
    fn row_is_subscriber(&self) -> bool;
    fn row_quick_reaction(&self) -> Option<&str>;
    fn row_subscription_status(&self) -> Option<&str>;

    fn as_profile_model(&self) -> ProfileModel {
        let m = self.more();
        let sub_raw = self
            .row_subscription_status()
            .map(|status| trimming_whitespaces_and_newlines(status).to_lowercase())
            .unwrap_or_default();
        let sub = if sub_raw.is_empty() {
            SubscriptionStatus::Inactive
        } else {
            SubscriptionStatus::from_raw(&sub_raw).unwrap_or(SubscriptionStatus::Inactive)
        };
        let logo = m.map(|more| more.logo.clone()).unwrap_or_default();
        let description = m.map(|more| more.description.clone()).unwrap_or_default();
        let banner = m.map(|more| more.banner.clone()).unwrap_or_default();
        let phone = self.phone();

        ProfileModel {
            id: self.id().to_owned(),
            name: self.name().unwrap_or("").to_owned(),
            tag: self.tag().to_owned(),
            alias_tags: self.alias_tags().to_vec(),
            description: if description.is_empty() { None } else { Some(description) },
            more: AuthorInfoMore {
                p_lang: m.map(|more| more.p_lang.clone()).unwrap_or_default(),
                who: m.map(|more| more.who.clone()).unwrap_or_default(),
                role: self.role().to_owned(),
                logo: logo.clone(),
                is_premium: self.is_premium(),
                is_official: self.is_official(),
                phone: if phone.is_empty() { None } else { Some(phone.to_owned()) },
                premium_badge_emoji: self.premium_badge_emoji().map(str::to_owned),
                name_color: self.name_color().map(str::to_owned),
            },
            posts_count: self.posts_count(),
            subs_count: self.subs_count(),
            subs_count_hidden: ProfileViewerPrivacy::hides_subs_count(
                self.privacy_settings().map(|settings| settings.subs_rule.as_str()),
                self.privacy_settings().map(|settings| settings.subs_count_hidden),
            ),
            subscribers_count: self.subscribers_count(),
            friends_count: self.friends_count(),
            subscription_status: sub,
            cover_image_url: if banner.is_empty() { None } else { Some(banner) },
            avatar_url: if logo.is_empty() { None } else { Some(logo) },
            profile_photos_count: m.map(|more| more.profile_photos_count).unwrap_or(0),
            hb: m.and_then(|more| more.hb.clone()),
            gender: Some(self.gender().to_owned()),
            level: m.map(|more| more.level),
            streak: m.map(|more| more.streak),
            rating: m.map(|more| more.rating),
            is_subbed: self.is_subbed().unwrap_or(false),
            is_subscriber: self.row_is_subscriber(),
            is_friend: self.is_friend().unwrap_or(false),
            friend_request_id: self.friend_request_id(),
            social_links: m.map(|more| more.profile_social_links()).unwrap_or_default(),
            quick_reaction: EmojiPickerCatalog::normalized_selectable_emoji_or_custom_ref(self.row_quick_reaction())
                .unwrap_or_else(|| ProfileModel::DEFAULT_QUICK_REACTION.to_owned()),
            is_phone_public: self.row_is_phone_public(),
            is_bot: self.is_bot(),
            bot_owner_id: self.bot_owner_id().map(str::to_owned),
            bot_is_active: self.bot_is_active(),
            is_deleted: self.is_deleted(),
            ..ProfileModel::default()
        }
    }
}

macro_rules! rise_user_profile_row_fields {
    () => {
        fn id(&self) -> &str {
            &self.id
        }

        fn name(&self) -> Option<&str> {
            self.name.as_deref()
        }

        fn phone(&self) -> &str {
            &self.phone
        }

        fn tag(&self) -> &str {
            &self.tag
        }

        fn alias_tags(&self) -> &[String] {
            &self.alias_tags
        }

        fn is_premium(&self) -> bool {
            self.is_premium
        }

        fn is_official(&self) -> bool {
            self.is_official
        }

        fn gender(&self) -> &str {
            &self.gender
        }

        fn role(&self) -> &str {
            &self.role
        }

        fn friends_count(&self) -> i64 {
            self.friends_count
        }

        fn subscribers_count(&self) -> i64 {
            self.subscribers_count
        }

        fn subs_count(&self) -> i64 {
            self.subs_count
        }

        fn posts_count(&self) -> i64 {
            self.posts_count
        }

        fn premium_badge_emoji(&self) -> Option<&str> {
            self.premium_badge_emoji.as_deref()
        }

        fn name_color(&self) -> Option<&str> {
            self.name_color.as_deref()
        }

        fn more(&self) -> Option<&rise_proto::user::MoreDataResponse> {
            self.more.as_ref()
        }

        fn is_friend(&self) -> Option<bool> {
            self.is_friend
        }

        fn is_subbed(&self) -> Option<bool> {
            self.is_subbed
        }

        fn friend_request_id(&self) -> Option<i64> {
            self.friend_request_id
        }

        fn is_bot(&self) -> bool {
            self.is_bot
        }

        fn bot_owner_id(&self) -> Option<&str> {
            self.bot_owner_id.as_deref()
        }

        fn bot_is_active(&self) -> bool {
            self.bot_is_active
        }

        fn privacy_settings(&self) -> Option<&rise_proto::user::PrivacySettingsResponse> {
            self.privacy_settings.as_ref()
        }

        fn is_deleted(&self) -> bool {
            self.is_deleted
        }
    };
}

impl RiseUserProfileRow for rise_proto::user::UserProfileWithSubscriptionResponse {
    rise_user_profile_row_fields!();

    fn row_is_phone_public(&self) -> bool {
        self.is_phone_public
    }

    fn row_is_subscriber(&self) -> bool {
        false
    }

    fn row_quick_reaction(&self) -> Option<&str> {
        self.quick_reaction.as_deref()
    }

    fn row_subscription_status(&self) -> Option<&str> {
        self.subscription_status.as_deref()
    }
}

impl RiseUserProfileRow for rise_proto::user::BaseUserProfileResponse {
    rise_user_profile_row_fields!();

    fn row_is_phone_public(&self) -> bool {
        self.is_phone_public.unwrap_or(false)
    }

    fn row_is_subscriber(&self) -> bool {
        self.is_subscriber.unwrap_or(false)
    }

    fn row_quick_reaction(&self) -> Option<&str> {
        None
    }

    fn row_subscription_status(&self) -> Option<&str> {
        None
    }
}

pub trait UserGetProfileByTagGatewayDTOExt {
    fn resolved_owner_block_scope(&self) -> Option<&str>;
    fn resolved_viewer_block_scope(&self) -> Option<&str>;
    fn resolved_viewer_has_blocked(&self) -> Option<bool>;
    fn as_profile_model(&self) -> Option<ProfileModel>;
}

impl UserGetProfileByTagGatewayDTOExt for UserGetProfileByTagGatewayDTO {
    fn resolved_owner_block_scope(&self) -> Option<&str> {
        self.profile.as_ref().and_then(|profile| profile.owner_block_scope.as_deref())
    }

    fn resolved_viewer_block_scope(&self) -> Option<&str> {
        self.profile.as_ref().and_then(|profile| profile.viewer_block_scope.as_deref())
    }

    fn resolved_viewer_has_blocked(&self) -> Option<bool> {
        self.profile.as_ref().and_then(|profile| profile.viewer_has_blocked)
    }

    fn as_profile_model(&self) -> Option<ProfileModel> {
        self.profile.as_ref().map(RiseUserProfileRow::as_profile_model)
    }
}

pub type FriendUserWireDTO = rise_proto::user::FriendUserDto;
pub type FriendItemWireDTO = rise_proto::user::FriendDto;
pub type UserSubscriptionItemWireDTO = rise_proto::user::UserSubscriptionDto;
pub type UserSubsListGatewayDTO = rise_view::user::Subs;

pub trait FriendUserDtoExt {
    fn as_author_info(&self) -> AuthorInfo;
}

impl FriendUserDtoExt for rise_proto::user::FriendUserDto {
    fn as_author_info(&self) -> AuthorInfo {
        let more = self.more.as_ref();
        AuthorInfo {
            id: Some(self.id.clone()),
            name: self.name.clone(),
            tag: self.tag.clone(),
            more: AuthorInfoMore {
                p_lang: more.map(|more| more.p_lang.clone()).unwrap_or_default(),
                who: more.map(|more| more.who.clone()).unwrap_or_default(),
                role: String::new(),
                logo: more.map(|more| more.logo.clone()).unwrap_or_default(),
                is_premium: self.is_premium,
                is_official: self.is_official,
                phone: None,
                premium_badge_emoji: self.premium_badge_emoji.clone(),
                ..AuthorInfoMore::default()
            },
            ..AuthorInfo::default()
        }
    }
}

pub trait FriendDtoExt {
    fn as_user_list_entry(&self) -> Option<UserListEntry>;
}

impl FriendDtoExt for rise_proto::user::FriendDto {
    fn as_user_list_entry(&self) -> Option<UserListEntry> {
        let user = self.user.as_ref()?;
        Some(UserListEntry {
            entry_key: Some(self.entry_key.clone()),
            entry_id: Some(self.id),
            author: user.as_author_info(),
            owner_is_friend: true,
            viewer_is_subbed: Some(self.viewer_is_subbed),
            viewer_is_subscriber: Some(self.viewer_is_subscriber),
            viewer_is_friend: Some(self.viewer_is_friend),
            viewer_friend_request_id: self.viewer_friend_request_id,
            created_at: Some(self.created_at.clone()),
        })
    }
}

pub trait UserSubscriptionDtoExt {
    fn as_user_list_entry(&self) -> Option<UserListEntry>;
}

impl UserSubscriptionDtoExt for rise_proto::user::UserSubscriptionDto {
    fn as_user_list_entry(&self) -> Option<UserListEntry> {
        let user = self.user.as_ref()?;
        Some(UserListEntry {
            entry_key: Some(self.entry_key.clone()),
            entry_id: self.id.parse::<i64>().ok(),
            author: user.as_author_info(),
            owner_is_friend: self.is_friend,
            viewer_is_subbed: Some(self.viewer_is_subbed),
            viewer_is_subscriber: Some(self.viewer_is_subscriber),
            viewer_is_friend: Some(self.viewer_is_friend),
            viewer_friend_request_id: self.viewer_friend_request_id,
            created_at: Some(self.created_at.clone()),
        })
    }
}
