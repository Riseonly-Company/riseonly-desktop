use std::collections::HashSet;

use gpui::{App, Context};
use rise_canon::rise_key::post as post_keys;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_limits::service_limit::POST_LIKES_SEARCH_MAX_CHARS;
use serde_json::{Map, Value};

use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::post::stores::post_actions::post_likes_types::PostLikeUser;
use crate::modules::post::stores::post_actions::post_likes_wire_types::{GetPostLikesGatewayDTO, PostLikeUserDTOExt};
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::UserListRelationship;

const POST_LIKES_SEARCH_SEGMENT: &str = "-search-";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PostLikesTarget {
    Post(String),
    Comment(String),
}

impl PostLikesTarget {
    pub fn id(&self) -> &str {
        match self {
            Self::Post(id) | Self::Comment(id) => id,
        }
    }

    fn cache_tag(&self) -> &'static str {
        match self {
            Self::Post(_) => "getPostLikes",
            Self::Comment(_) => "getCommentLikes",
        }
    }
}

impl PostServicesStore {
    pub fn normalized_post_likes_search(search: Option<&str>) -> Option<String> {
        let search = search?;

        let trimmed = trimming_whitespaces_and_newlines(search);
        if trimmed.is_empty() {
            return None;
        }
        let limit = usize::try_from(POST_LIKES_SEARCH_MAX_CHARS).unwrap_or(usize::MAX);
        Some(trimmed.chars().take(limit).collect())
    }

    pub fn post_likes_cache_key(target: &PostLikesTarget, search: Option<&str>) -> String {
        if let Some(search) = Self::normalized_post_likes_search(search) {
            let lowered = search.to_lowercase();
            return SaiCache::new([target.cache_tag(), target.id(), "search", lowered.as_str()]).key;
        }
        SaiCache::new([target.cache_tag(), target.id()]).key
    }

    pub fn present_post_likes(&mut self, target: &PostLikesTarget, cx: &mut Context<Self>) {
        if target.id().is_empty() {
            return;
        }

        self.post_likes_target = Some(target.clone());
        self.post_likes_sheet_visible = true;
        cx.notify();
    }

    pub fn dismiss_post_likes(&mut self, cx: &mut Context<Self>) {
        self.post_likes_sheet_visible = false;
        self.post_likes_target = None;
        self.post_likes_active_cache_key = None;
        cx.notify();
    }

    pub fn prepare_post_likes_request(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        if let Some(previous) = self.post_likes_active_cache_key.clone()
            && previous != cache_key
            && previous.contains(POST_LIKES_SEARCH_SEGMENT)
        {
            SaiWsResponseCache::remove(&previous, SaiCacheCleanLayer::Everywhere, cx);
            self.failed_post_likes_cache_keys.remove(&previous);
        }
        self.post_likes_active_cache_key = Some(cache_key.to_owned());
        self.clear_post_likes_failure(cache_key, cx);
    }

    pub fn post_likes(&self, target: &PostLikesTarget, search: Option<&str>, cx: &mut App) -> Vec<PostLikeUser> {
        self.post_likes_dto(target, search, cx)
            .map(|dto| dto.items.iter().map(PostLikeUserDTOExt::as_domain).collect())
            .unwrap_or_default()
    }

    pub fn post_likes_has_more(&self, target: &PostLikesTarget, search: Option<&str>, cx: &mut App) -> bool {
        self.post_likes_dto(target, search, cx).is_some_and(|dto| dto.has_more)
    }

    pub fn post_likes_relative_id(&self, target: &PostLikesTarget, search: Option<&str>, cx: &mut App) -> Option<i64> {
        self.post_likes_dto(target, search, cx).and_then(|dto| dto.relative_id)
    }

    pub fn post_likes_status(&self, target: &PostLikesTarget, search: Option<&str>, cx: &mut App) -> SaiWsSliceStatus {
        let cache_key = Self::post_likes_cache_key(target, search);
        if SaiWsResponseCache::observed_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
        .is_some()
        {
            return SaiWsSliceStatus::Fulfilled;
        }
        if SaiWsPendingRegistry::shared(cx).read(cx).in_flight_request_id(&cache_key).is_some() {
            return SaiWsSliceStatus::Pending;
        }
        if self.failed_post_likes_cache_keys.contains(&cache_key) {
            return SaiWsSliceStatus::Rejected;
        }
        SaiWsSliceStatus::Idle
    }

    pub fn set_like_user_relationship(
        &mut self,
        user_id: &str,
        relationship: UserListRelationship,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.post_likes_target.clone() else {
            return;
        };
        if user_id.is_empty() {
            return;
        }

        let cache_keys: HashSet<String> =
            [Some(Self::post_likes_cache_key(&target, None)), self.post_likes_active_cache_key.clone()]
                .into_iter()
                .flatten()
                .collect();
        for cache_key in cache_keys {
            let Some(payload) = SaiWsResponseCache::current_payload(
                &cache_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            ) else {
                continue;
            };
            let Some(dto) = SaiJSONCodec::decode::<GetPostLikesGatewayDTO>(Some(&payload)) else {
                continue;
            };
            let Some(row) = dto.items.iter().find(|row| row.id == user_id) else {
                continue;
            };
            if row.as_domain().relationship() == relationship {
                continue;
            }

            let mut patch = Map::new();
            patch.insert(post_keys::IS_FRIEND.to_owned(), Value::Bool(relationship.is_friend));
            patch.insert(post_keys::IS_SUBBED.to_owned(), Value::Bool(relationship.is_subbed));
            patch.insert(post_keys::IS_SUBSCRIBER.to_owned(), Value::Bool(relationship.is_subscriber));
            let _ = SaiUpdater::patch_row(
                &cache_key,
                post_keys::ITEMS,
                post_keys::ID,
                user_id,
                &patch,
                SaiUpdaterWriteScope::All,
                cx,
            );
        }
    }

    pub fn mark_post_likes_failure(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        if self.failed_post_likes_cache_keys.insert(cache_key.to_owned()) {
            cx.notify();
        }
    }

    pub fn clear_post_likes_failure(&mut self, cache_key: &str, cx: &mut Context<Self>) {
        if self.failed_post_likes_cache_keys.remove(cache_key) {
            cx.notify();
        }
    }

    fn post_likes_dto(&self, target: &PostLikesTarget, search: Option<&str>, cx: &mut App) -> Option<GetPostLikesGatewayDTO> {
        let cache_key = Self::post_likes_cache_key(target, search);
        let payload = SaiWsResponseCache::observed_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )?;

        SaiJSONCodec::decode::<GetPostLikesGatewayDTO>(Some(&payload))
    }
}
