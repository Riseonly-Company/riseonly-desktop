use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Entity, TestAppContext};
use parking_lot::{Mutex, MutexGuard};
use rise_canon::rise_key::stories as key;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::story::stores::story::story_actions::story_types::{
    StoryPlaybackPolicy, StoryPlayerPresentationStyle,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    StoriesStateDTO, StoryAuthorDTO, StoryDTO, StoryGroupDTO,
};
use crate::modules::story::stores::story::story_services::story_services::{StoryCacheTransform, StoryServicesStore};

static SERIAL: Mutex<()> = Mutex::new(());

fn serialized() -> MutexGuard<'static, ()> {
    SERIAL.lock()
}

struct StoriesPresentationTests;

impl StoriesPresentationTests {
    fn reset(cx: &mut App) -> Entity<StoryServicesStore> {
        let service = StoryServicesStore::shared(cx);
        service.update(cx, |service, cx| service.reset_account_state(cx));
        service
    }

    fn state(author: &str, has_active_stories: bool, has_unseen: bool) -> StoriesStateDTO {
        StoriesStateDTO { has_active_stories, has_unseen, user_id: author.to_owned(), ..StoriesStateDTO::default() }
    }

    fn known(service: &Entity<StoryServicesStore>, user_id: &str, cx: &mut App) -> bool {
        service.update(cx, |service, cx| service.has_known_stories_state(Some(user_id), cx))
    }

    fn active(service: &Entity<StoryServicesStore>, user_id: &str, cx: &mut App) -> bool {
        service.update(cx, |service, cx| service.has_active_stories(Some(user_id), cx))
    }

    fn unseen(service: &Entity<StoryServicesStore>, user_id: &str, cx: &mut App) -> bool {
        service.update(cx, |service, cx| service.has_unseen_stories(Some(user_id), cx))
    }

    fn group(author_id: &str, story_id: i64, unseen: bool) -> StoryGroupDTO {
        let author = StoryAuthorDTO {
            id: author_id.to_owned(),
            is_official: false,
            is_online: false,
            is_premium: false,
            last_seen_ms: None,
            logo: String::new(),
            name: author_id.to_owned(),
            premium_badge_emoji: None,
            tag: author_id.to_owned(),
            ..StoryAuthorDTO::default()
        };
        let story = StoryDTO {
            allow_screenshots: true,
            audience: "everyone".to_owned(),
            author: Some(author.clone()),
            author_id: author_id.to_owned(),
            caption: None,
            created_at_ms: 1,
            duration_days: 1,
            expires_at_ms: i64::MAX,
            file_id: format!("file-{story_id}"),
            has_viewed: Some(!unseen),
            id: story_id,
            is_liked: false,
            like_count: 0,
            media_type: "image".to_owned(),
            post_to_profile: false,
            view_count: 0,
            ..StoryDTO::default()
        };
        StoryGroupDTO { all_seen: !unseen, author: Some(author), stories: vec![story], unseen_count: i64::from(unseen) }
    }
}

#[test]
fn story_picker_transitions_without_an_export_progress_overlay() {
    assert!(!MediaPickerConfiguration::story().shows_export_progress_overlay);
}

#[test]
fn playback_policy_uses_fifteen_second_images_and_five_item_prefetch_window() {
    assert!((StoryPlaybackPolicy::IMAGE_DURATION - 15.0).abs() < f64::EPSILON);
    assert_eq!(StoryPlaybackPolicy::PREFETCH_COUNT, 5);
    assert_eq!(StoryPlaybackPolicy::IMAGE_MAXIMUM_PIXEL_DIMENSION, 2_048);
}

#[gpui::test]
fn state_distinguishes_unknown_from_known_empty(cx: &mut TestAppContext) {
    let _serial = serialized();
    cx.update(|cx| {
        let service = StoriesPresentationTests::reset(cx);

        assert!(!StoriesPresentationTests::known(&service, "author", cx));
        service.update(cx, |service, cx| {
            service.apply_stories_state(vec![StoriesPresentationTests::state("author", false, false)], &[], cx);
        });

        assert!(StoriesPresentationTests::known(&service, "author", cx));
        assert!(!StoriesPresentationTests::active(&service, "author", cx));
        assert!(!StoriesPresentationTests::unseen(&service, "author", cx));
        StoriesPresentationTests::reset(cx);
    });
}

#[gpui::test]
fn empty_state_response_clears_previously_active_requested_users(cx: &mut TestAppContext) {
    let _serial = serialized();
    cx.update(|cx| {
        let service = StoriesPresentationTests::reset(cx);

        service.update(cx, |service, cx| {
            service.apply_stories_state(vec![StoriesPresentationTests::state("author", true, true)], &[], cx);
        });
        assert!(StoriesPresentationTests::active(&service, "author", cx));

        service.update(cx, |service, cx| service.apply_stories_state(Vec::new(), &["author".to_owned()], cx));

        assert!(StoriesPresentationTests::known(&service, "author", cx));
        assert!(!StoriesPresentationTests::active(&service, "author", cx));
        assert!(!StoriesPresentationTests::unseen(&service, "author", cx));
        StoriesPresentationTests::reset(cx);
    });
}

#[gpui::test]
fn feed_hydrates_rings_and_opens_at_tapped_author(cx: &mut TestAppContext) {
    let _serial = serialized();
    cx.update(|cx| {
        let service = StoriesPresentationTests::reset(cx);

        let first = StoriesPresentationTests::group("first", 1, false);
        let second = StoriesPresentationTests::group("second", 2, true);
        service.update(cx, |service, cx| service.open_stories_feed(vec![first, second], 1, cx));

        let request = service.read(cx).player_request.clone().expect("a player request");
        assert_eq!(request.groups.len(), 2);
        assert_eq!(request.start_group_index, 1);
        assert_eq!(request.presentation_style, StoryPlayerPresentationStyle::FeedCube);
        assert!(StoriesPresentationTests::active(&service, "first", cx));
        assert!(StoriesPresentationTests::active(&service, "second", cx));
        assert!(!StoriesPresentationTests::unseen(&service, "first", cx));
        assert!(StoriesPresentationTests::unseen(&service, "second", cx));
        StoriesPresentationTests::reset(cx);
    });
}

#[gpui::test]
fn empty_feed_refresh_clears_authors_from_previous_feed(cx: &mut TestAppContext) {
    let _serial = serialized();
    cx.update(|cx| {
        let service = StoriesPresentationTests::reset(cx);

        service.update(cx, |service, cx| {
            service.apply_stories_feed(vec![StoriesPresentationTests::group("author", 1, true)], true, false, cx);
        });
        assert!(StoriesPresentationTests::active(&service, "author", cx));

        service.update(cx, |service, cx| service.apply_stories_feed(Vec::new(), true, false, cx));

        assert!(StoriesPresentationTests::known(&service, "author", cx));
        assert!(!StoriesPresentationTests::active(&service, "author", cx));
        assert!(!StoriesPresentationTests::unseen(&service, "author", cx));
        StoriesPresentationTests::reset(cx);
    });
}

#[gpui::test]
fn viewing_last_story_turns_author_ring_seen_immediately(cx: &mut TestAppContext) {
    let _serial = serialized();
    cx.update(|cx| {
        let service = StoriesPresentationTests::reset(cx);

        let group = StoriesPresentationTests::group("author", 42, true);
        let story = group.stories.first().cloned().expect("a story");
        service.update(cx, |service, cx| service.apply_stories_feed(vec![group], true, false, cx));

        assert!(StoriesPresentationTests::unseen(&service, "author", cx));
        service.update(cx, |service, cx| {
            service.mark_story_viewed_locally(story.id, "author", std::slice::from_ref(&story), cx);
        });

        assert!(service.update(cx, |service, cx| service.is_story_viewed(&story, cx)));
        assert!(!StoriesPresentationTests::unseen(&service, "author", cx));

        service.update(cx, |service, cx| {
            service.apply_stories_state(vec![StoriesPresentationTests::state("author", true, true)], &[], cx);
        });
        assert!(!StoriesPresentationTests::unseen(&service, "author", cx));

        service.update(cx, |service, cx| {
            service.apply_stories_feed(vec![StoriesPresentationTests::group("author", 43, true)], false, false, cx);
        });
        assert!(StoriesPresentationTests::unseen(&service, "author", cx));
        StoriesPresentationTests::reset(cx);
    });
}

#[test]
fn story_acknowledgement_replaces_temporary_row_without_duplicate() {
    let temporary = StoriesPresentationTests::group("author", -1, false).stories.first().cloned().expect("a story");
    let authoritative = StoriesPresentationTests::group("author", 42, false).stories.first().cloned().expect("a story");
    let mut initial = Map::new();
    initial.insert(key::SUCCESS.to_owned(), Value::Bool(true));
    initial.insert(key::LIST.to_owned(), Value::Array(Vec::new()));

    let temporary_id = temporary.id;
    let authoritative_id = authoritative.id;
    let optimistic = StoryServicesStore::upsert_story_transform(temporary, None)(Some(initial));
    let acknowledged = StoryServicesStore::upsert_story_transform(authoritative, Some(temporary_id))(optimistic);
    let rows = AnyDictionaryBridge::array_of_dictionaries(acknowledged.as_ref().and_then(|root| root.get(key::LIST)));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows.first().and_then(|row| row.get(key::ID)).and_then(Value::as_i64), Some(authoritative_id));
}

#[gpui::test]
fn later_story_mutation_survives_earlier_rollback(cx: &mut TestAppContext) {
    let _serial = serialized();
    cx.update(|cx| {
        let service = StoriesPresentationTests::reset(cx);
        let cache_key = SaiCache::new(["storyMutationConcurrencyTest"]).key;
        let mut root = Map::new();
        root.insert("values".to_owned(), Value::Array(Vec::new()));
        SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::Cache, cx).expect("the root is replaced");

        let append = |value: i64| -> StoryCacheTransform {
            Rc::new(move |existing: Option<Map<String, Value>>| {
                let mut root = existing.unwrap_or_default();
                let mut values = root.get("values").and_then(Value::as_array).cloned().unwrap_or_default();
                values.push(Value::from(value));
                root.insert("values".to_owned(), Value::Array(values));
                Some(root)
            })
        };

        let first = service.update(cx, |service, cx| {
            service.begin_story_mutation(HashMap::from([(cache_key.clone(), append(1))]), cx)
        });
        let second = service.update(cx, |service, cx| {
            service.begin_story_mutation(HashMap::from([(cache_key.clone(), append(2))]), cx)
        });
        service.update(cx, |service, cx| {
            service.rollback_story_mutation(&first, cx);
            service.commit_story_mutation(&second, HashMap::new(), cx);
        });

        let payload = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        let root = AnyDictionaryBridge::dictionary(payload.as_ref()).expect("a cached root");
        let values = root.get("values").and_then(Value::as_array).cloned().unwrap_or_default();
        assert_eq!(values, vec![Value::from(2)]);

        StoriesPresentationTests::reset(cx);
        SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
    });
}
