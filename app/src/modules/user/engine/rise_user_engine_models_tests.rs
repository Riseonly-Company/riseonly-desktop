use std::collections::HashSet;

use rise_canon::rise_key::user as user_key;
use rise_canon::rise_request::user::{GetThemeSettings, UpdateThemeSettings, update_theme_settings};
use rise_canon::rise_response::user::GetThemeSettings as GetThemeSettingsResponse;
use rise_canon::rise_wire_methods;
use rise_endpoints::ENDPOINTS;
use rise_engine::{CancellationError, RiseWireReplayPolicy};
use rise_theme::{RiseThemeAppearance, RiseThemePlatform, RiseThemePreferences, ThemePalette};
use serde_json::{Map, Value};

use crate::modules::user::engine::core::rise_user_rpc::RiseUserEndpoints;
use crate::modules::user::engine::core::rise_user_theme_settings_resource_repository::{
    RiseUserThemeSettingsPersistedCodec, RiseUserThemeSettingsPersistedState,
};
use crate::modules::user::engine::core::rise_user_theme_wire_models::ThemeSettingsPreferences;
use crate::modules::user::engine::rise_user_engine_models::{
    RiseUserSettingsError, RiseUserThemeMutationBuffer, RiseUserThemeMutationFailurePolicy,
};

fn json_object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        other => panic!("expected a JSON object, got {other}"),
    }
}

#[test]
fn light_palette_keeps_telegram_day_classic_semantic_roles() {
    let palette = ThemePalette::default_light();

    assert_eq!(palette.bg_100, "#FFFFFF");
    assert_eq!(palette.bg_300, "#F2F2F7");
    assert_eq!(palette.bg_400, "#E5E5EA");
    assert_eq!(palette.bg_600, "#EFEFF4");
    assert_eq!(palette.border_100, "#C8C7CC");
    assert_eq!(palette.border_300, "#C7C7CC");
    assert_eq!(palette.text_100, "#000000");
    assert_eq!(palette.secondary_100, "#8E8E93");
    assert_eq!(palette.success_200, "#35C759");
    assert_eq!(palette.error_100, "#FF3B30");
    assert_eq!(palette.input_bg_100, "#F2F2F7");
    assert_eq!(palette.primary_100, "#FF5A5A");
}

#[test]
fn platform_preference_updates_stay_isolated() {
    let original = RiseThemePreferences::new(
        Some(RiseThemeAppearance::Dark),
        Some(RiseThemeAppearance::Light),
        Some(RiseThemeAppearance::Dark),
    );

    let updated = original.setting(Some(RiseThemeAppearance::Light), RiseThemePlatform::Mobile);
    let system_web = updated.setting(None, RiseThemePlatform::Web);

    assert_eq!(updated.mobile, Some(RiseThemeAppearance::Light));
    assert_eq!(updated.desktop, Some(RiseThemeAppearance::Light));
    assert_eq!(updated.web, Some(RiseThemeAppearance::Dark));
    assert_eq!(system_web.mobile, Some(RiseThemeAppearance::Light));
    assert_eq!(system_web.desktop, Some(RiseThemeAppearance::Light));
    assert_eq!(system_web.web, None);
}

#[test]
fn endpoint_catalogue_owns_theme_settings_contracts() {
    let all = RiseUserEndpoints::all();
    let catalogue: HashSet<&str> = all.iter().map(|endpoint| endpoint.method.as_str()).collect();
    assert_eq!(catalogue.len(), all.len());
    let canon_user: Vec<_> = ENDPOINTS
        .iter()
        .filter(|entry| entry.service == rise_wire_methods::user::SERVICE)
        .collect();
    let canon_user_methods: HashSet<&str> = canon_user.iter().map(|entry| entry.method).collect();
    assert!(catalogue.is_subset(&canon_user_methods));
    let catalogued_user_methods: HashSet<&str> = canon_user
        .iter()
        .filter(|entry| entry.catalog == Some(rise_wire_methods::user::SERVICE))
        .map(|entry| entry.method)
        .collect();
    assert!(catalogue.is_superset(&catalogued_user_methods));
    assert!(catalogue.is_superset(&HashSet::from([
        rise_wire_methods::user::GET_THEME_SETTINGS,
        rise_wire_methods::user::UPDATE_THEME_SETTINGS,
    ])));
    assert!(
        all.iter()
            .all(|endpoint| endpoint.descriptor().service == rise_wire_methods::user::SERVICE)
    );
    assert_eq!(
        RiseUserEndpoints::get_theme_settings().descriptor().replay_policy,
        RiseWireReplayPolicy::ReadOnly
    );
    assert_eq!(
        RiseUserEndpoints::update_theme_settings().descriptor().replay_policy,
        RiseWireReplayPolicy::Never
    );
}

#[test]
fn theme_settings_read_carries_nothing_because_identity_is_injected() -> anyhow::Result<()> {
    let object = json_object(serde_json::to_value(GetThemeSettings {})?);

    assert!(object.get(user_key::USER_ID).is_none());
    Ok(())
}

#[test]
fn clearing_theme_override_omits_the_theme() -> anyhow::Result<()> {
    let request = UpdateThemeSettings {
        platform: Some(update_theme_settings::Platform::Mobile),
        theme: None,
    };
    let object = json_object(serde_json::to_value(&request)?);

    assert!(object.get(user_key::USER_ID).is_none());
    assert_eq!(object.get(user_key::PLATFORM), Some(&serde_json::to_value(RiseThemePlatform::Mobile)?));
    assert!(object.get(user_key::THEME).is_none());
    Ok(())
}

#[test]
fn explicit_theme_override_uses_gateway_raw_values() -> anyhow::Result<()> {
    let request = UpdateThemeSettings {
        platform: Some(update_theme_settings::Platform::Desktop),
        theme: Some(update_theme_settings::Theme::Dark),
    };
    let object = json_object(serde_json::to_value(&request)?);

    assert!(object.get(user_key::USER_ID).is_none());
    assert_eq!(object.get(user_key::PLATFORM), Some(&serde_json::to_value(RiseThemePlatform::Desktop)?));
    assert_eq!(
        object.get(user_key::THEME).and_then(Value::as_str),
        Some(RiseThemeAppearance::Dark.raw_value())
    );
    Ok(())
}

#[test]
fn full_gateway_snapshot_decodes_nullable_platform_overrides() -> anyhow::Result<()> {
    let mut settings = Map::new();
    settings.insert(user_key::MOBILE_THEME.to_owned(), Value::from(RiseThemeAppearance::Light.raw_value()));
    settings.insert(user_key::DESKTOP_THEME.to_owned(), Value::from(RiseThemeAppearance::Dark.raw_value()));
    settings.insert(user_key::WEB_THEME.to_owned(), Value::Null);
    let mut payload = Map::new();
    payload.insert(user_key::SUCCESS.to_owned(), Value::Bool(true));
    payload.insert(user_key::ERROR.to_owned(), Value::from(String::new()));
    payload.insert(user_key::SETTINGS.to_owned(), Value::Object(settings));

    let response: GetThemeSettingsResponse = serde_json::from_value(Value::Object(payload))?;

    assert!(response.success);
    assert_eq!(
        response.settings.as_ref().map(ThemeSettingsPreferences::preferences),
        Some(RiseThemePreferences::new(
            Some(RiseThemeAppearance::Light),
            Some(RiseThemeAppearance::Dark),
            None,
        ))
    );
    Ok(())
}

#[test]
fn latest_intent_coalesces_per_platform_before_sending() {
    let mut buffer = RiseUserThemeMutationBuffer::default();
    let first = buffer.enqueue(RiseThemePlatform::Mobile, Some(RiseThemeAppearance::Dark));
    let latest = buffer.enqueue(RiseThemePlatform::Mobile, Some(RiseThemeAppearance::Light));

    let first_ready = buffer.mark_ready(&first);
    let latest_ready = buffer.mark_ready(&latest);
    let pending = buffer.begin_next();

    assert!(!first_ready);
    assert!(latest_ready);
    let Some(next) = pending else {
        panic!("a ready intent must begin");
    };
    assert_eq!(next.generation, latest.generation);
    assert_eq!(next.appearance, Some(RiseThemeAppearance::Light));
    assert_eq!(buffer.mutating_platforms(), HashSet::from([RiseThemePlatform::Mobile]));
}

#[test]
fn only_persisted_intents_enter_the_mutation_lane() {
    let mut buffer = RiseUserThemeMutationBuffer::default();
    buffer.enqueue(RiseThemePlatform::Mobile, Some(RiseThemeAppearance::Dark));
    let desktop = buffer.enqueue(RiseThemePlatform::Desktop, Some(RiseThemeAppearance::Light));
    let desktop_ready = buffer.mark_ready(&desktop);
    let pending = buffer.begin_next();

    assert!(desktop_ready);
    let Some(next) = pending else {
        panic!("a ready intent must begin");
    };
    assert_eq!(next.platform, RiseThemePlatform::Desktop);
    assert_eq!(next.appearance, Some(RiseThemeAppearance::Light));
}

#[test]
fn successful_response_elides_a_queued_equivalent_intent() {
    let mut buffer = RiseUserThemeMutationBuffer::default();
    let first = buffer.enqueue(RiseThemePlatform::Mobile, Some(RiseThemeAppearance::Dark));
    let first_ready = buffer.mark_ready(&first);
    let pending = buffer.begin_next();
    assert!(first_ready);
    let Some(in_flight) = pending else {
        panic!("a ready intent must begin");
    };
    buffer.enqueue(RiseThemePlatform::Mobile, Some(RiseThemeAppearance::Dark));

    let did_finish = buffer.finish(&in_flight);
    let discarded = buffer.discard_intents_satisfied(&RiseThemePreferences::new(
        Some(RiseThemeAppearance::Dark),
        None,
        None,
    ));

    assert!(did_finish);
    assert_eq!(discarded, HashSet::from([RiseThemePlatform::Mobile]));
    assert!(!buffer.is_mutating());
}

#[test]
fn failed_generation_rolls_back_without_dropping_newer_intent() {
    let mut buffer = RiseUserThemeMutationBuffer::default();
    let failed = buffer.enqueue(RiseThemePlatform::Mobile, Some(RiseThemeAppearance::Dark));
    let failed_ready = buffer.mark_ready(&failed);
    let pending = buffer.begin_next();
    assert!(failed_ready);
    let Some(in_flight) = pending else {
        panic!("a ready intent must begin");
    };
    buffer.enqueue(RiseThemePlatform::Desktop, Some(RiseThemeAppearance::Light));

    let committed = RiseThemePreferences::new(Some(RiseThemeAppearance::Light), None, None);
    let optimistic = buffer.applying_optimistic(&committed);
    assert_eq!(optimistic.mobile, Some(RiseThemeAppearance::Dark));
    assert_eq!(optimistic.desktop, Some(RiseThemeAppearance::Light));

    let did_finish = buffer.finish(&in_flight);
    let rolled_back = buffer.applying_optimistic(&committed);
    assert!(did_finish);
    assert_eq!(rolled_back.mobile, Some(RiseThemeAppearance::Light));
    assert_eq!(rolled_back.desktop, Some(RiseThemeAppearance::Light));
    assert_eq!(buffer.mutating_platforms(), HashSet::from([RiseThemePlatform::Desktop]));
}

#[test]
fn crash_before_acknowledgement_recovers_committed_preferences() -> anyhow::Result<()> {
    let committed = RiseThemePreferences::new(
        Some(RiseThemeAppearance::Light),
        Some(RiseThemeAppearance::Dark),
        None,
    );
    let optimistic = committed.setting(Some(RiseThemeAppearance::Dark), RiseThemePlatform::Mobile);
    let persisted = RiseUserThemeSettingsPersistedState::new(committed, Some(optimistic));

    let encoded = RiseUserThemeSettingsPersistedCodec::encode(&persisted)?;
    let recovered = RiseUserThemeSettingsPersistedCodec::decode(&encoded)?;

    assert!(recovered.requires_optimistic_rollback());
    assert_eq!(recovered.preferences_for_activation(), committed);
    assert_ne!(recovered.preferences_for_activation(), optimistic);

    let cleared = RiseUserThemeSettingsPersistedState::new(recovered.preferences_for_activation(), None);
    let cleared_data = RiseUserThemeSettingsPersistedCodec::encode(&cleared)?;
    let restored = RiseUserThemeSettingsPersistedCodec::decode(&cleared_data)?;

    assert!(!restored.requires_optimistic_rollback());
    assert_eq!(restored.preferences_for_activation(), committed);
    Ok(())
}

#[test]
fn legacy_preferences_decode_as_committed_state() -> anyhow::Result<()> {
    let legacy = RiseThemePreferences::new(
        Some(RiseThemeAppearance::Dark),
        None,
        Some(RiseThemeAppearance::Light),
    );
    let data = serde_json::to_vec(&legacy)?;
    let recovered = RiseUserThemeSettingsPersistedCodec::decode(&data)?;

    assert_eq!(recovered.preferences_for_activation(), legacy);
    assert!(!recovered.requires_optimistic_rollback());
    Ok(())
}

#[test]
fn ambiguous_mutation_failure_requires_authoritative_read() {
    assert!(RiseUserThemeMutationFailurePolicy::requires_authoritative_refresh(
        &anyhow::Error::new(RiseUserSettingsError::InvalidResponse)
    ));
    assert!(!RiseUserThemeMutationFailurePolicy::requires_authoritative_refresh(
        &anyhow::Error::new(CancellationError)
    ));
}
