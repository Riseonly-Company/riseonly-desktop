use rise_canon::rise_request::notify::register_device_token;
use serde_json::Value;

use crate::core::push_notifications::push_ownership_generation::{
    PushOwnershipGenerationPolicy, PushOwnershipRegistrationContext,
};

fn context(user_id: &str, generation: i64) -> PushOwnershipRegistrationContext {
    PushOwnershipRegistrationContext { user_id: user_id.to_owned(), generation }
}

#[test]
fn first_owner_uses_clock_floor_and_persists_it() {
    let decision = PushOwnershipGenerationPolicy::registration(" account-a ", None, 41, 1_000)
        .expect("a valid owner gets a decision");

    assert_eq!(decision.context, context("account-a", 1_000));
    assert_eq!(decision.high_water_to_persist, Some(1_000));
}

#[test]
fn reconnect_for_same_owner_reuses_generation() {
    let current = context("account-a", 77);
    let decision = PushOwnershipGenerationPolicy::registration("account-a", Some(&current), 77, 2_000)
        .expect("a valid owner gets a decision");

    assert_eq!(decision.context, current);
    assert_eq!(decision.high_water_to_persist, None);
}

#[test]
fn account_switch_always_advances_past_in_memory_and_persisted_versions() {
    let current = context("account-a", 101);
    let decision = PushOwnershipGenerationPolicy::registration("account-b", Some(&current), 150, 120)
        .expect("a valid owner gets a decision");

    assert_eq!(decision.context, context("account-b", 151));
    assert_eq!(decision.high_water_to_persist, Some(151));
}

#[test]
fn externally_advanced_high_water_forces_even_same_owner_forward() {
    let current = context("account-a", 77);
    let decision = PushOwnershipGenerationPolicy::registration("account-a", Some(&current), 90, 80)
        .expect("a valid owner gets a decision");

    assert_eq!(decision.context.generation, 91);
    assert_eq!(decision.high_water_to_persist, Some(91));
}

#[test]
fn erased_persistence_can_recover_with_newer_clock_floor() {
    let decision =
        PushOwnershipGenerationPolicy::registration("account-b", None, 0, 1_750_000_000_000)
            .expect("a valid owner gets a decision");

    assert_eq!(decision.context.generation, 1_750_000_000_000);
}

#[test]
fn invalid_user_and_overflow_fail_closed() {
    assert_eq!(PushOwnershipGenerationPolicy::registration("   ", None, 1, 2), None);
    assert_eq!(
        PushOwnershipGenerationPolicy::registration("account-b", None, i64::MAX, i64::MAX),
        None
    );
}

#[test]
fn registration_body_uses_backward_compatible_snake_case_field() {
    let mut body = rise_canon::rise_request::notify::RegisterDeviceToken::new(
        "apns-token".to_owned(),
        register_device_token::Platform::Ios,
    );
    body.ownership_generation = Some(42);
    let object = serde_json::to_value(&body).expect("the body encodes");

    assert!(object.get("user_id").is_none());
    assert_eq!(object.get("device_token").and_then(Value::as_str), Some("apns-token"));
    assert_eq!(object.get("ownership_generation").and_then(Value::as_i64), Some(42));
}
