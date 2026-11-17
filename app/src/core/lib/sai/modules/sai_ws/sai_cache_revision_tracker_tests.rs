use std::cell::Cell;

use gpui::TestAppContext;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Value, json};

use super::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use super::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use super::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;

fn revision(key: &str, cx: &mut gpui::App) -> i64 {
    SaiCacheRevisionTracker::shared(cx).read(cx).revision(key)
}

#[gpui::test]
fn hydration_invalidates_observers_only_for_a_new_object(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let key = format!("test.hydration.{}", Uuid::new_v4().uuid_string());
        let first_payload = json!({ "value": 1 });
        let initial_revision = revision(&key, cx);

        SaiWsResponseCache::hydrate_memory(&key, first_payload.clone(), false, cx);
        assert_eq!(revision(&key, cx), initial_revision + 1);

        SaiWsResponseCache::hydrate_memory(&key, first_payload.clone(), false, cx);
        assert_eq!(revision(&key, cx), initial_revision + 1);

        SaiWsResponseCache::hydrate_memory(&key, json!({ "value": 2 }), false, cx);
        assert_eq!(revision(&key, cx), initial_revision + 2);

        SaiWsResponseCache::remove(&key, SaiCacheCleanLayer::Memory, cx);
    });
}

#[gpui::test]
fn observed_projection_memoizes_by_framework_revision(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let key = format!("test.projection.{}", Uuid::new_v4().uuid_string());
        let transformations = Cell::new(0);
        SaiWsResponseCache::hydrate_memory(&key, json!({ "value": 1 }), false, cx);

        let projected_value = |cx: &mut gpui::App| -> i64 {
            SaiWsResponseCache::observed_projection(
                &key,
                "test.int.v1",
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                |payload: Option<&Value>| {
                    transformations.set(transformations.get() + 1);
                    payload
                        .and_then(|payload| payload.get("value"))
                        .and_then(Value::as_i64)
                        .unwrap_or(0)
                },
                cx,
            )
        };

        assert_eq!(projected_value(cx), 1);
        assert_eq!(projected_value(cx), 1);
        assert_eq!(transformations.get(), 1);

        SaiWsResponseCache::hydrate_memory(&key, json!({ "value": 2 }), false, cx);
        assert_eq!(projected_value(cx), 2);
        assert_eq!(transformations.get(), 2);

        SaiWsResponseCache::remove(&key, SaiCacheCleanLayer::Memory, cx);
    });
}
