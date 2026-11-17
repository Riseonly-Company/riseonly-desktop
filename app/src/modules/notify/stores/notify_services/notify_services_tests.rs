use gpui::TestAppContext;
use rise_canon::rise_key;
use serde_json::{Map, Value};

use super::notify_services::NotifyServicesStore;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsResponseError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;

fn patch(field: &str, value: bool) -> Map<String, Value> {
    Map::from_iter([(field.to_owned(), Value::Bool(value))])
}

#[gpui::test]
fn notification_settings_rollback_does_not_overwrite_newer_field_edit(cx: &mut TestAppContext) {
    let service = cx.update(NotifyServicesStore::shared);
    let cache_key = NotifyServicesStore::notification_settings_cache_key();
    let all_enabled = rise_key::notify::ALL_ENABLED;
    let music_enabled = rise_key::notify::MUSIC_ENABLED;
    service.update(cx, |service, cx| service.reset_account_state(cx));
    cx.update(|cx| {
        let mut root = patch(all_enabled, true);
        root.extend(patch(music_enabled, true));
        SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx).expect("the test root is written");
    });

    service.update(cx, |service, cx| {
        service.stage_notification_settings_patch(patch(all_enabled, false), cx);
        let first = service
            .take_pending_notification_settings_mutation(cx)
            .expect("the staged patch is pending");
        service.stage_notification_settings_patch(patch(all_enabled, false), cx);
        service.stage_notification_settings_patch(patch(music_enabled, false), cx);
        service.finish_notification_settings_mutation(&first, false, cx);
    });

    let root = cx.update(|cx| {
        let payload = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        AnyDictionaryBridge::dictionary(payload.as_ref()).expect("the cached root stays a dictionary")
    });
    service.update(cx, |service, cx| service.reset_account_state(cx));
    cx.update(|cx| SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx));

    assert_eq!(root.get(all_enabled).and_then(Value::as_bool), Some(false));
    assert_eq!(root.get(music_enabled).and_then(Value::as_bool), Some(false));
}

#[gpui::test]
fn notification_settings_only_revalidate_version_conflicts(cx: &mut TestAppContext) {
    let service = cx.update(NotifyServicesStore::shared);
    let conflict = anyhow::Error::new(SaiWsResponseError {
        message: String::new(),
        code: Some("409".to_owned()),
        ..SaiWsResponseError::default()
    });
    let offline = anyhow::anyhow!("offline");

    service.read_with(cx, |service, _| {
        assert!(service.notification_settings_error_requires_revalidation(&conflict));
        assert!(!service.notification_settings_error_requires_revalidation(&offline));
    });
}
