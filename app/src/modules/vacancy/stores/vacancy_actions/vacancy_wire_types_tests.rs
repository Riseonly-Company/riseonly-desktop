use rise_canon::rise_key::vacancy as key;
use serde_json::{Map, Value};

use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::vacancy::stores::vacancy_actions::vacancy_wire_types::{WireApplicationsListDTO, WireResumeListDTO};
use crate::modules::vacancy::stores::vacancy_services::vacancy_services::VacancyServicesStore;

fn page_json(total: i64, relative_id: Value, is_have_more: bool) -> Value {
    let mut page = Map::new();
    page.insert(key::LIST.to_owned(), Value::Array(Vec::new()));
    page.insert(key::TOTAL.to_owned(), Value::from(total));
    page.insert(key::LIMIT.to_owned(), Value::from(20));
    page.insert(key::RELATIVE_ID.to_owned(), relative_id);
    page.insert(key::IS_HAVE_MORE.to_owned(), Value::Bool(is_have_more));
    Value::Object(page)
}

#[test]
fn vacancy_cache_key_scopes_every_server_filter_and_viewer() {
    let base = VacancyServicesStore::vacancy_list_cache_key(
        Some("swift"),
        Some("full_time"),
        Some("remote"),
        Some(100),
        Some(500),
        Some("Almaty"),
        Some(true),
        Some("viewer-a"),
    );

    assert_ne!(
        base,
        VacancyServicesStore::vacancy_list_cache_key(
            Some("swift"),
            Some("full_time"),
            Some("remote"),
            Some(101),
            Some(500),
            Some("Almaty"),
            Some(true),
            Some("viewer-a"),
        )
    );
    assert_ne!(
        base,
        VacancyServicesStore::vacancy_list_cache_key(
            Some("swift"),
            Some("full_time"),
            Some("remote"),
            Some(100),
            Some(500),
            Some("Astana"),
            Some(true),
            Some("viewer-a"),
        )
    );
    assert_ne!(
        base,
        VacancyServicesStore::vacancy_list_cache_key(
            Some("swift"),
            Some("full_time"),
            Some("remote"),
            Some(100),
            Some(500),
            Some("Almaty"),
            Some(true),
            Some("viewer-b"),
        )
    );
    assert_eq!(
        base,
        VacancyServicesStore::vacancy_list_cache_key(
            Some("  swift  "),
            Some(" full_time "),
            Some(" remote "),
            Some(100),
            Some(500),
            Some(" Almaty "),
            Some(true),
            Some("viewer-a"),
        )
    );
}

#[test]
fn bot_mini_app_cache_keys_scope_owned_and_public_lists() {
    assert_ne!(
        BotServicesStore::mini_apps_cache_key("owner-a", "bot"),
        BotServicesStore::mini_apps_cache_key("owner-b", "bot")
    );
    assert_ne!(
        BotServicesStore::public_mini_apps_cache_key(Some("bot"), None, None),
        BotServicesStore::public_mini_apps_cache_key(None, Some("helperbot"), None)
    );
}

#[test]
fn vacancy_application_page_decodes_pagination_metadata() -> serde_json::Result<()> {
    let page: WireApplicationsListDTO = serde_json::from_value(page_json(0, Value::Null, false))?;

    assert_eq!(page.relative_id, None);
    assert!(!page.is_have_more);
    Ok(())
}

#[test]
fn vacancy_resume_page_decodes_pagination_metadata() -> serde_json::Result<()> {
    let page: WireResumeListDTO = serde_json::from_value(page_json(21, Value::from(42), true))?;

    assert_eq!(page.relative_id, Some(42));
    assert!(page.is_have_more);
    Ok(())
}
