use std::collections::HashMap;
use std::rc::Rc;

use gpui::App;
use rise_canon::rise_key::stories as key;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::modules::story::stores::story::story_actions::story_profile_wire_types::GetCloseFriendsGatewayDTO;
use crate::modules::story::stores::story::story_actions::story_wire_types::StoryAuthorDTO;
use crate::modules::story::stores::story::story_services::story_services::{StoryCacheTransform, StoryServicesStore};

impl StoryServicesStore {
    pub fn close_friends_cache_key() -> String {
        SaiCache::new(["getCloseFriends"]).key
    }

    pub fn close_friends(&self, cx: &mut App) -> Vec<StoryAuthorDTO> {
        self.decode_gateway::<GetCloseFriendsGatewayDTO>(&Self::close_friends_cache_key(), cx)
            .map(|gateway| gateway.friends)
            .unwrap_or_default()
    }

    pub fn close_friends_loaded(&self, cx: &mut App) -> bool {
        self.decode_gateway::<GetCloseFriendsGatewayDTO>(&Self::close_friends_cache_key(), cx)
            .is_some()
    }

    pub fn is_close_friend(&self, user_id: Option<&str>, cx: &mut App) -> bool {
        let Some(user_id) = user_id.map(trimming_whitespaces_and_newlines).filter(|id| !id.is_empty()) else {
            return false;
        };
        self.close_friends(cx).iter().any(|friend| friend.id == user_id)
    }

    pub fn close_friends_replace_transforms(&self, friends: &[StoryAuthorDTO]) -> HashMap<String, StoryCacheTransform> {
        let rows: Vec<Value> = friends
            .iter()
            .filter_map(SaiJSONCodec::encode)
            .map(Value::Object)
            .collect();
        let transform: StoryCacheTransform = Rc::new(move |_: Option<Map<String, Value>>| {
            let mut root = Map::new();
            root.insert(key::SUCCESS.to_owned(), Value::Bool(true));
            root.insert(key::TOTAL.to_owned(), Value::from(rows.len()));
            root.insert(key::FRIENDS.to_owned(), Value::Array(rows.clone()));
            Some(root)
        });
        HashMap::from([(Self::close_friends_cache_key(), transform)])
    }

    pub fn close_friend_toggle_transforms(
        &self,
        author: &StoryAuthorDTO,
        is_close: bool,
    ) -> HashMap<String, StoryCacheTransform> {
        let target_id = trimming_whitespaces_and_newlines(&author.id).to_owned();
        if target_id.is_empty() {
            return HashMap::new();
        }
        let encoded = if is_close { SaiJSONCodec::encode(author) } else { None };
        let transform: StoryCacheTransform = Rc::new(move |existing: Option<Map<String, Value>>| {
            let mut root = existing.unwrap_or_else(|| {
                let mut empty = Map::new();
                empty.insert(key::SUCCESS.to_owned(), Value::Bool(true));
                empty.insert(key::FRIENDS.to_owned(), Value::Array(Vec::new()));
                empty.insert(key::TOTAL.to_owned(), Value::from(0));
                empty
            });
            let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::FRIENDS));
            rows.retain(|row| row.get(key::ID).and_then(Value::as_str) != Some(target_id.as_str()));
            if let Some(encoded) = encoded.clone() {
                rows.insert(0, encoded);
            }
            let total = rows.len();
            root.insert(key::FRIENDS.to_owned(), Value::Array(rows.into_iter().map(Value::Object).collect()));
            root.insert(key::TOTAL.to_owned(), Value::from(total));
            Some(root)
        });
        HashMap::from([(Self::close_friends_cache_key(), transform)])
    }
}
