use crate::modules::post::stores::post_actions::post_likes_wire_types::{GetPostLikesGatewayDTO, PostLikeUserDTOExt};
use crate::modules::post::stores::post_services::post_likes_services::PostLikesTarget;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::FollowButtonState;

struct PostLikesContractTests;

impl PostLikesContractTests {
    fn gateway_payload() -> &'static str {
        r#"
        {
          "limit": 30,
          "items": [{
            "id": "user-2",
            "name": "Dana",
            "tag": "dana",
            "logo": "avatar.jpg",
            "is_premium": true,
            "is_official": false,
            "is_friend": false,
            "is_subbed": false,
            "is_subscriber": true,
            "liked_at_ms": 1721412000000
          }],
          "has_more": true,
          "relative_id": 41
        }
        "#
    }
}

#[test]
fn gateway_payload_decodes_relationship_and_pagination_fields() {
    let response: GetPostLikesGatewayDTO = serde_json::from_str(PostLikesContractTests::gateway_payload()).expect("likes payload decodes");
    let user = response.items.first().map(PostLikeUserDTOExt::as_domain).expect("one like row");

    assert!(response.has_more);
    assert_eq!(response.relative_id, Some(41));
    assert_eq!(user.id, "user-2");
    assert!(user.is_premium);
    assert_eq!(user.relationship().follow_button_state(), FollowButtonState::SubscribeBack);
}

#[test]
fn search_normalization_trims_caps_and_stabilizes_cache_keys() {
    let oversized = format!("  {}  ", "A".repeat(160));
    let normalized = PostServicesStore::normalized_post_likes_search(Some(&oversized));

    assert_eq!(normalized.map(|search| search.chars().count()), Some(128));
    assert_eq!(PostServicesStore::normalized_post_likes_search(Some("  \n")), None);
    assert_eq!(
        PostServicesStore::post_likes_cache_key(&PostLikesTarget::Post("42".to_owned()), Some(" Dana ")),
        PostServicesStore::post_likes_cache_key(&PostLikesTarget::Post("42".to_owned()), Some("dana"))
    );
    assert_ne!(
        PostServicesStore::post_likes_cache_key(&PostLikesTarget::Post("42".to_owned()), None),
        PostServicesStore::post_likes_cache_key(&PostLikesTarget::Post("43".to_owned()), None)
    );
    assert_ne!(
        PostServicesStore::post_likes_cache_key(&PostLikesTarget::Post("42".to_owned()), None),
        PostServicesStore::post_likes_cache_key(&PostLikesTarget::Comment("42".to_owned()), None)
    );
}
