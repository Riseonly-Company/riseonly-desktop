use std::any::Any;
use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity};
use rise_canon::rise_key::post as key;
use serde_json::Value;

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsDiskCachePolicy;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, GetPostFeedResponse};
use crate::modules::post::stores::post_services::post_services::PostServicesStoreCachedFeedPage;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services_hooks::ShortsServicesHooks;
use crate::modules::user::stores::user::user_actions::user_types::UserListRelationship;

pub struct ShortsServicesStore {
    external_shorts: HashMap<String, GetPostFeedResponse>,
    last_error_text: Option<String>,
    did_load: bool,
    hidden_short_ids: HashSet<String>,
    pub feed_audio_muted: bool,
    full_screen_playback_owners: HashSet<String>,
    pending_shorts: Vec<GetPostFeedResponse>,
    feed_top_request: u64,
    feed_chrome_hidden: bool,
}

impl ShortsServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            external_shorts: HashMap::new(),
            last_error_text: None,
            did_load: false,
            hidden_short_ids: HashSet::new(),
            feed_audio_muted: true,
            full_screen_playback_owners: HashSet::new(),
            pending_shorts: Vec::new(),
            feed_top_request: 0,
            feed_chrome_hidden: false,
        }
    }

    pub fn feed_cache_key() -> String {
        SaiCache::new(["getShortsFeed", "withImageReels"]).key
    }

    pub fn external_shorts(&self) -> &HashMap<String, GetPostFeedResponse> {
        &self.external_shorts
    }

    pub fn last_error_text(&self) -> Option<&str> {
        self.last_error_text.as_deref()
    }

    pub fn did_load(&self) -> bool {
        self.did_load
    }

    pub fn hidden_short_ids(&self) -> &HashSet<String> {
        &self.hidden_short_ids
    }

    pub fn hide_short(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if post_id.is_empty() {
            return;
        }
        self.hidden_short_ids.insert(post_id.to_owned());
        cx.notify();
    }

    pub fn unhide_short(&mut self, post_id: &str, cx: &mut Context<Self>) {
        self.hidden_short_ids.remove(post_id);
        cx.notify();
    }

    pub fn inline_feed_playback_enabled(&self) -> bool {
        self.full_screen_playback_owners.is_empty()
    }

    pub fn begin_full_screen_playback(&mut self, owner: String, cx: &mut Context<Self>) {
        self.full_screen_playback_owners.insert(owner);
        cx.notify();
    }

    pub fn end_full_screen_playback(&mut self, owner: String, cx: &mut Context<Self>) {
        self.full_screen_playback_owners.remove(&owner);
        cx.notify();
    }

    fn cached_page(&self, cx: &mut App) -> Option<PostServicesStoreCachedFeedPage> {
        ShortsServicesHooks::cached_feed_page(&Self::feed_cache_key(), SaiWsDiskCachePolicy::MemoryOnly, cx)
    }

    pub fn feed_top_request(&self) -> u64 {
        self.feed_top_request
    }

    pub fn request_feed_top(&mut self, cx: &mut Context<Self>) {
        self.feed_top_request = self.feed_top_request.wrapping_add(1);
        cx.notify();
    }

    pub fn feed_chrome_hidden(&self) -> bool {
        self.feed_chrome_hidden
    }

    pub fn update_feed_chrome_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        if self.feed_chrome_hidden == hidden {
            return;
        }
        self.feed_chrome_hidden = hidden;
        cx.notify();
    }

    pub fn pending_shorts(&self) -> &[GetPostFeedResponse] {
        &self.pending_shorts
    }

    pub fn items(&self, cx: &mut App) -> Vec<GetPostFeedResponse> {
        let cached = self.cached_items(cx);
        if self.pending_shorts.is_empty() {
            return cached;
        }
        let mut items = self.pending_shorts.clone();
        items.extend(cached);
        items
    }

    fn cached_items(&self, cx: &mut App) -> Vec<GetPostFeedResponse> {
        self.cached_page(cx).map(|page| page.list).unwrap_or_default()
    }

    pub fn upsert_pending_short(&mut self, post: GetPostFeedResponse, cx: &mut Context<Self>) {
        if !post.is_temp || post.id.is_empty() {
            return;
        }

        if let Some(idx) = self.pending_shorts.iter().position(|pending| pending.id == post.id) {
            self.pending_shorts[idx] = post;
        } else {
            self.pending_shorts.insert(0, post);
        }
        cx.notify();
    }

    pub fn remove_pending_short(&mut self, temp_id: &str, cx: &mut Context<Self>) {
        self.pending_shorts.retain(|pending| pending.id != temp_id);
        cx.notify();
    }

    pub fn settle_pending_short(&mut self, temp_id: &str, post: GetPostFeedResponse, cx: &mut Context<Self>) {
        if !post.id.is_empty()
            && !post.is_temp
            && self.cached_page(cx).is_some()
            && let Some(row) = SaiJSONCodec::encode(&post)
        {
            let feed_cache_key = Self::feed_cache_key();
            let _ = SaiUpdater::remove_row(
                &feed_cache_key,
                key::LIST,
                key::ID,
                &post.id,
                SaiUpdaterWriteScope::Cache,
                cx,
            );
            let _ = SaiUpdater::insert_row_if_missing(
                &feed_cache_key,
                key::LIST,
                key::ID,
                row,
                true,
                SaiUpdaterWriteScope::Cache,
                cx,
            );
        }
        self.remove_pending_short(temp_id, cx);
    }

    pub fn has_more(&self, cx: &mut App) -> bool {
        self.cached_page(cx).and_then(|page| page.is_have_more).unwrap_or(true)
    }

    pub fn relative_id(&self, cx: &mut App) -> Option<String> {
        self.cached_page(cx).and_then(|page| page.relative_id)
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        SaiWsResponseCache::remove(&Self::feed_cache_key(), SaiCacheCleanLayer::Everywhere, cx);
        self.external_shorts.clear();
        self.pending_shorts.clear();
        self.hidden_short_ids.clear();
        self.full_screen_playback_owners.clear();
        self.last_error_text = None;
        self.did_load = false;
        cx.notify();
    }

    pub fn set_last_error_text(&mut self, text: Option<String>, cx: &mut Context<Self>) {
        self.last_error_text = text;
        cx.notify();
    }

    pub fn feed_page_success_handler(&mut self, payload: Option<Box<dyn Any>>, cx: &mut Context<Self>) {
        self.last_error_text = None;
        self.did_load = true;
        ShortsServicesHooks::cache_own_posts(payload.as_deref(), cx);
        cx.notify();
    }

    fn update_cached_items(&self, transform: impl FnOnce(&mut Vec<GetPostFeedResponse>), cx: &mut App) {
        let mut updated = self.cached_items(cx);
        transform(&mut updated);
        let rows: Vec<Value> = updated.iter().filter_map(|post| SaiJSONCodec::encode(post)).map(Value::Object).collect();

        let _ = SaiUpdater::update_json_value(
            &Self::feed_cache_key(),
            |context| {
                let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                root.insert(key::LIST.to_owned(), Value::Array(rows));
                Ok(Value::Object(root))
            },
            SaiUpdaterWriteScope::Cache,
            cx,
        );
    }

    pub fn resolved_short(&self, post_id: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
        self.items(cx)
            .into_iter()
            .find(|item| item.id == post_id)
            .or_else(|| self.external_shorts.get(post_id).cloned())
    }

    pub fn transform_short(
        &mut self,
        post_id: &str,
        transform: impl Fn(GetPostFeedResponse) -> GetPostFeedResponse,
        cx: &mut Context<Self>,
    ) {
        self.update_cached_items(
            |items| {
                if let Some(idx) = items.iter().position(|item| item.id == post_id) {
                    items[idx] = transform(items[idx].clone());
                }
            },
            cx,
        );
        if let Some(existing) = self.external_shorts.get(post_id).cloned() {
            self.external_shorts.insert(post_id.to_owned(), transform(existing));
        }
        cx.notify();
    }

    pub fn register_external_short_if_needed(&mut self, post: GetPostFeedResponse, cx: &mut Context<Self>) {
        if post.id.is_empty() {
            return;
        }
        if self.items(cx).iter().any(|item| item.id == post.id) {
            return;
        }

        if !self.external_shorts.contains_key(&post.id) {
            self.external_shorts.insert(post.id.clone(), post);
            cx.notify();
        }
    }

    fn author_matches(post: &GetPostFeedResponse, author_id: &str) -> bool {
        if post.author.id.as_deref() == Some(author_id) {
            return true;
        }
        post.author_id == author_id
    }

    pub fn transform_author(
        &mut self,
        author_id: &str,
        transform: impl Fn(AuthorInfo) -> AuthorInfo,
        cx: &mut Context<Self>,
    ) {
        if author_id.is_empty() {
            return;
        }

        if self.items(cx).iter().any(|item| Self::author_matches(item, author_id)) {
            self.update_cached_items(
                |items| {
                    for item in items.iter_mut().filter(|item| Self::author_matches(item, author_id)) {
                        item.author = transform(item.author.clone());
                    }
                },
                cx,
            );
        }
        for value in self.external_shorts.values_mut().filter(|value| Self::author_matches(value, author_id)) {
            value.author = transform(value.author.clone());
        }
        cx.notify();
    }

    pub fn set_author_relationship(
        &mut self,
        author_id: &str,
        relationship: UserListRelationship,
        cx: &mut Context<Self>,
    ) {
        if author_id.is_empty() {
            return;
        }

        let is_stale = |post: &GetPostFeedResponse| {
            Self::author_matches(post, author_id) && Self::author_relationship_from_author(&post.author) != relationship
        };
        let any_stale = self.items(cx).iter().any(|post| is_stale(post))
            || self.external_shorts.values().any(|post| is_stale(post));
        if !any_stale {
            return;
        }

        self.transform_author(
            author_id,
            |author| AuthorInfo {
                is_subscribed: relationship.is_subbed,
                is_subscriber: relationship.is_subscriber,
                is_friend: relationship.is_friend,
                ..author
            },
            cx,
        );
    }

    pub fn author_relationship(&self, post: &GetPostFeedResponse) -> UserListRelationship {
        Self::author_relationship_from_author(&post.author)
    }

    fn author_relationship_from_author(author: &AuthorInfo) -> UserListRelationship {
        UserListRelationship {
            is_subbed: author.is_subscribed,
            is_subscriber: author.is_subscriber,
            is_friend: author.is_friend,
        }
    }

    pub fn remove_short_snapshot(
        &mut self,
        post_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<(GetPostFeedResponse, usize)> {
        let cached = self.cached_items(cx);
        let idx = cached.iter().position(|item| item.id == post_id)?;

        let snapshot = cached[idx].clone();
        self.update_cached_items(|items| items.retain(|item| item.id != post_id), cx);
        cx.notify();
        Some((snapshot, idx))
    }

    pub fn remove_short_everywhere_snapshot(
        &mut self,
        post_id: &str,
        cx: &mut Context<Self>,
    ) -> (Option<GetPostFeedResponse>, Option<usize>, Option<GetPostFeedResponse>) {
        let feed = self.remove_short_snapshot(post_id, cx);
        let external = self.external_shorts.remove(post_id);
        cx.notify();
        match feed {
            Some((snapshot, index)) => (Some(snapshot), Some(index), external),
            None => (None, None, external),
        }
    }

    pub fn remove_short_everywhere(&mut self, post_id: &str, cx: &mut Context<Self>) {
        let _ = self.remove_short_snapshot(post_id, cx);
        self.external_shorts.remove(post_id);
        cx.notify();
    }

    pub fn restore_short_snapshot(&mut self, snapshot: GetPostFeedResponse, index: usize, cx: &mut Context<Self>) {
        if self.cached_items(cx).iter().any(|item| item.id == snapshot.id) {
            return;
        }

        self.update_cached_items(
            |items| {
                if items.iter().any(|item| item.id == snapshot.id) {
                    return;
                }
                let insert_at = index.min(items.len());
                items.insert(insert_at, snapshot);
            },
            cx,
        );
        cx.notify();
    }

    pub fn restore_short_everywhere_snapshot(
        &mut self,
        feed_post: Option<GetPostFeedResponse>,
        feed_index: Option<usize>,
        external_post: Option<GetPostFeedResponse>,
        cx: &mut Context<Self>,
    ) {
        if let (Some(feed_post), Some(feed_index)) = (feed_post, feed_index) {
            self.restore_short_snapshot(feed_post, feed_index, cx);
        }
        if let Some(external_post) = external_post
            && !self.external_shorts.contains_key(&external_post.id)
        {
            self.external_shorts.insert(external_post.id.clone(), external_post);
            cx.notify();
        }
    }
}
