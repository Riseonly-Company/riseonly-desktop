use std::collections::HashSet;

use gpui::{AppContext as _, Entity, TestAppContext};
use rise_canon::rise_key;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_range::NSRange;
use rise_regions::PHONE_VALIDITY_SAMPLES;
use rise_ui::phone_country_catalog::{PhoneCountryCatalog, PhoneInputCountry};
use rise_validate::VERIFICATION_CODE;
use serde_json::{Map, Value};

use crate::core::lib::canon_rules::ValidationSchemaCanonRules as _;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_error::SaiFetchHttpError;
use crate::core::stores::http::http_service::HttpService;
use crate::core::ui::phone_input_ui::PhoneInputUi;
use crate::modules::auth::pages::sign::auth_step_flow::{
    AuthCodeEntry, AuthStepFlowModel, AuthStepFlowModelPurpose, AuthStepFlowModelStep,
    AuthStepFlowModelTagAvailability,
};
use crate::modules::auth::services::registration_policy_probe::RegistrationPolicyProbeMode;
use crate::modules::auth::stores::auth_actions::auth_types::{AuthCodeChannel, AuthCodeDelivery};
use crate::modules::auth::stores::auth_actions::auth_wire_types::{
    AuthCheckTagResponse, AuthRegisterBody, AuthSendCodeResponse,
};
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

fn step_flow_model(purpose: AuthStepFlowModelPurpose, cx: &TestAppContext) -> Entity<AuthStepFlowModel> {
    cx.update(|cx| cx.new(|_| AuthStepFlowModel::new(purpose)))
}

fn phone_country(id: &str, region_code: &str, calling_code: &str, format_pattern: &str, english_name: &str) -> PhoneInputCountry {
    PhoneInputCountry {
        id: id.to_owned(),
        region_code: region_code.to_owned(),
        calling_code: calling_code.to_owned(),
        format_pattern: format_pattern.to_owned(),
        english_name: english_name.to_owned(),
    }
}

fn applying(replacement: &str, location: usize, length: usize, current: &str, digit_count: i64) -> String {
    AuthCodeEntry::applying(replacement, NSRange::new(location, length), current, digit_count)
}

fn refusal(code: &str, details: &[(&str, i64)]) -> anyhow::Error {
    let details: Map<String, Value> =
        details.iter().map(|(name, value)| ((*name).to_owned(), Value::from(*value))).collect();
    let mut object = Map::new();
    object.insert("code".to_owned(), Value::from(code));
    object.insert("error".to_owned(), Value::from(""));
    object.insert("details".to_owned(), Value::Object(details));
    let body = serde_json::to_vec(&Value::Object(object)).expect("refusal body");
    anyhow::Error::new(SaiFetchHttpError::BadStatus { code: 400, body })
}

fn prose(message: &str) -> anyhow::Error {
    let mut object = Map::new();
    object.insert("error".to_owned(), Value::from(message));
    let body = serde_json::to_vec(&Value::Object(object)).expect("prose body");
    anyhow::Error::new(SaiFetchHttpError::BadStatus { code: 400, body })
}

#[gpui::test]
async fn login_to_registration_keeps_the_entered_phone(cx: &mut TestAppContext) {
    let model = step_flow_model(AuthStepFlowModelPurpose::Login, cx);
    cx.update(|cx| {
        model.update(cx, |model, cx| {
            model.selected_country = phone_country("test-kz", "KZ", "+7", "", "Kazakhstan");
            model.update_phone("7771234567", cx);
            model.advance_after_local_validation(cx);
        });
    });
    assert_eq!(cx.read(|cx| model.read(cx).step), AuthStepFlowModelStep::LoginPassword);

    let opening = cx.update(|cx| {
        model.update(cx, |model, cx| {
            model.begin_registration_from_login_read_policy(
                async { Some(RegistrationPolicyProbeMode::Open) },
                |_| {},
                cx,
            )
        })
    });
    opening.expect("opening task").await;
    cx.read(|cx| {
        let model = model.read(cx);
        assert_eq!(model.purpose, AuthStepFlowModelPurpose::Registration);
        assert_eq!(model.step, AuthStepFlowModelStep::RegistrationName);
        assert_eq!(model.phone_number, "7771234567");
        assert_eq!(model.composed_phone_number(), "77771234567");
    });

    cx.update(|cx| model.update(cx, |model, cx| model.go_back(cx)));
    cx.read(|cx| {
        let model = model.read(cx);
        assert_eq!(model.purpose, AuthStepFlowModelPurpose::Login);
        assert_eq!(model.step, AuthStepFlowModelStep::LoginPassword);
        assert_eq!(model.phone_number, "7771234567");
    });
}

#[test]
fn phone_normalization_supports_full_e164_range() {
    assert_eq!(AuthInteractionsStore::compose_phone_number("+376", "12 34 56"), "376123456");
    assert!(AuthStepFlowModel::is_valid_phone("6831234"));
    assert!(AuthStepFlowModel::is_valid_phone(&"1".repeat(15)));
    assert!(!AuthStepFlowModel::is_valid_phone("123456"));
    assert!(!AuthStepFlowModel::is_valid_phone(&"1".repeat(16)));
}

#[test]
fn pasting_full_international_number_does_not_duplicate_calling_code() {
    let country = phone_country("KZ-7-test", "KZ", "+7", "XXX XXX XX XX", "Kazakhstan");

    assert_eq!(PhoneInputUi::national_digits("+7 777 123 45 67", &country), "7771234567");
    assert_eq!(PhoneInputUi::national_digits("7771234567", &country), "7771234567");
    assert_eq!(
        PhoneCountryCatalog::country_matching_international_number("+44 20 7946 0958")
            .map(|country| country.calling_code)
            .as_deref(),
        Some("+44")
    );
}

#[test]
fn username_validation_matches_server_rules() {
    assert_eq!(AuthStepFlowModel::tag_validation_key("rise_only51"), None);
    assert_eq!(AuthStepFlowModel::tag_validation_key("ab"), Some("tag_too_short"));
    assert_eq!(AuthStepFlowModel::tag_validation_key("_rise"), Some("auth_tag_edge_underscore"));
    assert_eq!(AuthStepFlowModel::tag_validation_key("rise__only"), Some("auth_tag_double_underscore"));
    assert_eq!(
        AuthStepFlowModel::tag_validation_key("\u{440}\u{430}\u{439}\u{437}"),
        Some("tag_invalid_characters")
    );
    assert_eq!(AuthStepFlowModel::tag_validation_key(&"a".repeat(33)), Some("auth_tag_too_long"));
}

#[test]
fn unicode_length_validation_matches_rust_scalar_counting() {
    let composed_character = "e\u{301}";

    assert_eq!(AuthStepFlowModel::name_validation_key(composed_character), None);
    assert_eq!(AuthStepFlowModel::name_validation_key(&composed_character.repeat(33)), Some("auth_name_invalid"));
    assert_eq!(AuthStepFlowModel::password_validation_key(&composed_character.repeat(4)), None);
    assert_eq!(
        AuthStepFlowModel::password_validation_key(&composed_character.repeat(65)),
        Some("auth_password_too_long")
    );
}

#[gpui::test]
fn registration_tag_conflict_forces_a_recheck(cx: &mut TestAppContext) {
    let model = step_flow_model(AuthStepFlowModelPurpose::Registration, cx);
    cx.update(|cx| {
        model.update(cx, |model, cx| {
            model.tag = "race_winner".to_owned();
            model.tag_availability = AuthStepFlowModelTagAvailability::Available;
            model.code = "1234".to_owned();
            model.step = AuthStepFlowModelStep::VerificationCode;

            model.handle_registration_tag_conflict(cx);

            assert_eq!(model.step, AuthStepFlowModelStep::RegistrationTag);
            assert_eq!(model.tag_availability, AuthStepFlowModelTagAvailability::Unavailable);
            assert_eq!(model.error_key(AuthStepFlowModelStep::RegistrationTag), Some("tag_already_exists"));
            assert!(!model.can_continue());
            assert!(model.resume_reusable_verification_code(cx));
            assert_eq!(model.step, AuthStepFlowModelStep::VerificationCode);
            assert_eq!(model.code, "1234");
        });
    });
}

#[gpui::test]
fn delivery_failure_opens_the_code_step_without_alert_or_bot_sheet(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let auth = AuthServicesStore::shared(cx);
        let body = AuthInteractionsStore::send_code_body("7001234567", "+7", Some("registration"), None, cx);

        for refusal in
            [refusal("VERIFY_COUNTRY_NOT_SUPPORTED", &[]), refusal("VERIFY_TEMPORARILY_UNAVAILABLE", &[])]
        {
            auth.update(cx, |auth, cx| auth.reset_auth_flow_presentation(cx));
            let assumed =
                AuthServicesStore::assumed_sms_delivery(&body, &refusal, None).expect("assumed sms delivery");
            auth.update(cx, |auth, cx| {
                auth.handle_send_code_success(
                    assumed,
                    body.phone_number.as_deref().unwrap_or_default(),
                    body.purpose.as_deref(),
                    cx,
                );
            });

            let store = auth.read(cx);
            assert!(store.code_entry_active());
            assert!(!store.telegram_redirect_active());
            assert_eq!(store.auth_flow_error(), None);
            let delivery = store.code_delivery().expect("code delivery");
            assert_eq!(delivery.channel, Some(AuthCodeChannel::Sms));
            assert_eq!(delivery.code_length, VERIFICATION_CODE.maximum_value() as i64);
            assert!(delivery.offers_telegram);
            assert!(delivery.offers_resend());
        }

        for refusal in [
            refusal("VERIFY_TEMPORARILY_UNAVAILABLE", &[("retry_after_s", 5)]),
            refusal("VERIFY_PHONE_INVALID", &[]),
            refusal("COMMON_QUOTA_EXCEEDED", &[("retry_after_s", 60)]),
            prose("Send code failed: Please wait 42 seconds"),
        ] {
            assert!(AuthServicesStore::assumed_sms_delivery(&body, &refusal, None).is_none());
        }

        let bot_body = AuthInteractionsStore::send_code_body(
            "7001234567",
            "+7",
            Some("registration"),
            Some(AuthCodeChannel::TelegramBot),
            cx,
        );
        assert!(
            AuthServicesStore::assumed_sms_delivery(&bot_body, &refusal("VERIFY_TEMPORARILY_UNAVAILABLE", &[]), None)
                .is_none()
        );
    });
}

#[gpui::test]
fn telegram_row_asks_the_bot_and_its_answer_opens_the_bot_sheet(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let auth = AuthServicesStore::shared(cx);
        auth.update(cx, |auth, cx| auth.reset_auth_flow_presentation(cx));
        let body = AuthInteractionsStore::send_code_body("7001234567", "+7", Some("password_reset"), None, cx);
        let assumed =
            AuthServicesStore::assumed_sms_delivery(&body, &refusal("VERIFY_COUNTRY_NOT_SUPPORTED", &[]), None)
                .expect("assumed sms delivery");
        auth.update(cx, |auth, cx| {
            auth.handle_send_code_success(
                assumed,
                body.phone_number.as_deref().unwrap_or_default(),
                body.purpose.as_deref(),
                cx,
            );
        });
        let delivery = auth.read(cx).code_delivery().cloned().expect("code delivery");

        let bot_body =
            AuthInteractionsStore::telegram_code_body(&delivery, "7001234567", "+7", cx).expect("telegram body");
        assert_eq!(bot_body.channel_preference.as_deref(), Some("telegram_bot"));
        assert_eq!(bot_body.purpose.as_deref(), Some("password_reset"));
        assert_eq!(bot_body.phone_number.as_deref(), Some("77001234567"));

        auth.update(cx, |auth, cx| {
            auth.handle_send_code_success(
                AuthSendCodeResponse {
                    channel: "telegram_bot".to_owned(),
                    code_length: 4,
                    success: true,
                    ..Default::default()
                },
                bot_body.phone_number.as_deref().unwrap_or_default(),
                bot_body.purpose.as_deref(),
                cx,
            );
        });
        assert!(auth.read(cx).telegram_redirect_active());
        assert!(!auth.read(cx).code_entry_active());
    });
}

#[gpui::test]
fn resend_cooldown_still_counts_down(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let auth = AuthServicesStore::shared(cx);
        auth.update(cx, |auth, cx| {
            auth.reset_auth_flow_presentation(cx);
            auth.handle_send_code_success(
                AuthSendCodeResponse {
                    channel: "sms".to_owned(),
                    code_length: 4,
                    resend_after_ms: 0,
                    success: true,
                    ..Default::default()
                },
                "77001234567",
                Some("registration"),
                cx,
            );
        });
        let cooldown = refusal("VERIFY_RESEND_TOO_EARLY", &[("retry_after_s", 42)]);
        let body = AuthInteractionsStore::send_code_body(
            "7001234567",
            "+7",
            Some("registration"),
            Some(AuthCodeChannel::Sms),
            cx,
        );
        assert!(
            AuthServicesStore::assumed_sms_delivery(&body, &cooldown, auth.read(cx).code_delivery()).is_none()
        );

        auth.update(cx, |auth, cx| {
            auth.handle_send_code_error(&cooldown, "77001234567", Some("registration"), cx);
        });

        let store = auth.read(cx);
        assert_eq!(store.auth_flow_error(), None);
        assert!(store.code_entry_active());
        let wait = store.code_delivery().expect("code delivery").resend_available_at.time_interval_since_now();
        assert!(wait > 40.0 && wait <= 42.0, "wait {wait}");
    });
}

#[gpui::test]
fn failed_resend_keeps_the_channels_offered_before(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let previous = AuthCodeDelivery {
            phone_digits: "77001234567".to_owned(),
            purpose: Some("registration".to_owned()),
            channel: Some(AuthCodeChannel::Sms),
            destination_masked: String::new(),
            code_length: 4,
            resend_available_at: RiseClock::now(),
            alternatives: vec![AuthCodeChannel::FlashCall, AuthCodeChannel::Viber],
            offers_telegram: true,
            named_by_server: true,
        };
        let body = AuthInteractionsStore::send_code_body(
            "7001234567",
            "+7",
            Some("registration"),
            Some(AuthCodeChannel::Sms),
            cx,
        );
        let assumed = AuthServicesStore::assumed_sms_delivery(
            &body,
            &refusal("VERIFY_TEMPORARILY_UNAVAILABLE", &[]),
            Some(&previous),
        )
        .expect("assumed sms delivery");
        assert_eq!(assumed.channel, "sms");
        assert_eq!(assumed.alternatives, vec!["sms", "flash_call", "viber", "telegram_bot"]);
    });
}

#[test]
fn code_entry_takes_a_whole_inserted_code() {
    assert_eq!(applying("1234", 0, 0, "", 4), "1234");
    assert_eq!(applying("123456", 0, 0, "", 6), "123456");
    assert_eq!(applying("5678", 4, 0, "1234", 4), "5678");
}

#[test]
fn code_entry_absorbs_the_double_insertion() {
    assert_eq!(applying("12341234", 0, 0, "", 4), "1234");
    assert_eq!(applying("1234", 4, 0, "1234", 4), "1234");
    assert_eq!(AuthCodeEntry::normalized("12341234", 4), "1234");
}

#[test]
fn code_entry_drops_everything_but_digits() {
    assert_eq!(applying(" 12-34 ", 0, 0, "", 4), "1234");
    assert_eq!(applying("\u{41a}\u{43e}\u{434}: 1234, 30 \u{43c}\u{438}\u{43d}", 0, 0, "", 4), "1234");
    assert_eq!(applying("\u{663}", 0, 0, "1", 4), "1");
    assert_eq!(AuthCodeEntry::normalized("a1b2c3", 4), "123");
}

#[test]
fn code_entry_replaces_partial_input_with_a_whole_code() {
    assert_eq!(applying("5678", 2, 0, "12", 4), "5678");
    assert_eq!(applying("56", 2, 0, "12", 4), "1256");
}

#[test]
fn code_entry_keeps_typing_and_backspace() {
    assert_eq!(applying("3", 2, 0, "12", 4), "123");
    assert_eq!(applying("5", 4, 0, "1234", 4), "1234");
    assert_eq!(applying("", 2, 1, "123", 4), "12");
    assert_eq!(applying("", 0, 0, "", 4), "");
    assert_eq!(applying("7", 9, 0, "12", 4), "127");
}

#[test]
fn phone_country_catalog_parses_telegram_format() {
    let source = "7;KZ;XXX XXX XX XX;Kazakhstan\n1;US;XXX XXX XXXX;United States\n888;FT;XXXX XXXX;Fragment";
    let countries = PhoneCountryCatalog::parse(source);

    assert_eq!(countries.len(), 2);
    assert_eq!(countries[0].calling_code, "+7");
    assert_eq!(countries[0].region_code, "KZ");
    assert_eq!(countries[1].calling_code, "+1");
}

#[test]
fn bundled_phone_catalog_covers_every_dialable_region() {
    let regions: HashSet<&str> =
        PhoneCountryCatalog::all().iter().map(|country| country.region_code.as_str()).collect();

    assert!(regions.len() >= 245);
    for region in ["AX", "GG", "IM", "JE", "MF", "VA", "YT", "XK"] {
        assert!(regions.contains(region), "{region}");
    }
}

#[test]
fn registration_wire_body_includes_tag_and_boolean_browser_without_gender() {
    let body = AuthRegisterBody::build(
        "77771234567".to_owned(),
        "Rise User".to_owned(),
        "rise_user".to_owned(),
        "1234".to_owned(),
        "strong-password".to_owned(),
        None,
        Some("device-token".to_owned()),
        None,
        None,
    );
    let json = serde_json::to_value(&body).expect("register body");

    assert_eq!(json.get(rise_key::auth::TAG).and_then(Value::as_str), Some("rise_user"));
    assert_eq!(json.get(rise_key::auth::BROWSER).and_then(Value::as_bool), Some(false));
    assert_eq!(json.get(rise_key::auth::DEVICE_TOKEN).and_then(Value::as_str), Some("device-token"));
    assert!(json.get(rise_key::auth::GENDER).is_none());
}

#[test]
fn tag_response_decodes_snake_case_with_shared_decoder_strategy() {
    let response: AuthCheckTagResponse =
        serde_json::from_str(r#"{"success":true,"available":true,"normalized_tag":"rise_only"}"#)
            .expect("tag response");

    assert_eq!(response.normalized_tag.as_deref(), Some("rise_only"));
}

#[test]
fn registration_tag_check_does_not_attach_an_existing_account_token() {
    let url = Url::from_string("https://riseonly.net/api/auth/check-tag").expect("check tag url");
    assert!(!HttpService::request_should_attach_bearer(Some(&url)));
}

#[test]
fn phone_validity_answers_as_the_server_crate() {
    for sample in &PHONE_VALIDITY_SAMPLES {
        assert_eq!(
            PhoneCountryCatalog::is_phone_number_valid(&format!("{}{}", sample.calling_code, sample.national)),
            sample.valid,
            "+{} {}",
            sample.calling_code,
            sample.national
        );
    }
}

#[gpui::test]
fn phone_step_refuses_a_number_with_no_such_operator_code(cx: &mut TestAppContext) {
    let model = step_flow_model(AuthStepFlowModelPurpose::Registration, cx);
    cx.update(|cx| {
        model.update(cx, |model, cx| {
            model.selected_country = phone_country("test-uz", "UZ", "+998", "", "Uzbekistan");
            model.update_phone("587374271", cx);
            model.advance_after_local_validation(cx);
            assert_eq!(model.step, AuthStepFlowModelStep::Phone);
            assert_eq!(model.error_key(AuthStepFlowModelStep::Phone), Some("auth_phone_invalid"));
        });
    });
}
