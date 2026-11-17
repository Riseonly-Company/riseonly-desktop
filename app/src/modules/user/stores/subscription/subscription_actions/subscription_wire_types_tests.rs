use rise_canon::rise_key::subscription as key;
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::Locale;
use serde_json::{Map, Value};

use super::subscription_types::{SubscriptionBillingDateKind, SubscriptionStatusModel};
use super::subscription_wire_types::{SubscriptionStatusGatewayDTO, SubscriptionStatusGatewayModel};

const PRODUCT_ID: &str = "riseonly_1month_premium";
const SUBSCRIPTION_ID: &str = "subscription-1";
const PLATFORM: &str = "ios";
const STATUS: &str = "active";

fn reference_now() -> Date {
    Date::from_time_interval_since_1970(1_752_537_600.0)
}

fn date(seconds: i64) -> Date {
    Date::from_time_interval_since_1970(seconds as f64)
}

fn dto(expires_at: i64, auto_renew: bool, next_billing_date: Option<i64>) -> SubscriptionStatusGatewayDTO {
    SubscriptionStatusGatewayDTO {
        is_active: true,
        is_premium: true,
        subscription_id: Some(SUBSCRIPTION_ID.to_owned()),
        expires_at: Some(expires_at),
        status: STATUS.to_owned(),
        auto_renew,
        platform: Some(PLATFORM.to_owned()),
        product_id: Some(PRODUCT_ID.to_owned()),
        next_billing_date,
        ..Default::default()
    }
}

#[test]
fn gateway_status_decodes_renewal_contract() {
    let mut object = Map::new();
    object.insert(key::SUCCESS.to_owned(), Value::Bool(true));
    object.insert(key::STATUS.to_owned(), Value::from(STATUS));
    object.insert(key::camel::EXPIRES_AT.to_owned(), Value::from(1_755_216_000_i64));
    object.insert(key::camel::NEXT_BILLING_DATE.to_owned(), Value::from(1_755_216_000_i64));
    object.insert(key::camel::IS_ACTIVE.to_owned(), Value::Bool(true));
    object.insert(key::camel::IS_PREMIUM.to_owned(), Value::Bool(true));
    object.insert(key::camel::PRODUCT_ID.to_owned(), Value::from(PRODUCT_ID));
    object.insert(key::camel::AUTO_RENEW.to_owned(), Value::Bool(true));
    let data = serde_json::to_vec(&Value::Object(object)).expect("fixture encodes");

    let dto: SubscriptionStatusGatewayDTO = serde_json::from_slice(&data).expect("status decodes");
    let status = dto.as_subscription_status_model(reference_now());

    assert_eq!(dto.expires_at, Some(1_755_216_000));
    assert_eq!(dto.next_billing_date, Some(1_755_216_000));
    assert!(status.is_active);
    assert_eq!(status.billing_date, Some(date(1_755_216_000)));
    assert_eq!(status.billing_date_kind, SubscriptionBillingDateKind::Renewal);
}

#[test]
fn active_non_renewing_status_uses_expiration_date() {
    let status = dto(1_755_216_000, false, None).as_subscription_status_model(reference_now());

    assert!(status.is_active);
    assert_eq!(status.billing_date, Some(date(1_755_216_000)));
    assert_eq!(status.billing_date_kind, SubscriptionBillingDateKind::Expiration);
}

#[test]
fn stale_cached_active_status_expires_locally() {
    let status = dto(1_752_451_200, true, Some(1_752_451_200)).as_subscription_status_model(reference_now());

    assert!(!status.is_active);
    assert_eq!(status.billing_date, None);
}

#[test]
fn expired_renewal_date_falls_back_to_future_expiration() {
    let status = dto(1_755_216_000, true, Some(1_752_451_200)).as_subscription_status_model(reference_now());

    assert!(status.is_active);
    assert_eq!(status.billing_date, Some(date(1_755_216_000)));
    assert_eq!(status.billing_date_kind, SubscriptionBillingDateKind::Renewal);
}

#[test]
fn expired_expiration_date_falls_back_to_future_renewal() {
    let status = dto(1_752_451_200, false, Some(1_755_216_000)).as_subscription_status_model(reference_now());

    assert!(status.is_active);
    assert_eq!(status.billing_date, Some(date(1_755_216_000)));
    assert_eq!(status.billing_date_kind, SubscriptionBillingDateKind::Expiration);
}

#[test]
fn cached_millisecond_timestamp_is_normalized() {
    let status = dto(1_755_216_000_000, false, None).as_subscription_status_model(reference_now());

    assert_eq!(status.billing_date, Some(date(1_755_216_000)));
}

#[test]
fn billing_date_formatting_uses_requested_locale() {
    let model = SubscriptionStatusModel {
        status: STATUS.to_owned(),
        plan: None,
        billing_date: Some(date(1_755_216_000)),
        billing_date_kind: SubscriptionBillingDateKind::Renewal,
        auto_renew: true,
        is_active: true,
    };
    let utc = TimeZone::with_seconds_from_gmt(0).expect("utc zone");
    let english = model.formatted_billing_date(&Locale::new("en_US"), &utc).expect("english date");
    let russian = model.formatted_billing_date(&Locale::new("ru_RU"), &utc).expect("russian date");

    let russian_august = "\u{0430}\u{0432}\u{0433}\u{0443}\u{0441}\u{0442}";
    assert!(english.contains("August"));
    assert!(russian.to_lowercase().contains(russian_august));
    assert_ne!(english, russian);
}
