use std::time::Duration;

use rise_engine::MethodDescriptor;
use serde::{Deserialize, Serialize};

pub const GET_MY_PROFILE: MethodDescriptor = MethodDescriptor::read("user", "get_my_profile");
pub const GET_PROFILE_BY_ID: MethodDescriptor = MethodDescriptor::read("user", "get_profile_by_id");
pub const GET_PROFILE_BY_TAG: MethodDescriptor =
    MethodDescriptor::read("user", "get_profile_by_tag");

// Following someone is a friend request the server auto-accepts into a follow;
// there is no `follow_user` verb. Not replayable: the response carries the
// created row, and a retry would answer about a request the first call made.
pub const SEND_FRIEND_REQUEST: MethodDescriptor =
    MethodDescriptor::mutation("user", "send_friend_request");
pub const UNFOLLOW_USER: MethodDescriptor =
    MethodDescriptor::mutation("user", "unfollow_user").with_timeout(Duration::from_secs(20));
pub const DELETE_FRIEND: MethodDescriptor = MethodDescriptor::mutation("user", "delete_friend");

#[derive(Clone, Debug, Serialize)]
pub struct GetMyProfileRequest {
    pub user_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GetProfileByIdRequest {
    pub profile_user_id: String,
    pub user_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GetProfileByTagRequest {
    pub tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SendFriendRequestRequest {
    pub user_id: String,
    pub receiver_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct UnfollowUserRequest {
    pub user_id: String,
    pub target_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeleteFriendRequest {
    pub user_id: String,
    pub friend_id: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct SocialLinkDto {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct GoalDto {
    pub id: Option<String>,
    pub title: String,
    pub progress: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct PlanDto {
    pub id: Option<String>,
    pub title: String,
    #[serde(alias = "dateMs")]
    pub date_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct MoreDto {
    pub description: String,
    pub hb: Option<String>,
    pub streak: i32,
    #[serde(alias = "pLang")]
    pub p_lang: Vec<String>,
    pub plans: Vec<String>,
    pub subscribers: i32,
    pub friends: i32,
    pub status: String,
    #[serde(alias = "postsCount")]
    pub posts_count: i32,
    pub level: i32,
    pub stack: Vec<String>,
    pub logo: String,
    pub banner: String,
    pub who: String,
    pub rating: i32,
    pub goals: Vec<GoalDto>,
    #[serde(alias = "plansStruct")]
    pub plans_struct: Vec<PlanDto>,
    #[serde(alias = "mediaLinks")]
    pub media_links: Vec<String>,
    #[serde(alias = "socialLinks")]
    pub social_links: Vec<SocialLinkDto>,
}

/// One shape for both profile responses. `BaseUserProfileResponse` and
/// `UserProfileWithSubscriptionResponse` share every field this reads and differ
/// only in what each adds, so a viewer's profile and the owner's decode alike.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProfileDto {
    pub id: String,
    pub name: Option<String>,
    pub phone: Option<String>,
    pub tag: Option<String>,
    #[serde(alias = "isPremium")]
    pub is_premium: Option<bool>,
    #[serde(alias = "isOfficial")]
    pub is_official: Option<bool>,
    pub gender: Option<String>,
    pub role: Option<String>,
    #[serde(alias = "isBlocked")]
    pub is_blocked: Option<bool>,
    pub more: Option<MoreDto>,
    #[serde(alias = "isFriend")]
    pub is_friend: Option<bool>,
    #[serde(alias = "isSubbed")]
    pub is_subbed: Option<bool>,
    #[serde(alias = "isSubscriber")]
    pub is_subscriber: Option<bool>,
    #[serde(alias = "friendRequestId")]
    pub friend_request_id: Option<i64>,
    #[serde(alias = "friendsCount")]
    pub friends_count: i64,
    #[serde(alias = "subscribersCount")]
    pub subscribers_count: i64,
    #[serde(alias = "subsCount")]
    pub subs_count: i64,
    #[serde(alias = "postsCount")]
    pub posts_count: i64,
    #[serde(alias = "isOnline")]
    pub is_online: Option<bool>,
    #[serde(alias = "lastSeen")]
    pub last_seen: Option<i64>,
    #[serde(alias = "quickReaction")]
    pub quick_reaction: Option<String>,
    #[serde(alias = "isPhonePublic")]
    pub is_phone_public: Option<bool>,
    #[serde(alias = "subscriptionStatus")]
    pub subscription_status: Option<String>,
    #[serde(alias = "viewerHasBlocked")]
    pub viewer_has_blocked: Option<bool>,
    #[serde(alias = "userChatId")]
    pub user_chat_id: Option<String>,
}

/// Every profile method answers `{success, error, profile}`: the gateway hands
/// `json!(response)` to the socket, so the inner message keeps its own key.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProfileResponse {
    pub error: Option<String>,
    pub profile: Option<ProfileDto>,
}

/// `send_friend_request` answers with the created row and no relationship flags,
/// so a follow has no authoritative echo to reconcile against — only `unfollow`
/// does. Anything optimistic about a follow therefore stands until the next read.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FriendRequestResponse {
    pub error: Option<String>,
    pub id: Option<i64>,
    pub status: Option<String>,
    #[serde(alias = "receiverId")]
    pub receiver_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RelationshipResponse {
    pub error: Option<String>,
    #[serde(alias = "isFriend")]
    pub is_friend: Option<bool>,
    #[serde(alias = "isSubbed")]
    pub is_subbed: Option<bool>,
    #[serde(alias = "isSubscriber")]
    pub is_subscriber: Option<bool>,
    #[serde(alias = "friendRequestId")]
    pub friend_request_id: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_engine::ReplayPolicy;
    use serde_json::json;

    #[test]
    fn nothing_a_user_can_only_do_once_is_replayable() {
        assert_eq!(GET_MY_PROFILE.replay, ReplayPolicy::ReadOnly);
        assert_eq!(GET_PROFILE_BY_ID.replay, ReplayPolicy::ReadOnly);
        assert_eq!(
            SEND_FRIEND_REQUEST.replay,
            ReplayPolicy::Never,
            "the response is the row this call created, so a retry answers about the first attempt"
        );
        assert_eq!(UNFOLLOW_USER.replay, ReplayPolicy::Never);
        assert_eq!(DELETE_FRIEND.replay, ReplayPolicy::Never);
    }

    #[test]
    fn a_viewer_is_omitted_by_tag_and_required_by_id() {
        let anonymous = serde_json::to_value(GetProfileByTagRequest {
            tag: "aianov".into(),
            user_id: None,
        })
        .unwrap();
        assert!(anonymous.get("user_id").is_none());

        let identified = serde_json::to_value(GetProfileByIdRequest {
            profile_user_id: "u2".into(),
            user_id: "u1".into(),
        })
        .unwrap();
        assert_eq!(identified["user_id"], json!("u1"));
        assert_eq!(identified["profile_user_id"], json!("u2"));
    }

    #[test]
    fn a_profile_arrives_nested_under_its_own_key() {
        let response: ProfileResponse = serde_json::from_value(json!({
            "success": true,
            "error": "",
            "profile": {
                "id": "u1",
                "name": "Aianov",
                "tag": "aianov",
                "friends_count": 4,
                "subscribers_count": 1200,
                "is_subbed": true,
                "more": {"description": "Day 370", "logo": "https://cdn/a.png", "level": 7}
            }
        }))
        .unwrap();

        let profile = response
            .profile
            .expect("the gateway nests it under `profile`");
        assert_eq!(profile.id, "u1");
        assert_eq!(profile.subscribers_count, 1200);
        assert_eq!(profile.is_subbed, Some(true));
        assert_eq!(profile.more.unwrap().level, 7);
        assert!(rise_engine::remote_message(response.error.as_deref()).is_none());
    }

    #[test]
    fn the_camel_case_form_decodes_beside_the_snake_case_one() {
        let snake: ProfileDto = serde_json::from_value(json!({
            "id": "u1", "friends_count": 2, "is_phone_public": true,
            "more": {"posts_count": 9, "plans_struct": [{"title": "Ship", "date_ms": 1}]}
        }))
        .unwrap();
        assert_eq!(snake.friends_count, 2);
        assert_eq!(snake.is_phone_public, Some(true));
        assert_eq!(snake.more.as_ref().unwrap().posts_count, 9);
        assert_eq!(snake.more.unwrap().plans_struct[0].date_ms, Some(1));

        let camel: ProfileDto = serde_json::from_value(json!({
            "id": "u1", "friendsCount": 2, "isPhonePublic": true,
            "more": {"postsCount": 9, "plansStruct": [{"title": "Ship", "dateMs": 1}]}
        }))
        .unwrap();
        assert_eq!(camel.friends_count, 2);
        assert_eq!(camel.is_phone_public, Some(true));
        assert_eq!(camel.more.unwrap().posts_count, 9);
    }

    #[test]
    fn a_profile_with_every_proto_default_dropped_still_decodes() {
        let profile: ProfileDto = serde_json::from_value(json!({"id": "u1"})).unwrap();

        assert_eq!(profile.friends_count, 0);
        assert_eq!(profile.is_subbed, None);
        assert!(profile.more.is_none());
        assert_eq!(profile.tag, None);
    }

    #[test]
    fn only_unfollow_answers_with_a_relationship() {
        let follow: FriendRequestResponse =
            serde_json::from_value(json!({"id": 12, "status": "accepted", "receiver_id": "u2"}))
                .unwrap();
        assert_eq!(follow.id, Some(12));

        let unfollow: RelationshipResponse = serde_json::from_value(json!({
            "is_friend": false, "is_subbed": false, "is_subscriber": true
        }))
        .unwrap();
        assert_eq!(unfollow.is_subbed, Some(false));
        assert_eq!(unfollow.is_subscriber, Some(true));
    }

    #[test]
    fn an_empty_error_string_is_not_an_error() {
        let response: ProfileResponse =
            serde_json::from_value(json!({"error": "", "profile": {"id": "u1"}})).unwrap();
        assert!(rise_engine::remote_message(response.error.as_deref()).is_none());
    }
}
