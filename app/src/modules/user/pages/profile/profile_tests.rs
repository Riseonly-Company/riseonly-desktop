use gpui::TestAppContext;
use rise_canon::rise_request::user::GetProfileById;
use serde_json::{Value, json};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsDiskCachePolicy;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::user::pages::profile::profile::ProfileRouteTarget;
use crate::modules::user::stores::user::user_actions::user_wire_types::UserGetProfileByTagGatewayDTOExt;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const VIEWER: &str = "viewer-user";

fn foreign_payload(id: &str, tag: &str) -> Value {
    let mut name = tag.to_owned();
    if let Some(first) = name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    json!({
        "success": true,
        "profile": {
            "id": id,
            "name": name,
            "tag": tag,
            "is_premium": false,
            "is_official": false,
            "is_blocked": false,
            "friends_count": 0,
            "subscribers_count": 0,
            "subs_count": 0,
            "posts_count": 0
        },
        "is_friend": false,
        "is_subscribed": false,
        "is_subscriber": false
    })
}

fn install(root: Value, cache_key: &str, cx: &mut gpui::App) {
    SaiWsResponseCache::install_optimistic_root(
        root,
        cache_key,
        true,
        false,
        SaiWsDiskCachePolicy::PreserveExisting,
        false,
        cx,
    )
    .expect("the root installs");
}

#[test]
fn id_only_foreign_route_never_resolves_as_own_profile() {
    let target = ProfileRouteTarget::resolve(true, Some("  user-42  "), None);
    assert_eq!(target, ProfileRouteTarget::ForeignUserId("user-42".to_owned()));
}

#[test]
fn tag_remains_canonical_when_route_contains_both_identifiers() {
    let target = ProfileRouteTarget::resolve(true, Some("user-42"), Some("  twink  "));
    assert_eq!(target, ProfileRouteTarget::ForeignTag("twink".to_owned()));
}

#[test]
fn malformed_foreign_route_does_not_fall_back_to_authenticated_profile() {
    let target = ProfileRouteTarget::resolve(true, Some("  "), None);
    assert_eq!(target, ProfileRouteTarget::Invalid);
}

#[test]
fn own_tab_without_foreign_identifiers_resolves_as_own_profile() {
    let target = ProfileRouteTarget::resolve(false, None, None);
    assert_eq!(target, ProfileRouteTarget::Own);
}

#[test]
fn profile_by_id_body_uses_distinct_target_and_viewer_wire_fields() {
    let body = GetProfileById::new("target-user".to_owned());
    let encoded = serde_json::to_value(&body).expect("the body encodes");
    assert_eq!(encoded, json!({ "profile_user_id": "target-user" }));
}

#[gpui::test]
fn late_response_for_previous_route_cannot_evict_current_foreign_profile(cx: &mut TestAppContext) {
    cx.update(|cx| {
        SaiAccountCacheScope::activate(VIEWER);
        let key_a = UserServicesStore::profile_by_id_cache_key("user-a", VIEWER);
        let key_b = UserServicesStore::profile_by_id_cache_key("user-b", VIEWER);

        install(foreign_payload("user-b", "beta"), &key_b, cx);
        install(foreign_payload("user-a", "alpha"), &key_a, cx);

        let current = UserServicesStore::shared(cx)
            .update(cx, |store, cx| store.cached_foreign_profile_user_id("user-b", Some(VIEWER), cx))
            .and_then(|payload| payload.as_profile_model())
            .expect("the current route keeps its profile");
        assert_eq!(current.id, "user-b");

        SaiWsResponseCache::remove(&key_a, SaiCacheCleanLayer::Everywhere, cx);
        SaiWsResponseCache::remove(&key_b, SaiCacheCleanLayer::Everywhere, cx);
        SaiAccountCacheScope::clear_active_account();
    });
}

#[gpui::test]
fn response_with_wrong_identity_is_not_exposed_for_requested_route(cx: &mut TestAppContext) {
    cx.update(|cx| {
        SaiAccountCacheScope::activate(VIEWER);
        let key_b = UserServicesStore::profile_by_id_cache_key("user-b", VIEWER);

        install(foreign_payload("user-a", "alpha"), &key_b, cx);

        let exposed = UserServicesStore::shared(cx)
            .update(cx, |store, cx| store.cached_foreign_profile_user_id("user-b", Some(VIEWER), cx));
        assert!(exposed.is_none());

        SaiWsResponseCache::remove(&key_b, SaiCacheCleanLayer::Everywhere, cx);
        SaiAccountCacheScope::clear_active_account();
    });
}

#[gpui::test]
fn tag_profile_cache_uses_canonical_case_insensitive_identity(cx: &mut TestAppContext) {
    cx.update(|cx| {
        SaiAccountCacheScope::activate(VIEWER);
        let key = UserServicesStore::profile_by_tag_cache_key("beta");

        install(foreign_payload("user-b", "beta"), &key, cx);

        let found = UserServicesStore::shared(cx)
            .update(cx, |store, cx| store.cached_foreign_profile_tag("BETA", cx))
            .and_then(|payload| payload.as_profile_model())
            .map(|model| model.id);
        assert_eq!(found.as_deref(), Some("user-b"));

        SaiWsResponseCache::remove(&key, SaiCacheCleanLayer::Everywhere, cx);
        SaiAccountCacheScope::clear_active_account();
    });
}
