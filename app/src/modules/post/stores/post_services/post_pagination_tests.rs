use gpui::TestAppContext;
use rise_canon::rise_key::post as key;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsDiskCachePolicy;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

struct PostPaginationTests;

impl PostPaginationTests {
    fn row(entries: Vec<(&str, Value)>) -> Value {
        Value::Object(entries.into_iter().map(|(name, value)| (name.to_owned(), value)).collect::<Map<String, Value>>())
    }
}

#[gpui::test]
fn cached_feed_page_supports_wire_ids_and_optimistic_rows(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let cache_key = SaiCache::new(["post-pagination-test".to_owned(), Uuid::new_v4().uuid_string()]).key;

        let mut root = Map::new();
        root.insert(
            key::LIST.to_owned(),
            Value::Array(vec![
                PostPaginationTests::row(vec![(key::ID, Value::from(101)), (key::TITLE, Value::from("wire"))]),
                PostPaginationTests::row(vec![
                    (key::ID, Value::from("temp-post")),
                    (key::TITLE, Value::from("optimistic")),
                    (RiseLocalKey::IS_TEMP, Value::Bool(true)),
                ]),
            ]),
        );
        root.insert(key::RELATIVE_ID.to_owned(), Value::from(101));
        root.insert(key::IS_HAVE_MORE.to_owned(), Value::Bool(true));
        root.insert(key::FEED_SESSION_ID.to_owned(), Value::from("session"));
        SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::Cache, cx).expect("root replaced");

        let page = PostServicesStore::shared(cx)
            .update(cx, |services, cx| {
                services.cached_feed_page(&cache_key, SaiWsDiskCachePolicy::PreserveExisting, cx)
            })
            .expect("cached page");

        let ids: Vec<&str> = page.list.iter().map(|post| post.id.as_str()).collect();
        assert_eq!(ids, vec!["101", "temp-post"]);
        assert_eq!(page.list.last().map(|post| post.is_temp), Some(true));
        assert_eq!(page.relative_id.as_deref(), Some("101"));
        assert_eq!(page.is_have_more, Some(true));
        assert_eq!(page.feed_session_id.as_deref(), Some("session"));

        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}
