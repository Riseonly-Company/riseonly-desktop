use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{App, Context};
use rise_canon::rise_key::stories as key;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_uuid::Uuid;
use crate::modules::media_picker::editor::story_canvas_model::StoryCanvasGeometry;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;
use crate::modules::story::shared::story_guide_sandbox_hooks::story_guide_sandbox_hooks;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::{
    GetProfileStoriesGatewayDTO, StoryProfileScope,
};
use crate::modules::story::stores::story::story_actions::story_types::{
    StoryPlayerEntrance, StoryPlayerRequest, StoryPlayerSharedElementOrigin,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    StoryAuthorDTO, StoryDTO, StoryDTOExt, StoryGroupDTO,
};
use crate::modules::story::stores::story::story_services::story_services::{StoryCacheTransform, StoryServicesStore};

impl StoryServicesStore {
    pub fn profile_stories_cache_key(target_user_id: &str, scope: StoryProfileScope) -> String {
        SaiCache::new(["getProfileStories", target_user_id, scope.raw_value()]).key
    }

    pub fn profile_stories(&self, target_user_id: &str, scope: StoryProfileScope, cx: &mut App) -> Vec<StoryDTO> {
        if let Some(invented) = (story_guide_sandbox_hooks().profile_stories)(target_user_id, scope, cx) {
            return invented;
        }
        self.decode_gateway::<GetProfileStoriesGatewayDTO>(&Self::profile_stories_cache_key(target_user_id, scope), cx)
            .map(|gateway| gateway.list)
            .unwrap_or_default()
    }

    pub fn profile_stories_loaded(&self, target_user_id: &str, scope: StoryProfileScope, cx: &mut App) -> bool {
        if (story_guide_sandbox_hooks().profile_stories)(target_user_id, scope, cx).is_some() {
            return true;
        }
        self.decode_gateway::<GetProfileStoriesGatewayDTO>(&Self::profile_stories_cache_key(target_user_id, scope), cx)
            .is_some()
    }

    pub fn profile_stories_has_more(&self, target_user_id: &str, scope: StoryProfileScope, cx: &mut App) -> bool {
        if (story_guide_sandbox_hooks().profile_stories)(target_user_id, scope, cx).is_some() {
            return false;
        }
        self.decode_gateway::<GetProfileStoriesGatewayDTO>(&Self::profile_stories_cache_key(target_user_id, scope), cx)
            .is_some_and(|gateway| gateway.has_more)
    }

    pub fn profile_stories_next_cursor(
        &self,
        target_user_id: &str,
        scope: StoryProfileScope,
        cx: &mut App,
    ) -> Option<i64> {
        self.decode_gateway::<GetProfileStoriesGatewayDTO>(&Self::profile_stories_cache_key(target_user_id, scope), cx)
            .and_then(|gateway| gateway.relative_id)
    }

    pub fn highlight_picker_stories(&self, owner_id: &str, cx: &mut App) -> Vec<StoryDTO> {
        let edge = self
            .highlight_picker_paging_scope(owner_id, cx)
            .and_then(|scope| self.profile_stories_next_cursor(owner_id, scope, cx))
            .unwrap_or(0);
        self.highlight_picker_stories_from_edge(owner_id, edge, cx)
    }

    pub fn highlight_picker_stories_from_edge(&self, owner_id: &str, edge: i64, cx: &mut App) -> Vec<StoryDTO> {
        let mut seen = HashSet::new();
        let mut stories: Vec<StoryDTO> = self
            .profile_stories(owner_id, StoryProfileScope::Profile, cx)
            .into_iter()
            .chain(self.profile_stories(owner_id, StoryProfileScope::Archive, cx))
            .filter(|story| story.id >= edge && seen.insert(story.id))
            .collect();
        stories.sort_by(|lhs, rhs| rhs.id.cmp(&lhs.id));
        stories
    }

    pub fn highlight_picker_stories_loaded(&self, owner_id: &str, cx: &mut App) -> bool {
        self.profile_stories_loaded(owner_id, StoryProfileScope::Profile, cx)
            && self.profile_stories_loaded(owner_id, StoryProfileScope::Archive, cx)
    }

    pub fn highlight_picker_paging_scope(&self, owner_id: &str, cx: &mut App) -> Option<StoryProfileScope> {
        let mut newest: Option<(StoryProfileScope, i64)> = None;
        for scope in [StoryProfileScope::Profile, StoryProfileScope::Archive] {
            if !self.profile_stories_has_more(owner_id, scope, cx) {
                continue;
            }
            let Some(cursor) = self.profile_stories_next_cursor(owner_id, scope, cx) else {
                continue;
            };
            if newest.is_none_or(|(_, best)| best < cursor) {
                newest = Some((scope, cursor));
            }
        }
        newest.map(|(scope, _)| scope)
    }

    pub fn pinned_profile_story_ids(&self, target_user_id: &str, cx: &mut App) -> Vec<i64> {
        self.profile_stories(target_user_id, StoryProfileScope::Profile, cx)
            .iter()
            .filter(|story| story.is_pinned_in_profile())
            .map(|story| story.id)
            .collect()
    }

    pub fn apply_updated_story(&mut self, story: &StoryDTO, user_id: &str, cx: &mut Context<Self>) {
        let transforms = self.stories_upsert_transforms(std::slice::from_ref(story), user_id, cx);
        let token = self.begin_story_mutation(transforms, cx);
        self.commit_story_mutation(&token, HashMap::new(), cx);
        self.refresh_open_player_stories(user_id, cx);
    }

    pub fn stories_upsert_transforms(
        &self,
        stories: &[StoryDTO],
        user_id: &str,
        _cx: &mut App,
    ) -> HashMap<String, StoryCacheTransform> {
        if stories.is_empty() {
            return HashMap::new();
        }

        let profile_key = Self::profile_stories_cache_key(user_id, StoryProfileScope::Profile);
        let archive_key = Self::profile_stories_cache_key(user_id, StoryProfileScope::Archive);
        let user_stories_key = Self::user_stories_cache_key(user_id);
        let stories: Rc<Vec<StoryDTO>> = Rc::new(stories.to_vec());

        let mut transforms: HashMap<String, StoryCacheTransform> = HashMap::new();
        let profile_stories = Rc::clone(&stories);
        transforms.insert(
            profile_key,
            Rc::new(move |existing: Option<Map<String, Value>>| {
                let mut root = existing;
                let now_ms = Self::now_ms();
                for story in profile_stories.iter() {
                    root = if story.sits_in_profile_archive(now_ms) {
                        (Self::remove_story_transform(story.id))(root)
                    } else {
                        (Self::insert_profile_story_transform(story.clone()))(root)
                    };
                }
                (Self::reorder_profile_stories_transform())(root)
            }),
        );
        let archive_stories = Rc::clone(&stories);
        transforms.insert(
            archive_key,
            Rc::new(move |existing: Option<Map<String, Value>>| {
                let mut root = existing;
                let now_ms = Self::now_ms();
                for story in archive_stories.iter() {
                    root = if story.sits_in_profile_archive(now_ms) {
                        (Self::insert_profile_story_transform(story.clone()))(root)
                    } else {
                        (Self::remove_story_transform(story.id))(root)
                    };
                }
                root
            }),
        );
        let tray_stories = Rc::clone(&stories);
        transforms.insert(
            user_stories_key,
            Rc::new(move |existing: Option<Map<String, Value>>| {
                existing.as_ref()?;
                let mut root = existing;
                for story in tray_stories.iter() {
                    root = if story.is_archived() {
                        (Self::remove_story_transform(story.id))(root)
                    } else {
                        (Self::patch_story_row_transform(story.clone()))(root)
                    };
                }
                root
            }),
        );
        let feed_stories = stories;
        transforms.insert(
            Self::feed_cache_key(),
            Rc::new(move |existing: Option<Map<String, Value>>| {
                existing.as_ref()?;
                let mut root = existing;
                for story in feed_stories.iter().filter(|story| story.is_archived()) {
                    root = (Self::remove_feed_story_transform(story.id))(root);
                }
                root
            }),
        );
        transforms
    }

    pub fn archive_stories_mutation_transforms(
        &self,
        story_ids: &[i64],
        archived: bool,
        user_id: &str,
        cx: &mut App,
    ) -> HashMap<String, StoryCacheTransform> {
        let ids: Rc<HashSet<i64>> = Rc::new(story_ids.iter().copied().collect());
        let source_scope = if archived { StoryProfileScope::Profile } else { StoryProfileScope::Archive };
        let destination_scope = if archived { StoryProfileScope::Archive } else { StoryProfileScope::Profile };
        let moved: Vec<StoryDTO> = self
            .profile_stories(user_id, source_scope, cx)
            .into_iter()
            .filter(|story| ids.contains(&story.id))
            .map(|story| {
                let mut next = story;
                next.archived_at_ms = if archived { Some(Self::now_ms()) } else { None };
                if archived {
                    next.pinned_at_ms = None;
                }
                next
            })
            .collect();

        let source_key = Self::profile_stories_cache_key(user_id, source_scope);
        let destination_key = Self::profile_stories_cache_key(user_id, destination_scope);

        let mut transforms: HashMap<String, StoryCacheTransform> = HashMap::new();
        let source_ids = Rc::clone(&ids);
        transforms.insert(
            source_key,
            Rc::new(move |existing: Option<Map<String, Value>>| {
                let mut root = existing;
                for id in source_ids.iter() {
                    root = (Self::remove_story_transform(*id))(root);
                }
                root
            }),
        );
        transforms.insert(
            destination_key,
            Rc::new(move |existing: Option<Map<String, Value>>| {
                let mut root = existing;
                for story in &moved {
                    root = (Self::insert_profile_story_transform(story.clone()))(root);
                }
                if destination_scope == StoryProfileScope::Profile {
                    (Self::reorder_profile_stories_transform())(root)
                } else {
                    root
                }
            }),
        );
        if archived {
            let tray_ids = Rc::clone(&ids);
            transforms.insert(
                Self::user_stories_cache_key(user_id),
                Rc::new(move |existing: Option<Map<String, Value>>| {
                    existing.as_ref()?;
                    let mut root = existing;
                    for id in tray_ids.iter() {
                        root = (Self::remove_story_transform(*id))(root);
                    }
                    root
                }),
            );
            let feed_ids = ids;
            transforms.insert(
                Self::feed_cache_key(),
                Rc::new(move |existing: Option<Map<String, Value>>| {
                    existing.as_ref()?;
                    let mut root = existing;
                    for id in feed_ids.iter() {
                        root = (Self::remove_feed_story_transform(*id))(root);
                    }
                    root
                }),
            );
        }
        transforms
    }

    pub fn pin_stories_mutation_transforms(
        &self,
        story_ids: &[i64],
        pinned: bool,
        user_id: &str,
        _cx: &mut App,
    ) -> HashMap<String, StoryCacheTransform> {
        let ids: HashSet<i64> = story_ids.iter().copied().collect();
        let stamp = Self::now_ms();
        let key = Self::profile_stories_cache_key(user_id, StoryProfileScope::Profile);
        let transform: StoryCacheTransform = Rc::new(move |existing: Option<Map<String, Value>>| {
            let mut root = existing?;
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST));
            for row in rows.iter_mut() {
                if !ids.contains(&Self::story_id(row.get(key::ID))) {
                    continue;
                }
                row.insert(key::PINNED_AT_MS.to_owned(), if pinned { Value::from(stamp) } else { Value::Null });
                if pinned {
                    row.insert(key::POST_TO_PROFILE.to_owned(), Value::Bool(true));
                }
            }
            root.insert(key::LIST.to_owned(), Value::Array(rows.into_iter().map(Value::Object).collect()));
            (Self::reorder_profile_stories_transform())(Some(root))
        });
        HashMap::from([(key, transform)])
    }

    pub fn insert_profile_story_transform(story: StoryDTO) -> StoryCacheTransform {
        Rc::new(move |existing: Option<Map<String, Value>>| {
            let Some(encoded) = SaiJSONCodec::encode(&story) else {
                return existing;
            };
            let mut root = existing.unwrap_or_else(|| {
                let mut empty = Map::new();
                empty.insert(key::SUCCESS.to_owned(), Value::Bool(true));
                empty.insert(key::LIST.to_owned(), Value::Array(Vec::new()));
                empty.insert(key::HAS_MORE.to_owned(), Value::Bool(false));
                empty.insert(key::TOTAL.to_owned(), Value::from(0));
                empty
            });
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST));
            if let Some(index) = rows.iter().position(|row| Self::story_id(row.get(key::ID)) == story.id) {
                rows[index] = encoded;
            } else {
                let insertion = rows
                    .iter()
                    .position(|row| Self::story_id(row.get(key::ID)) < story.id)
                    .unwrap_or(rows.len());
                rows.insert(insertion, encoded);
                let fallback = i64::try_from(rows.len()).unwrap_or(i64::MAX) - 1;
                let total = root.get(key::TOTAL).and_then(Value::as_i64).unwrap_or(fallback) + 1;
                root.insert(key::TOTAL.to_owned(), Value::from(total));
            }
            root.insert(key::LIST.to_owned(), Value::Array(rows.into_iter().map(Value::Object).collect()));
            Some(root)
        })
    }

    pub fn patch_story_row_transform(story: StoryDTO) -> StoryCacheTransform {
        Rc::new(move |existing: Option<Map<String, Value>>| {
            let mut root = existing?;
            let Some(encoded) = SaiJSONCodec::encode(&story) else {
                return Some(root);
            };
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST));
            let Some(index) = rows.iter().position(|row| Self::story_id(row.get(key::ID)) == story.id) else {
                return Some(root);
            };
            rows[index] = encoded;
            root.insert(key::LIST.to_owned(), Value::Array(rows.into_iter().map(Value::Object).collect()));
            Some(root)
        })
    }

    pub fn reorder_profile_stories_transform() -> StoryCacheTransform {
        Rc::new(|existing: Option<Map<String, Value>>| {
            let mut root = existing?;
            let rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST));
            let (mut pinned, mut rest): (Vec<Map<String, Value>>, Vec<Map<String, Value>>) =
                rows.into_iter().partition(|row| Self::pinned_at_ms(row).is_some());
            pinned.sort_by(|lhs, rhs| {
                Self::pinned_at_ms(rhs).unwrap_or(0).cmp(&Self::pinned_at_ms(lhs).unwrap_or(0))
            });
            rest.sort_by(|lhs, rhs| Self::story_id(rhs.get(key::ID)).cmp(&Self::story_id(lhs.get(key::ID))));
            pinned.extend(rest);
            root.insert(key::LIST.to_owned(), Value::Array(pinned.into_iter().map(Value::Object).collect()));
            Some(root)
        })
    }

    fn pinned_at_ms(row: &Map<String, Value>) -> Option<i64> {
        let value = row.get(key::PINNED_AT_MS)?;
        let millis = value.as_i64().or_else(|| value.as_f64().map(|number| number as i64))?;
        (millis > 0).then_some(millis)
    }

    fn now_ms() -> i64 {
        (Date::now().time_interval_since_1970() * 1_000.0) as i64
    }

    pub fn reedit_retention_seconds() -> TimeInterval {
        StoryCanvasGeometry::edit_window_ms() as TimeInterval / 1_000.0 + 600.0
    }

    pub fn reedit_identity(story_id: i64) -> String {
        format!("story-{story_id}")
    }

    pub fn persist_story_reedit_document(&self, item: &MediaPickerItem, operation_signature: &str, cx: &mut App) {
        let Some(image) = item.image_payload.as_ref() else {
            return;
        };
        if !image.was_edited {
            return;
        }
        let (Some(state_data), Some(original_url)) = (image.editor_state_data.as_ref(), image.original_image_url.as_ref())
        else {
            return;
        };
        let Some(original_path) = original_url.to_file_path() else {
            return;
        };
        MediaReeditStore::shared(cx).update(cx, |store, _| {
            let Some(edit_id) = store.save_document(&original_path, state_data, Self::reedit_retention_seconds()) else {
                return;
            };
            store.register_pending_temp(operation_signature, &edit_id);
        });
    }

    pub fn bind_story_reedit_document(&self, operation_signature: &str, story_id: i64, cx: &mut App) {
        MediaReeditStore::shared(cx).update(cx, |store, _| {
            let Some(edit_id) = store.consume_pending_temp(operation_signature) else {
                return;
            };
            store.link(&Self::reedit_identity(story_id), &edit_id);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open_stories(
        &mut self,
        stories: Vec<StoryDTO>,
        author_id: &str,
        author: Option<StoryAuthorDTO>,
        starting_at_story_id: i64,
        request_id: Uuid,
        entrance: StoryPlayerEntrance,
        shared_element_origin: Option<StoryPlayerSharedElementOrigin>,
        cx: &mut Context<Self>,
    ) {
        let playable: Vec<StoryDTO> = stories.into_iter().filter(|story| story.id != 0).collect();
        if playable.is_empty() {
            return;
        }

        let unseen = playable.iter().filter(|story| !self.is_story_viewed(story, cx)).count();
        let group = StoryGroupDTO {
            all_seen: unseen == 0,
            author: author.or_else(|| playable.first().and_then(|story| story.author.clone())),
            stories: playable,
            unseen_count: i64::try_from(unseen).unwrap_or(i64::MAX),
        };
        self.note_user_stories_target(author_id, cx);
        self.present_player(
            StoryPlayerRequest::new(vec![group], 0)
                .with_id(request_id)
                .with_start_story_id(Some(starting_at_story_id))
                .with_owns_explicit_stories(true)
                .with_entrance(entrance)
                .with_shared_element_origin(shared_element_origin),
            None,
            cx,
        );
    }
}
