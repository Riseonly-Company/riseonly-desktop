use std::collections::{BTreeMap, HashSet};

use rise_canon::rise_key::user::{ACHIEVEMENT_ID, ACHIEVEMENT_IDS, HIDDEN, USER_ID};
use rise_canon::{rise_request, rise_response, rise_wire_methods};
use rise_endpoints::ENDPOINTS;
use rise_engine::{RiseJSONValue, RiseWirePushEnvelope, RiseWireReplayPolicy};
use serde::Serialize;

use crate::modules::user::engine::core::rise_user_achievement_wire_models::{
    RiseAchievement, RiseAchievementMediaKind,
};
use crate::modules::user::engine::core::rise_user_rpc::RiseUserEndpoints;
use crate::modules::user::engine::rise_user_achievements_realtime::RiseUserAchievementsRealtime;

const ACHIEVEMENTS_CATALOG: &str = "user_achievements_actions";

#[test]
fn endpoint_catalogue_owns_every_achievement_contract() {
    let canon_user: Vec<_> = ENDPOINTS
        .iter()
        .filter(|endpoint| endpoint.service == rise_wire_methods::user::SERVICE)
        .collect();
    let achievement_methods: Vec<&str> = canon_user
        .iter()
        .filter(|endpoint| endpoint.catalog == Some(ACHIEVEMENTS_CATALOG))
        .map(|endpoint| endpoint.method)
        .collect();
    let endpoints = RiseUserEndpoints::all();
    let catalogue: HashSet<String> = endpoints.iter().map(|endpoint| endpoint.method.clone()).collect();
    let canon_methods: HashSet<String> = canon_user.iter().map(|endpoint| endpoint.method.to_owned()).collect();

    assert!(!achievement_methods.is_empty());
    assert_eq!(catalogue.len(), endpoints.len());
    assert!(catalogue.is_subset(&canon_methods));
    assert!(achievement_methods.iter().all(|method| catalogue.contains(*method)));
    assert!(
        endpoints
            .iter()
            .all(|endpoint| endpoint.descriptor().service == rise_wire_methods::user::SERVICE)
    );
    assert!(
        achievement_methods
            .iter()
            .all(|method| RiseUserEndpoints::endpoint(method).is_some())
    );
}

#[test]
fn reads_replay_and_writes_never_do() {
    assert_eq!(
        RiseUserEndpoints::get_my_achievements().descriptor().replay_policy,
        RiseWireReplayPolicy::ReadOnly
    );
    assert_eq!(
        RiseUserEndpoints::get_user_achievements().descriptor().replay_policy,
        RiseWireReplayPolicy::ReadOnly
    );
    assert_eq!(
        RiseUserEndpoints::set_achievement_hidden().descriptor().replay_policy,
        RiseWireReplayPolicy::Never
    );
    assert_eq!(
        RiseUserEndpoints::mark_achievements_seen().descriptor().replay_policy,
        RiseWireReplayPolicy::Never
    );
}

#[test]
fn own_list_request_carries_nothing_because_identity_is_injected() {
    let object = encoded_wire_object(&rise_request::user::GetMyAchievements {});

    assert!(object.is_empty());
}

#[test]
fn showcase_request_names_the_addressed_user() {
    let object = encoded_wire_object(&rise_request::user::GetUserAchievements::new("user-1".to_owned()));

    assert_eq!(object.len(), 1);
    assert_eq!(object.get(USER_ID).and_then(serde_json::Value::as_str), Some("user-1"));
}

#[test]
fn hide_request_uses_gateway_snake_case_keys() {
    let object = encoded_wire_object(&rise_request::user::SetAchievementHidden::new(12, true));

    assert_eq!(object.len(), 2);
    assert_eq!(object.get(ACHIEVEMENT_ID).and_then(serde_json::Value::as_i64), Some(12));
    assert_eq!(object.get(HIDDEN).and_then(serde_json::Value::as_bool), Some(true));
}

#[test]
fn seen_request_keeps_the_plural_ids_key_intact() {
    let object = encoded_wire_object(&rise_request::user::MarkAchievementsSeen {
        achievement_ids: Some(vec![12, 13]),
    });

    assert_eq!(object.len(), 1);
    assert_eq!(object.get(ACHIEVEMENT_IDS), Some(&serde_json::Value::from(vec![12_i64, 13])));
    assert!(!object.contains_key("achievement_i_ds"));
}

#[test]
fn empty_seen_request_still_sends_the_key() {
    let object = encoded_wire_object(&rise_request::user::MarkAchievementsSeen {
        achievement_ids: Some(Vec::new()),
    });

    assert_eq!(object.get(ACHIEVEMENT_IDS), Some(&serde_json::Value::from(Vec::<i64>::new())));
}

#[test]
fn achievement_decodes_from_the_gateway_snake_case_payload() {
    let payload = r##"
        {
          "success": true,
          "error": "",
          "achievements": [
            {
              "id": 12,
              "code_key": "account_created",
              "media_kind": "builtin",
              "media_ref": "greetings",
              "accent_color": "#FF7A45",
              "title": "Welcome",
              "description": "You joined",
              "how_to_earn": "Create an account",
              "granted_at_ms": 1750000000000,
              "seen_at_ms": 0,
              "hidden": false,
              "owner_only": false,
              "granted_count": 1234
            }
          ],
          "total": 3,
          "hidden_count": 1,
          "achievements_rule": "ALL"
        }
        "##;
    let response: rise_response::user::GetMyAchievements =
        serde_json::from_str(payload).expect("fixture decodes");
    let achievement = response.achievements.first().expect("fixture has one row");

    assert!(response.success);
    assert_eq!(response.total, 3);
    assert_eq!(response.hidden_count, 1);
    assert_eq!(response.achievements_rule, "ALL");
    assert_eq!(achievement.id, 12);
    assert_eq!(achievement.code_key, "account_created");
    assert_eq!(achievement.media_kind, "builtin");
    assert_eq!(achievement.media_ref, "greetings");
    assert_eq!(achievement.accent_color, "#FF7A45");
    assert_eq!(achievement.how_to_earn, "Create an account");
    assert_eq!(achievement.granted_at_ms, 1_750_000_000_000);
    assert_eq!(achievement.granted_count, 1234);
    assert_eq!(achievement.seen_at_ms, 0);
}

#[test]
fn a_missing_field_never_costs_the_rest_of_the_row() {
    let payload = r#"
        {
          "achievements": [
            { "id": "77", "title": "Partial", "media_kind": "hologram" }
          ]
        }
        "#;
    let response: rise_response::user::GetUserAchievements =
        serde_json::from_str(payload).expect("fixture decodes");
    let achievement = response.achievements.first().expect("fixture has one row");

    assert!(!response.success);
    assert!(!response.visible);
    assert_eq!(response.total, 0);
    assert_eq!(achievement.id, 77);
    assert_eq!(achievement.title, "Partial");
    assert_eq!(achievement.media_kind, "hologram");
    assert_eq!(RiseAchievement::from_canon(achievement).media_kind, RiseAchievementMediaKind::Builtin);
    assert!(achievement.code_key.is_empty());
    assert_eq!(achievement.granted_count, 0);
}

#[test]
fn grant_decodes_from_the_push_envelope() {
    let envelope = push_envelope(
        r#"
        {
          "id": 12,
          "code_key": "account_created",
          "title": "Welcome",
          "granted_at_ms": 1750000000000
        }
        "#,
    );
    let grant = RiseUserAchievementsRealtime::decode(&envelope).expect("grant decodes");

    assert_eq!(RiseUserAchievementsRealtime::EVENT_TYPE, "achievement_earned");
    assert_eq!(grant.id(), 12);
    assert_eq!(grant.achievement.title, "Welcome");
    assert_eq!(grant.occurred_at_ms, 1_750_000_000_000);
}

#[test]
fn a_malformed_push_is_dropped_instead_of_thrown() {
    let empty = push_envelope("{}");
    let without_identity = push_envelope(r#"{"id": 0}"#);
    let not_an_object = push_envelope("[1, 2]");

    assert!(RiseUserAchievementsRealtime::decode(&empty).is_none());
    assert!(RiseUserAchievementsRealtime::decode(&without_identity).is_none());
    assert!(RiseUserAchievementsRealtime::decode(&not_an_object).is_none());
}

fn encoded_wire_object(request: &impl Serialize) -> serde_json::Map<String, serde_json::Value> {
    match serde_json::to_value(request).expect("request encodes") {
        serde_json::Value::Object(object) => object,
        other => panic!("request encoded as {other}, not an object"),
    }
}

fn push_envelope(json: &str) -> RiseWirePushEnvelope {
    let data = RiseJSONValue::decode(json.as_bytes()).expect("fixture decodes");
    RiseWirePushEnvelope::new(
        RiseUserAchievementsRealtime::EVENT_TYPE.to_owned(),
        data,
        None,
        BTreeMap::new(),
    )
}
