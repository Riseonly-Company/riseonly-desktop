use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_canon::rise_key::post as key;
use rise_canon::{RiseEvent, RiseWireKey};
use rise_catalog::PostModerationStatus;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_events::name;
use rise_i18n::date_format::iso8601_date_formatter::ISO8601DateFormatter;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_cache_cleaner::SaiCacheCleanLayer;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::comment::stores::comment_actions::comment_types::CommentEntities;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_domain::RisePostDomainRegistry;
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostContentKind, RisePostFeedKind, RisePostPagePresentation, RisePostPageScope, RisePostPageStatus,
    RisePostProfileKind, RisePostSearchKind,
};
use crate::modules::post::engine::rise_post_presentation::RisePostPresentation;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostDeleteCachePlacement, PostDeleteOptimisticSnapshot, PostHashtagParsing,
    PostTrackPlateDTO, RiseStoredPostStats,
};
use crate::modules::post::stores::post_actions::post_wire_types::{
    CreatePostGatewayDTO, GetFeedGatewayDTO, GetFeedPostItemDTO, GetFeedPostItemDTOExt, GetViewHistoryGatewayDTO,
    RisePostWirePage,
};
use crate::modules::post::stores::post_actions::rise_post_publish_failure::RisePostPublishFailure;
use crate::modules::post::stores::post_services::post_guide_sandbox_hooks::PostGuideSandboxHooks;
use crate::modules::post::stores::post_services::post_likes_services::PostLikesTarget;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::UserListRelationship;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

static CLIENT_ONLY_POST_STATE_KEYS: LazyLock<HashSet<String>> = LazyLock::new(|| {
    [
        RiseLocalKey::IS_TEMP,
        RiseLocalKey::IS_UPLOADING,
        RiseLocalKey::IS_COMPRESSING,
        RiseLocalKey::UPLOAD_PROGRESS,
        RiseLocalKey::COMPRESSION_PROGRESS,
        RiseLocalKey::LOCAL_PREVIEW_URL,
        RiseLocalKey::OLD_IS_LIKED,
        RiseLocalKey::OLD_IS_FAVORITED,
    ]
    .into_iter()
    .flat_map(|snake| [snake.to_owned(), RiseWireKey::camel(snake)])
    .collect()
});

const NEW_SORT: &str = "new";

pub type PostTransform<'a> = &'a mut dyn FnMut(GetPostFeedResponse) -> GetPostFeedResponse;

#[derive(Clone, Debug, PartialEq)]
pub struct PostServicesStoreUserPostsState {
    pub items: Vec<GetPostFeedResponse>,
    pub has_more: bool,
    pub relative_id: Option<String>,
    pub feed_session_id: Option<String>,
    pub last_error_text: Option<String>,
    pub did_load: bool,
}

impl Default for PostServicesStoreUserPostsState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            has_more: true,
            relative_id: None,
            feed_session_id: None,
            last_error_text: None,
            did_load: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PostServicesStoreCachedFeedPage {
    pub list: Vec<GetPostFeedResponse>,
    pub is_have_more: Option<bool>,
    pub relative_id: Option<String>,
    pub feed_session_id: Option<String>,
    pub next_cursor: Option<i64>,
    pub next_cursor_post_id: Option<i64>,
    pub total: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostServicesStorePostViewHistoryState {
    pub items: Vec<GetPostFeedResponse>,
    pub has_more: bool,
    pub next_cursor: Option<i64>,
    pub next_cursor_post_id: Option<i64>,
    pub last_error_text: Option<String>,
    pub did_load: bool,
}

#[derive(Default)]
pub struct PostServicesStore {
    wire_loaded_feed_account_id: Option<String>,
    feed_last_error_text: Option<String>,
    pub selected_post: Option<GetPostFeedResponse>,
    comments_post_taken_at_milliseconds: i64,
    last_own_deleted_post_id: Option<String>,
    own_deleted_post_ids: HashSet<String>,
    live_deleted_post_ids: HashSet<String>,
    pub post_likes_sheet_visible: bool,
    pub post_likes_target: Option<PostLikesTarget>,
    pub post_likes_active_cache_key: Option<String>,
    pub failed_post_likes_cache_keys: HashSet<String>,
    pub detail_context_posts: Vec<GetPostFeedResponse>,
    pub detail_context_anchor_id: Option<String>,
    pub create_post_upload_progress: f64,
    pub create_post_uploading: bool,
    pub publish_short_pending: bool,
    pub last_publish_error: Option<String>,
    pub publish_retry: Option<Rc<dyn Fn(&mut App)>>,
    active_create_uploads: i64,
    active_riff_only_uploads: i64,
    in_flight_own_publications: HashMap<String, i64>,
    get_post_last_error_text: Option<String>,
    user_posts_error_by_tag: HashMap<String, String>,
    user_posts_pending_tags: HashSet<String>,
    known_user_posts_keys: HashSet<String>,
    persistent_own_user_posts_keys: HashSet<String>,
    user_reposts_error_by_tag: HashMap<String, String>,
    user_reposts_pending_tags: HashSet<String>,
    known_user_reposts_keys: HashSet<String>,
    post_view_history_error: Option<String>,
    post_view_history_pending: bool,
    pub audio_posts_error_by_track: HashMap<String, String>,
    pub audio_posts_pending_tracks: HashSet<String>,
    pub known_audio_posts_keys: HashSet<String>,
}

impl PostServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let presentation = RisePostContentPresentation::shared(cx);
        cx.observe(&presentation, |_, _, cx| cx.notify()).detach();
        Self::default()
    }

    pub fn feed_last_error_text(&self) -> Option<&str> {
        self.feed_last_error_text.as_deref()
    }

    pub fn last_own_deleted_post_id(&self) -> Option<&str> {
        self.last_own_deleted_post_id.as_deref()
    }

    pub fn own_deleted_post_ids(&self) -> &HashSet<String> {
        &self.own_deleted_post_ids
    }

    pub fn live_deleted_post_ids(&self) -> &HashSet<String> {
        &self.live_deleted_post_ids
    }

    pub fn get_post_last_error_text(&self) -> Option<&str> {
        self.get_post_last_error_text.as_deref()
    }

    pub fn post_view_history_error(&self) -> Option<&str> {
        self.post_view_history_error.as_deref()
    }

    pub fn post_view_history_pending(&self) -> bool {
        self.post_view_history_pending
    }

    pub fn is_post_hidden(&self, post_id: &str) -> bool {
        self.own_deleted_post_ids.contains(post_id) || self.live_deleted_post_ids.contains(post_id)
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.feed_last_error_text = None;
        self.last_own_deleted_post_id = None;
        self.own_deleted_post_ids.clear();
        self.live_deleted_post_ids.clear();
        self.selected_post = None;
        self.post_likes_sheet_visible = false;
        self.post_likes_target = None;
        self.post_likes_active_cache_key = None;
        self.failed_post_likes_cache_keys.clear();
        self.detail_context_posts.clear();
        self.detail_context_anchor_id = None;
        self.create_post_upload_progress = 0.0;
        self.create_post_uploading = false;
        self.publish_short_pending = false;
        self.active_create_uploads = 0;
        self.active_riff_only_uploads = 0;
        self.last_publish_error = None;
        self.publish_retry = None;
        self.get_post_last_error_text = None;
        self.user_posts_error_by_tag.clear();
        self.user_posts_pending_tags.clear();
        self.known_user_posts_keys.clear();
        self.persistent_own_user_posts_keys.clear();
        self.user_reposts_error_by_tag.clear();
        self.user_reposts_pending_tags.clear();
        self.known_user_reposts_keys.clear();
        self.post_view_history_error = None;
        self.post_view_history_pending = false;
        self.audio_posts_error_by_track.clear();
        self.audio_posts_pending_tracks.clear();
        self.known_audio_posts_keys.clear();
        cx.notify();
    }

    pub fn feed_cache_key() -> String {
        SaiCache::new(["getPostsAction"]).key
    }

    pub fn user_posts_cache_key(tag: &str, sort: &str) -> String {
        SaiCache::new(["getUserPosts", tag, sort]).key
    }

    pub fn own_user_posts_projection_cache_key(tag: &str, sort: &str) -> String {
        SaiCache::new(["getOwnUserPostsProjection", tag, sort]).key
    }

    pub fn user_reposts_cache_key(tag: &str) -> String {
        SaiCache::new(["getUserReposts", tag]).key
    }

    pub fn post_view_history_cache_key(user_id: Option<&str>, cx: &mut App) -> String {
        let resolved_user_id = match user_id {
            Some(user_id) => user_id.to_owned(),
            None => AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default(),
        };
        SaiCache::new(["getPostViewHistory", resolved_user_id.as_str()]).key
    }

    pub fn post_cache_key(post_id: &str) -> String {
        SaiCache::new(["getPost", post_id]).key
    }

    pub fn own_post_cache_key(post_id: &str) -> String {
        SaiCache::new(["getOwnPost", post_id]).key
    }

    pub fn cached_feed_page(
        &self,
        key: &str,
        disk_cache_policy: SaiWsDiskCachePolicy,
        cx: &mut App,
    ) -> Option<PostServicesStoreCachedFeedPage> {
        Self::observed_cached_feed_page(key, disk_cache_policy, cx)
    }

    pub fn observed_cached_feed_page(
        key: &str,
        disk_cache_policy: SaiWsDiskCachePolicy,
        cx: &mut App,
    ) -> Option<PostServicesStoreCachedFeedPage> {
        let projection_key = format!("post.cached-feed-page.v1.{}", disk_cache_policy.raw_value());
        SaiWsResponseCache::observed_projection(
            key,
            &projection_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            disk_cache_policy,
            Self::cached_feed_page_from,
            cx,
        )
    }

    fn cached_feed_page_from(payload: Option<&Value>) -> Option<PostServicesStoreCachedFeedPage> {
        let payload = payload?;
        let root = AnyDictionaryBridge::dictionary(Some(payload))?;
        let raw_rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST).or_else(|| root.get(key::ITEMS)));

        if !raw_rows.iter().any(Self::has_client_only_post_state)
            && let Some(dto) = SaiJSONCodec::decode::<GetFeedGatewayDTO>(Some(payload))
        {
            let page = dto.as_virtual_list();
            return Some(PostServicesStoreCachedFeedPage {
                list: page.list,
                is_have_more: page.is_have_more_bot_or_top,
                relative_id: page.relative_id,
                feed_session_id: page.feed_session_id,
                next_cursor: None,
                next_cursor_post_id: None,
                total: Some(dto.total),
            });
        }

        Some(PostServicesStoreCachedFeedPage {
            list: raw_rows.iter().filter_map(Self::feed_response).collect(),
            is_have_more: Self::bool(root.get(key::IS_HAVE_MORE)),
            relative_id: Self::scalar_string(root.get(key::RELATIVE_ID)),
            feed_session_id: Self::scalar_string(root.get(key::FEED_SESSION_ID)),
            next_cursor: None,
            next_cursor_post_id: None,
            total: root
                .get(key::TOTAL)
                .and_then(|total| total.as_i64().or_else(|| total.as_f64().map(|value| value as i64))),
        })
    }

    fn has_client_only_post_state(row: &Map<String, Value>) -> bool {
        row.keys().any(|row_key| CLIENT_ONLY_POST_STATE_KEYS.contains(row_key))
    }

    fn feed_response(row: &Map<String, Value>) -> Option<GetPostFeedResponse> {
        let value = Value::Object(row.clone());
        if matches!(row.get(key::ID), Some(Value::String(_)))
            && let Some(cached) = SaiJSONCodec::decode::<GetPostFeedResponse>(Some(&value))
        {
            return Some(cached);
        }
        SaiJSONCodec::decode::<GetFeedPostItemDTO>(Some(&value))?.as_feed_response()
    }

    fn scalar_string(value: Option<&Value>) -> Option<String> {
        match value? {
            Value::String(text) => {
                let normalized = trimming_whitespaces_and_newlines(text);
                (!normalized.is_empty()).then(|| normalized.to_owned())
            }
            Value::Bool(flag) => Some(if *flag { "1" } else { "0" }.to_owned()),
            Value::Number(number) => {
                if let Some(int) = number.as_i64() {
                    return Some(int.to_string());
                }
                if let Some(int) = number.as_u64() {
                    return Some(int.to_string());
                }
                let double = number.as_f64().filter(|double| double.is_finite())?;
                Some(if double.round() == double { (double as i64).to_string() } else { double.to_string() })
            }
            _ => None,
        }
    }

    fn bool(value: Option<&Value>) -> Option<bool> {
        match value? {
            Value::Bool(flag) => Some(*flag),
            Value::Number(number) => number.as_f64().map(|double| double != 0.0),
            Value::String(text) => match trimming_whitespaces_and_newlines(text).to_lowercase().as_str() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    fn decoded<T: DeserializeOwned + Clone + 'static>(payload: Option<&dyn Any>) -> Option<T> {
        let payload = payload?;
        if let Some(typed) = payload.downcast_ref::<T>() {
            return Some(typed.clone());
        }
        SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>())
    }

    pub fn page(&self, scope: &RisePostPageScope, cx: &mut App) -> RisePostPagePresentation {
        RisePostContentPresentation::shared(cx).read(cx).page(scope, cx).into_owned()
    }

    pub fn feed_items(&self, cx: &mut App) -> Vec<GetPostFeedResponse> {
        self.feed_page(RisePostFeedKind::All, cx).items
    }

    pub fn feed_has_more(&self, cx: &mut App) -> bool {
        self.feed_page(RisePostFeedKind::All, cx).has_more
    }

    pub fn feed_relative_id(&self) -> Option<String> {
        None
    }

    pub fn feed_session_id(&self) -> Option<String> {
        None
    }

    pub fn feed_did_load(&self, cx: &mut App) -> bool {
        let status = self.feed_page(RisePostFeedKind::All, cx).status;
        status == RisePostPageStatus::Loaded || status == RisePostPageStatus::Failed
    }

    pub fn feed_page(&self, kind: RisePostFeedKind, cx: &mut App) -> RisePostPagePresentation {
        RisePostContentPresentation::shared(cx).read(cx).feed_page(kind, cx).into_owned()
    }

    pub fn profile_page(&self, user_id: &str, kind: RisePostProfileKind, cx: &mut App) -> RisePostPagePresentation {
        RisePostContentPresentation::shared(cx).read(cx).profile_page(user_id, kind, cx).into_owned()
    }

    pub fn profile_page_with_tag(
        &self,
        user_id: &str,
        tag: &str,
        kind: RisePostProfileKind,
        cx: &mut App,
    ) -> RisePostPagePresentation {
        RisePostContentPresentation::shared(cx).read(cx).profile_page_with_tag(user_id, tag, kind, cx).into_owned()
    }

    pub fn search_post_page(&self, query: &str, kind: RisePostSearchKind, cx: &mut App) -> RisePostPagePresentation {
        RisePostContentPresentation::shared(cx).read(cx).search_post_page(query, kind, cx).into_owned()
    }

    pub fn detail_page(&self, post_id: &str, cx: &mut App) -> RisePostPagePresentation {
        RisePostContentPresentation::shared(cx).read(cx).detail_page(post_id).clone()
    }

    fn user_posts_state(page: RisePostPagePresentation) -> PostServicesStoreUserPostsState {
        PostServicesStoreUserPostsState {
            did_load: page.status == RisePostPageStatus::Loaded || page.status == RisePostPageStatus::Failed,
            items: page.items,
            has_more: page.has_more,
            relative_id: None,
            feed_session_id: None,
            last_error_text: page.error_message,
        }
    }

    pub fn user_posts(&self, tag: &str, profile_user_id: Option<&str>, cx: &mut App) -> PostServicesStoreUserPostsState {
        let page = match profile_user_id {
            Some(user_id) => self.profile_page(user_id, RisePostProfileKind::Publications, cx),
            None => RisePostContentPresentation::shared(cx)
                .read(cx)
                .profile_page_by_tag(tag, RisePostProfileKind::Publications, cx)
                .into_owned(),
        };
        Self::user_posts_state(page)
    }

    pub fn user_posts_session(&self, tag: &str, profile_user_id: &str, cx: &mut App) -> Option<String> {
        let wire = Self::observed_cached_feed_page(
            &Self::user_posts_cache_key(tag, NEW_SORT),
            SaiWsDiskCachePolicy::MemoryOnly,
            cx,
        );
        if let Some(session) = wire.and_then(|page| page.feed_session_id) {
            return Some(session);
        }
        if !AuthServicesStore::shared(cx).read(cx).is_current_user(profile_user_id) {
            return None;
        }
        Self::observed_cached_feed_page(
            &Self::own_user_posts_projection_cache_key(tag, NEW_SORT),
            SaiWsDiskCachePolicy::Persistent,
            cx,
        )?
        .feed_session_id
    }

    pub fn hydrate_own_user_posts_wire_cache_if_needed(&self, tag: &str, sort: &str, cx: &mut App) {
        let wire_key = Self::user_posts_cache_key(tag, sort);
        if SaiWsResponseCache::memory_only(&wire_key, cx).is_some() {
            return;
        }
        let Some(projection) = SaiWsResponseCache::current_payload(
            &Self::own_user_posts_projection_cache_key(tag, sort),
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        ) else {
            return;
        };
        SaiWsResponseCache::hydrate_memory(&wire_key, projection, false, cx);
    }

    pub fn mark_user_posts_pending(&mut self, tag: &str, cx: &mut Context<Self>) {
        if tag.is_empty() {
            return;
        }
        self.user_posts_pending_tags.insert(tag.to_owned());
        cx.notify();
    }

    pub fn is_user_posts_pending(&self, tag: &str) -> bool {
        self.user_posts_pending_tags.contains(tag)
    }

    pub fn user_posts_status(&self, tag: &str, profile_user_id: Option<&str>, cx: &mut App) -> SaiWsSliceStatus {
        let state = self.user_posts(tag, profile_user_id, cx);
        self.user_posts_status_for_state(tag, &state)
    }

    pub fn user_posts_status_for_state(&self, tag: &str, state: &PostServicesStoreUserPostsState) -> SaiWsSliceStatus {
        if tag.is_empty() {
            return SaiWsSliceStatus::Idle;
        }
        if state.did_load {
            return SaiWsSliceStatus::Fulfilled;
        }
        if self.user_posts_pending_tags.contains(tag) {
            return SaiWsSliceStatus::Pending;
        }
        if state.last_error_text.is_some() || self.user_posts_error_by_tag.contains_key(tag) {
            return SaiWsSliceStatus::Rejected;
        }
        SaiWsSliceStatus::Idle
    }

    pub fn user_reposts(&self, tag: &str, cx: &mut App) -> PostServicesStoreUserPostsState {
        let page = RisePostContentPresentation::shared(cx)
            .read(cx)
            .profile_page_by_tag(tag, RisePostProfileKind::Reposts, cx)
            .into_owned();
        Self::user_posts_state(page)
    }

    pub fn mark_user_reposts_pending(&mut self, tag: &str, cx: &mut Context<Self>) {
        if tag.is_empty() {
            return;
        }
        self.user_reposts_pending_tags.insert(tag.to_owned());
        cx.notify();
    }

    pub fn is_user_reposts_pending(&self, tag: &str) -> bool {
        self.user_reposts_pending_tags.contains(tag)
    }

    pub fn user_reposts_status(&self, tag: &str, cx: &mut App) -> SaiWsSliceStatus {
        let state = self.user_reposts(tag, cx);
        self.user_reposts_status_for_state(tag, &state)
    }

    pub fn user_reposts_status_for_state(&self, tag: &str, state: &PostServicesStoreUserPostsState) -> SaiWsSliceStatus {
        if tag.is_empty() {
            return SaiWsSliceStatus::Idle;
        }
        if state.did_load {
            return SaiWsSliceStatus::Fulfilled;
        }
        if self.user_reposts_pending_tags.contains(tag) {
            return SaiWsSliceStatus::Pending;
        }
        if state.last_error_text.is_some() || self.user_reposts_error_by_tag.contains_key(tag) {
            return SaiWsSliceStatus::Rejected;
        }
        SaiWsSliceStatus::Idle
    }

    pub fn get_user_reposts_success_handler(
        &mut self,
        tag: &str,
        payload: Option<Box<dyn Any>>,
        cx: &mut Context<Self>,
    ) {
        self.user_reposts_pending_tags.remove(tag);
        self.user_reposts_error_by_tag.remove(tag);
        self.known_user_reposts_keys.insert(Self::user_reposts_cache_key(tag));
        self.cache_own_posts_from(payload.as_deref(), cx);
        cx.notify();
    }

    pub fn get_user_reposts_error_handler(&mut self, tag: &str, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.user_reposts_pending_tags.remove(tag);
        self.user_reposts_error_by_tag.insert(tag.to_owned(), error.to_string());
        let message = format!("get_user_reposts tag={tag} error: {error}");
        Sai::log("Post", move || message, SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn post_view_history(&self, cx: &mut App) -> PostServicesStorePostViewHistoryState {
        let cache_key = Self::post_view_history_cache_key(None, cx);
        let Some(payload) = SaiWsResponseCache::observed_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::MemoryOnly,
            cx,
        ) else {
            return PostServicesStorePostViewHistoryState {
                last_error_text: self.post_view_history_error.clone(),
                did_load: false,
                ..PostServicesStorePostViewHistoryState::default()
            };
        };

        let page = SaiJSONCodec::decode::<PostServicesStoreCachedFeedPage>(Some(&payload));

        PostServicesStorePostViewHistoryState {
            has_more: page.as_ref().and_then(|page| page.is_have_more).unwrap_or(false),
            next_cursor: page.as_ref().and_then(|page| page.next_cursor),
            next_cursor_post_id: page.as_ref().and_then(|page| page.next_cursor_post_id),
            items: page.map(|page| page.list).unwrap_or_default(),
            last_error_text: self.post_view_history_error.clone(),
            did_load: true,
        }
    }

    pub fn mark_post_view_history_pending(&mut self, cx: &mut Context<Self>) {
        self.post_view_history_pending = true;
        cx.notify();
    }

    pub fn post_view_history_status(&self, cx: &mut App) -> SaiWsSliceStatus {
        if self.post_view_history(cx).did_load {
            return SaiWsSliceStatus::Fulfilled;
        }
        if self.post_view_history_pending {
            return SaiWsSliceStatus::Pending;
        }
        if self.post_view_history_error.is_some() {
            return SaiWsSliceStatus::Rejected;
        }
        SaiWsSliceStatus::Idle
    }

    pub fn get_post_view_history_success_handler(
        &mut self,
        payload: Option<Box<dyn Any>>,
        is_first_page: bool,
        keeps_loaded_tail: bool,
        cx: &mut Context<Self>,
    ) {
        self.post_view_history_pending = false;
        cx.notify();

        let Some(dto) = Self::decoded::<GetViewHistoryGatewayDTO>(payload.as_deref()) else {
            return;
        };

        self.post_view_history_error = None;
        self.cache_own_posts(&Self::view_history_rows(&dto), cx);
        if is_first_page && keeps_loaded_tail && self.revalidate_view_history(&dto, cx) {
            return;
        }
        Self::write_view_history_page(&dto, is_first_page, cx);
    }

    fn view_history_rows(dto: &GetViewHistoryGatewayDTO) -> Vec<GetPostFeedResponse> {
        dto.list.iter().filter_map(GetFeedPostItemDTOExt::as_feed_response).collect()
    }

    fn encoded_rows(posts: &[GetPostFeedResponse]) -> Value {
        Value::Array(posts.iter().filter_map(SaiJSONCodec::encode).map(Value::Object).collect())
    }

    fn revalidate_view_history(&self, dto: &GetViewHistoryGatewayDTO, cx: &mut App) -> bool {
        let fresh = Self::view_history_rows(dto);
        let loaded = self.post_view_history(cx);
        if !dto.is_have_more || !loaded.did_load || loaded.items.len() <= fresh.len() {
            return false;
        }
        let fresh_ids: HashSet<&str> = fresh.iter().map(|post| post.id.as_str()).collect();
        let tail: Vec<GetPostFeedResponse> =
            loaded.items.iter().filter(|post| !fresh_ids.contains(post.id.as_str())).cloned().collect();
        let mut rows = fresh.clone();
        rows.extend(tail);
        let mut page_dict = Map::new();
        page_dict.insert(key::LIST.to_owned(), Self::encoded_rows(&rows));
        page_dict.insert(key::IS_HAVE_MORE.to_owned(), Value::Bool(loaded.has_more));
        page_dict.insert(key::NEXT_CURSOR.to_owned(), Value::from(loaded.next_cursor));
        page_dict.insert(key::NEXT_CURSOR_POST_ID.to_owned(), Value::from(loaded.next_cursor_post_id));
        let cache_key = Self::post_view_history_cache_key(None, cx);
        let _ = SaiUpdater::replace_root(&cache_key, page_dict, SaiUpdaterWriteScope::Cache, cx);
        true
    }

    pub fn get_post_view_history_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.post_view_history_pending = false;
        self.post_view_history_error = Some(error.to_string());
        let message = format!("get_view_history error: {error}");
        Sai::log("Post", move || message, SaiLogKind::Network, cx);
        cx.notify();
    }

    fn write_view_history_page(dto: &GetViewHistoryGatewayDTO, is_first_page: bool, cx: &mut App) {
        let cache_key = Self::post_view_history_cache_key(None, cx);
        let mut page_dict = Map::new();
        page_dict.insert(key::LIST.to_owned(), Self::encoded_rows(&Self::view_history_rows(dto)));
        page_dict.insert(key::IS_HAVE_MORE.to_owned(), Value::Bool(dto.is_have_more));
        page_dict.insert(key::NEXT_CURSOR.to_owned(), Value::from(dto.next_cursor));
        page_dict.insert(key::NEXT_CURSOR_POST_ID.to_owned(), Value::from(dto.next_cursor_post_id));

        if is_first_page {
            let _ = SaiUpdater::replace_root(&cache_key, page_dict, SaiUpdaterWriteScope::Cache, cx);
        } else {
            let _ = SaiUpdater::merge_paginated_page(
                &cache_key,
                key::LIST,
                Some(key::ID),
                &Value::Object(page_dict),
                SaiUpdaterWriteScope::Cache,
                cx,
            );
        }
    }

    pub fn post_view_history_snapshot(&self, cx: &mut App) -> Option<Map<String, Value>> {
        let cache_key = Self::post_view_history_cache_key(None, cx);
        let payload = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::MemoryOnly,
            cx,
        );
        AnyDictionaryBridge::dictionary(payload.as_ref())
    }

    pub fn clear_view_history_local(&mut self, cx: &mut Context<Self>) {
        let mut page_dict = Map::new();
        page_dict.insert(key::LIST.to_owned(), Value::Array(Vec::new()));
        page_dict.insert(key::IS_HAVE_MORE.to_owned(), Value::Bool(false));
        page_dict.insert(key::NEXT_CURSOR.to_owned(), Value::Null);
        page_dict.insert(key::NEXT_CURSOR_POST_ID.to_owned(), Value::Null);

        let cache_key = Self::post_view_history_cache_key(None, cx);
        let _ = SaiUpdater::replace_root(&cache_key, page_dict, SaiUpdaterWriteScope::Cache, cx);
    }

    pub fn restore_view_history(&mut self, snapshot: Map<String, Value>, cx: &mut Context<Self>) {
        let cache_key = Self::post_view_history_cache_key(None, cx);
        let _ = SaiUpdater::replace_root(&cache_key, snapshot, SaiUpdaterWriteScope::Cache, cx);
    }

    pub fn remove_post_from_view_history_optimistic(
        &mut self,
        post_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<Map<String, Value>> {
        let snapshot = self.post_view_history_snapshot(cx)?;
        let cache_key = Self::post_view_history_cache_key(None, cx);
        let _ = SaiUpdater::remove_row(&cache_key, key::LIST, key::ID, post_id, SaiUpdaterWriteScope::Cache, cx);
        Some(snapshot)
    }

    pub fn remove_repost_from_tab(&mut self, tag: &str, repost_id: &str, cx: &mut Context<Self>) {
        let cache_key = Self::user_reposts_cache_key(tag);
        let _ = SaiUpdater::remove_row(&cache_key, key::LIST, key::ID, repost_id, SaiUpdaterWriteScope::Cache, cx);
    }

    fn fresh_detail_post(&self, post_id: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
        let presentation = RisePostContentPresentation::shared(cx);
        let page = presentation.read(cx).detail_page(post_id);
        if page.status != RisePostPageStatus::Loaded
            || page.error_message.is_some()
            || page.revision < self.comments_post_taken_at_milliseconds
        {
            return None;
        }
        page.items.first().filter(|fresh| fresh.id == post_id).cloned()
    }

    pub fn sync_selected_post_comment_state(&mut self, post_id: &str, cx: &mut Context<Self>) {
        let Some(mut selected) = self.selected_post.clone().filter(|selected| selected.id == post_id) else {
            return;
        };
        let Some(fresh) = self.fresh_detail_post(post_id, cx) else {
            return;
        };
        if selected.comments_count == fresh.comments_count
            && selected.main_comments_count == fresh.main_comments_count
            && selected.can_comment == fresh.can_comment
        {
            return;
        }
        selected.comments_count = fresh.comments_count;
        selected.main_comments_count = fresh.main_comments_count;
        selected.can_comment = fresh.can_comment;
        self.selected_post = Some(selected);
        cx.notify();
    }

    pub fn select_post_for_comments(&mut self, post: GetPostFeedResponse, cx: &mut Context<Self>) {
        self.selected_post = Some(post);
        self.comments_post_taken_at_milliseconds = (Date::now().time_interval_since_1970() * 1000.0) as i64;
        cx.notify();
    }

    pub fn set_author_relationship_on_held_posts(
        &mut self,
        author_id: &str,
        relationship: UserListRelationship,
        cx: &mut Context<Self>,
    ) {
        if author_id.is_empty() {
            return;
        }
        let patched = |post: &GetPostFeedResponse| -> Option<GetPostFeedResponse> {
            let is_author = post.author.id.as_deref() == Some(author_id) || post.author_id == author_id;
            let differs = post.author.is_subscribed != relationship.is_subbed
                || post.author.is_subscriber != relationship.is_subscriber
                || post.author.is_friend != relationship.is_friend;
            if !is_author || !differs {
                return None;
            }
            let mut next = post.clone();
            next.author.is_subscribed = relationship.is_subbed;
            next.author.is_subscriber = relationship.is_subscriber;
            next.author.is_friend = relationship.is_friend;
            Some(next)
        };
        if let Some(next) = self.selected_post.as_ref().and_then(patched) {
            self.selected_post = Some(next);
        }
        for index in 0..self.detail_context_posts.len() {
            if let Some(next) = patched(&self.detail_context_posts[index]) {
                self.detail_context_posts[index] = next;
            }
        }
        cx.notify();
    }

    pub fn selected_post_comments_count(&self, cx: &mut App) -> Option<i64> {
        let selected = self.selected_post.as_ref()?;
        match self.fresh_detail_post(&selected.id, cx) {
            Some(fresh) => Some(fresh.comments_count),
            None => Some(selected.comments_count),
        }
    }

    pub fn resolved_post(&self, post_id: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
        if let Some(invented) = PostGuideSandboxHooks::post(post_id, cx) {
            return Some(invented);
        }
        if let Some(post) = RisePostContentPresentation::shared(cx).read(cx).resolved_post(post_id) {
            return Some(post.clone());
        }
        self.selected_post.as_ref().filter(|selected| selected.id == post_id).cloned()
    }

    pub fn get_feed_success_handler(&mut self, payload: Option<Box<dyn Any>>, cx: &mut Context<Self>) {
        self.feed_last_error_text = None;
        self.wire_loaded_feed_account_id =
            AuthServicesStore::shared(cx).read(cx).active_account_id().map(str::to_owned);
        self.cache_own_posts_from(payload.as_deref(), cx);
        cx.notify();
    }

    pub fn get_feed_error_handler(&mut self, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.feed_last_error_text = Some(error.to_string());
        let message = format!("get_feed error: {error}");
        Sai::log("Post", move || message, SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn get_user_posts_success_handler(
        &mut self,
        tag: &str,
        sort: &str,
        is_own_profile: bool,
        is_first_page: bool,
        payload: Option<Box<dyn Any>>,
        cx: &mut Context<Self>,
    ) {
        self.user_posts_pending_tags.remove(tag);
        self.user_posts_error_by_tag.remove(tag);
        let wire_key = Self::user_posts_cache_key(tag, sort);
        let projection_key = Self::own_user_posts_projection_cache_key(tag, sort);
        self.known_user_posts_keys.insert(wire_key);
        if is_own_profile {
            self.persist_own_user_posts_projection(tag, sort, is_first_page, payload.as_deref(), cx);
        } else {
            self.persistent_own_user_posts_keys.remove(&projection_key);
        }
        self.cache_own_posts_from(payload.as_deref(), cx);
        cx.notify();
    }

    pub fn sanitized_own_user_posts_page(
        payload: Option<&dyn Any>,
        mut is_own_post: impl FnMut(&GetPostFeedResponse) -> bool,
    ) -> Option<PostServicesStoreCachedFeedPage> {
        let dto = Self::decoded::<GetFeedGatewayDTO>(payload)?;

        let page = dto.as_virtual_list();
        let has_more = page.is_have_more_bot_or_top.unwrap_or(page.relative_id.is_some());
        let relative_id = if has_more {
            page.relative_id.clone().or_else(|| page.list.last().map(|post| post.id.clone()))
        } else {
            None
        };
        Some(PostServicesStoreCachedFeedPage {
            list: page
                .list
                .into_iter()
                .filter(|post| {
                    !post.id.is_empty()
                        && !post.is_temp
                        && !post.is_uploading
                        && !post.is_compressing
                        && is_own_post(post)
                })
                .collect(),
            is_have_more: Some(has_more),
            relative_id,
            feed_session_id: page.feed_session_id,
            next_cursor: None,
            next_cursor_post_id: None,
            total: None,
        })
    }

    fn persist_own_user_posts_projection(
        &mut self,
        tag: &str,
        sort: &str,
        is_first_page: bool,
        payload: Option<&dyn Any>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut incoming) =
            Self::sanitized_own_user_posts_page(payload, |post| PostMediaCachePolicy::is_own_post(post, cx))
        else {
            return;
        };

        let cache_key = Self::own_user_posts_projection_cache_key(tag, sort);
        self.activate_persistent_own_user_posts_key(&cache_key, cx);
        if !is_first_page
            && let Some(existing) = Self::observed_cached_feed_page(&cache_key, SaiWsDiskCachePolicy::Persistent, cx)
        {
            let mut known_ids: HashSet<String> = existing.list.iter().map(|post| post.id.clone()).collect();
            let unique_incoming: Vec<GetPostFeedResponse> =
                incoming.list.into_iter().filter(|post| known_ids.insert(post.id.clone())).collect();
            let mut merged = existing.list;
            merged.extend(unique_incoming);
            incoming.list = merged;
            incoming.feed_session_id = incoming.feed_session_id.or(existing.feed_session_id);
        }

        let Some(root) = SaiJSONCodec::encode(&incoming) else {
            return;
        };
        let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx);
    }

    pub fn get_user_posts_error_handler(&mut self, tag: &str, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.user_posts_pending_tags.remove(tag);
        self.user_posts_error_by_tag.insert(tag.to_owned(), error.to_string());
        let message = format!("get_user_posts tag={tag} error: {error}");
        Sai::log("Post", move || message, SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn get_post_success_handler(&mut self, payload: Option<Box<dyn Any>>, cx: &mut Context<Self>) {
        let Some(dto) = payload.as_deref().and_then(|payload| payload.downcast_ref::<CreatePostGatewayDTO>()) else {
            return;
        };
        let Some(post) = dto.post.as_ref().and_then(GetFeedPostItemDTOExt::as_feed_response) else {
            return;
        };

        self.get_post_last_error_text = None;
        self.selected_post = Some(post.clone());

        let own_cache_key = Self::own_post_cache_key(&post.id);
        if !Self::cache_own_post_snapshot(&post, cx) {
            SaiWsResponseCache::remove(&own_cache_key, SaiCacheCleanLayer::Everywhere, cx);
        }

        self.patch_post_everywhere(&post, false, false, cx);
    }

    pub fn cache_own_posts_from(&self, payload: Option<&dyn Any>, cx: &mut App) {
        Self::cache_own_posts_from_payload(payload, cx);
    }

    pub fn cache_own_posts_from_payload(payload: Option<&dyn Any>, cx: &mut App) {
        let Some(dto) = Self::decoded::<GetFeedGatewayDTO>(payload) else {
            return;
        };
        for post in &dto.as_virtual_list().list {
            let _ = Self::cache_own_post_snapshot(post, cx);
        }
    }

    pub fn cache_own_posts(&self, posts: &[GetPostFeedResponse], cx: &mut App) {
        for post in posts {
            let _ = Self::cache_own_post_snapshot(post, cx);
        }
    }

    fn cache_own_post_snapshot(post: &GetPostFeedResponse, cx: &mut App) -> bool {
        if post.id.is_empty()
            || post.is_temp
            || post.is_uploading
            || post.is_compressing
            || !PostMediaCachePolicy::is_own_post(post, cx)
        {
            return false;
        }
        let Some(encoded) = SaiJSONCodec::encode(post) else {
            return false;
        };
        let _ = SaiUpdater::replace_root(&Self::own_post_cache_key(&post.id), encoded, SaiUpdaterWriteScope::All, cx);
        true
    }

    fn own_post_from_cache(cache_key: &str, cx: &mut App) -> Option<GetPostFeedResponse> {
        let payload = SaiWsResponseCache::current_payload(
            cache_key,
            SaiWsCachePreferredLayer::DiskFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        )?;
        SaiJSONCodec::decode::<GetPostFeedResponse>(Some(&payload))
    }

    pub fn hydrate_own_post_from_cache(&mut self, post_id: &str, cx: &mut Context<Self>) -> bool {
        let cache_key = Self::own_post_cache_key(post_id);
        let Some(post) = Self::own_post_from_cache(&cache_key, cx) else {
            return false;
        };
        if post.is_temp || post.is_uploading || post.is_compressing || !PostMediaCachePolicy::is_own_post(&post, cx) {
            SaiWsResponseCache::remove(&cache_key, SaiCacheCleanLayer::Everywhere, cx);
            return false;
        }
        self.selected_post = Some(post.clone());
        self.patch_post_everywhere(&post, false, false, cx);
        true
    }

    pub fn get_post_error_handler(&mut self, post_id: &str, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.get_post_last_error_text = Some(error.to_string());
        let message = format!("get_post error postId={post_id}: {error}");
        Sai::log("Post", move || message, SaiLogKind::Network, cx);
        cx.notify();
    }

    pub fn post_stats(&self, post_id: &str, cx: &mut App) -> Option<RiseStoredPostStats> {
        if let Some(invented) = PostGuideSandboxHooks::post_stats(post_id, cx) {
            return Some(invented);
        }
        RisePostPresentation::shared(cx).read(cx).stats(post_id)?.value.clone()
    }

    pub fn post_stats_error_text(&self, post_id: &str, cx: &mut App) -> Option<String> {
        RisePostPresentation::shared(cx).read(cx).stats(post_id)?.error_message.clone()
    }

    pub fn post_stats_status(&self, post_id: &str, cx: &mut App) -> SaiWsSliceStatus {
        if PostGuideSandboxHooks::post_stats(post_id, cx).is_some() {
            return SaiWsSliceStatus::Fulfilled;
        }
        let presentation = RisePostPresentation::shared(cx);
        let Some(state) = presentation.read(cx).stats(post_id) else {
            return SaiWsSliceStatus::Idle;
        };
        if state.is_loading {
            return SaiWsSliceStatus::Pending;
        }
        if state.value.is_some() {
            return SaiWsSliceStatus::Fulfilled;
        }
        if state.error_message.is_some() {
            return SaiWsSliceStatus::Rejected;
        }
        SaiWsSliceStatus::Idle
    }

    pub fn post_view_count(&self, post: &GetPostFeedResponse, cx: &mut App) -> i64 {
        RisePostPresentation::shared(cx)
            .read(cx)
            .view_count(&post.rise_view_target_post_id())
            .unwrap_or(post.views_count)
    }

    pub fn patch_feed_row(post: &GetPostFeedResponse, cx: &mut App) {
        Self::transform_row_in_cache(
            &Self::feed_cache_key(),
            &post.id,
            SaiUpdaterWriteScope::Cache,
            &mut |_| post.clone(),
            cx,
        );
    }

    pub fn begin_create_upload(&mut self, is_riff_only: bool, cx: &mut Context<Self>) {
        self.active_create_uploads += 1;
        if is_riff_only {
            self.active_riff_only_uploads += 1;
        }
        self.create_post_upload_progress = 0.0;
        self.create_post_uploading = true;
        self.publish_short_pending = self.active_riff_only_uploads > 0;
        cx.notify();
    }

    pub fn finish_create_upload(&mut self, is_riff_only: bool, cx: &mut Context<Self>) {
        self.active_create_uploads = (self.active_create_uploads - 1).max(0);
        if is_riff_only {
            self.active_riff_only_uploads = (self.active_riff_only_uploads - 1).max(0);
        }
        self.create_post_uploading = self.active_create_uploads > 0;
        self.publish_short_pending = self.active_riff_only_uploads > 0;
        cx.notify();
    }

    pub fn set_upload_progress(&mut self, value: f64, cx: &mut Context<Self>) {
        self.create_post_upload_progress = value;
        cx.notify();
    }

    pub fn adjust_own_publications_count(&mut self, delta: i64, user_id: &str, cx: &mut Context<Self>) {
        let auth = AuthServicesStore::shared(cx);
        let Some(mut profile) = auth.read(cx).profile().filter(|profile| profile.id == user_id).cloned() else {
            return;
        };
        profile.posts_count = (profile.posts_count + delta).max(0);
        auth.update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
    }

    pub fn begin_own_publication_upload(&mut self, user_id: &str, cx: &mut Context<Self>) {
        *self.in_flight_own_publications.entry(user_id.to_owned()).or_insert(0) += 1;
        self.adjust_own_publications_count(1, user_id, cx);
    }

    pub fn end_own_publication_upload(&mut self, user_id: &str, _cx: &mut Context<Self>) {
        let left = self.in_flight_own_publications.get(user_id).copied().unwrap_or(0) - 1;
        if left > 0 {
            self.in_flight_own_publications.insert(user_id.to_owned(), left);
        } else {
            self.in_flight_own_publications.remove(user_id);
        }
    }

    pub fn in_flight_own_publication_count(&self, user_id: &str) -> i64 {
        self.in_flight_own_publications.get(user_id).copied().unwrap_or(0)
    }

    pub fn reconcile_own_publications_count(&mut self, cx: &mut Context<Self>) {
        UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_my_profile(cx));
    }

    pub fn mark_own_post_deleted(&mut self, post_id: Option<&str>, cx: &mut Context<Self>) {
        self.last_own_deleted_post_id = post_id.map(str::to_owned);
        if let Some(post_id) = post_id {
            self.own_deleted_post_ids.insert(post_id.to_owned());
        }
        cx.notify();
    }

    pub fn restore_own_deleted_post(&mut self, post_id: &str, cx: &mut Context<Self>) {
        self.own_deleted_post_ids.remove(post_id);
        if self.last_own_deleted_post_id.as_deref() == Some(post_id) {
            self.last_own_deleted_post_id = None;
        }
        cx.notify();
    }

    pub fn counts_as_profile_publication(content_kind: &str, show_in_publications: bool) -> bool {
        content_kind == RisePostContentKind::Publication.wire() && show_in_publications
    }

    pub fn set_publish_error(&mut self, message: Option<String>, cx: &mut Context<Self>) {
        self.last_publish_error = message.as_deref().map(RisePostPublishFailure::display_text);
        cx.notify();
    }

    fn put_row_at_start(cache_key: &str, remove_id: &str, row: Map<String, Value>, cx: &mut App) {
        let _ = SaiUpdater::remove_row(cache_key, key::LIST, key::ID, remove_id, SaiUpdaterWriteScope::Cache, cx);
        let _ = SaiUpdater::insert_row_if_missing(
            cache_key,
            key::LIST,
            key::ID,
            row,
            true,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
    }

    pub fn insert_optimistic_user_post(&mut self, post: &GetPostFeedResponse, tag: &str, cx: &mut Context<Self>) {
        if tag.is_empty() {
            return;
        }
        let Some(row) = SaiJSONCodec::encode(post) else {
            return;
        };

        let cache_key = Self::user_posts_cache_key(tag, NEW_SORT);
        self.known_user_posts_keys.insert(cache_key.clone());

        Self::put_row_at_start(&cache_key, &post.id, row, cx);
    }

    pub fn patch_optimistic_post_progress(
        &mut self,
        temp_id: &str,
        tag: &str,
        is_compressing: bool,
        compression_progress: f64,
        upload_progress: f64,
        cx: &mut Context<Self>,
    ) {
        if tag.is_empty() {
            return;
        }

        let cache_key = Self::user_posts_cache_key(tag, NEW_SORT);
        let mut patch = Map::new();
        patch.insert(RiseLocalKey::IS_COMPRESSING.to_owned(), Value::Bool(is_compressing));
        patch.insert(RiseLocalKey::COMPRESSION_PROGRESS.to_owned(), Value::from(compression_progress));
        patch.insert(RiseLocalKey::IS_UPLOADING.to_owned(), Value::Bool(!is_compressing || upload_progress < 100.0));
        patch.insert(RiseLocalKey::UPLOAD_PROGRESS.to_owned(), Value::from(upload_progress));

        let _ = SaiUpdater::patch_row(&cache_key, key::LIST, key::ID, temp_id, &patch, SaiUpdaterWriteScope::Cache, cx);
    }

    pub fn remove_optimistic_user_post(&mut self, temp_id: &str, tag: &str, cx: &mut Context<Self>) {
        if tag.is_empty() {
            return;
        }
        let cache_key = Self::user_posts_cache_key(tag, NEW_SORT);
        let _ = SaiUpdater::remove_row(&cache_key, key::LIST, key::ID, temp_id, SaiUpdaterWriteScope::Cache, cx);
    }

    pub fn replace_optimistic_user_post(
        &mut self,
        temp_id: &str,
        tag: &str,
        post: &GetPostFeedResponse,
        cx: &mut Context<Self>,
    ) {
        if tag.is_empty() {
            self.apply_created_post(post, cx);
            return;
        }

        let Some(row) = SaiJSONCodec::encode(post) else {
            self.apply_created_post(post, cx);
            return;
        };

        let cache_key = Self::user_posts_cache_key(tag, NEW_SORT);
        self.known_user_posts_keys.insert(cache_key.clone());

        Self::put_row_at_start(&cache_key, temp_id, row, cx);

        self.upsert_own_user_post_projection(post, tag, NEW_SORT, cx);

        let _ = Self::cache_own_post_snapshot(post, cx);
        Self::insert_feed_row(post, cx);
        Self::bump_profile_posts_count(cx);
    }

    pub fn apply_like_state(&mut self, post_id: &str, liked: bool, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                if p.is_liked != liked {
                    p.is_liked = liked;
                    p.likes_count = (p.likes_count + if liked { 1 } else { -1 }).max(0);
                }
                p
            },
            cx,
        );
    }

    pub fn apply_favorite_state(&mut self, post_id: &str, favorited: bool, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                if p.is_favorited != favorited {
                    p.is_favorited = favorited;
                    p.favorites_count = (p.favorites_count + if favorited { 1 } else { -1 }).max(0);
                }
                p
            },
            cx,
        );
    }

    pub fn apply_sparks_delta(&mut self, post_id: &str, delta: i64, cx: &mut Context<Self>) {
        if delta == 0 {
            return;
        }
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                p.sparks_total = (p.sparks_total + delta).max(0);
                p.my_sparks = (p.my_sparks + delta).max(0);
                p
            },
            cx,
        );
    }

    pub fn apply_sparks_sent(
        &mut self,
        post_id: &str,
        amount: i64,
        total: Option<i64>,
        mine: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                p.sparks_total = total.unwrap_or(0).max((p.sparks_total - amount).max(0));
                p.my_sparks = mine.unwrap_or(p.my_sparks).max(0);
                p
            },
            cx,
        );
    }

    pub fn apply_repost_state(&mut self, post_id: &str, reposted: bool, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                if p.is_reposted == reposted {
                    return p;
                }
                p.is_reposted = reposted;
                p.repost_count = (p.repost_count + if reposted { 1 } else { -1 }).max(0);
                if reposted {
                    if p.my_repost_content.is_none() {
                        p.my_repost_content = Some(String::new());
                    }
                } else {
                    p.my_repost_id = None;
                    p.my_repost_content = None;
                }
                p
            },
            cx,
        );
    }

    pub fn apply_repost_optimistic(&mut self, original_id: &str, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            original_id,
            &mut |mut p| {
                p.is_reposted = true;
                p.repost_count += 1;
                if p.my_repost_content.is_none() {
                    p.my_repost_content = Some(String::new());
                }
                p
            },
            cx,
        );
    }

    pub fn confirm_repost(&mut self, original_id: &str, repost: &GetPostFeedResponse, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            original_id,
            &mut |mut p| {
                p.is_reposted = true;
                p.my_repost_id = Some(repost.id.clone());
                if !repost.content.is_empty() {
                    p.my_repost_content = Some(repost.content.clone());
                } else if p.my_repost_content.is_none() {
                    p.my_repost_content = Some(String::new());
                }
                p
            },
            cx,
        );
    }

    pub fn clear_repost_optimistic(&mut self, original_id: &str, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            original_id,
            &mut |mut p| {
                p.is_reposted = false;
                p.my_repost_id = None;
                p.my_repost_content = None;
                p.repost_count = (p.repost_count - 1).max(0);
                p
            },
            cx,
        );
    }

    pub fn restore_repost_optimistic(
        &mut self,
        original_id: &str,
        repost_id: Option<&str>,
        content: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.transform_post_everywhere(
            original_id,
            &mut |mut p| {
                p.is_reposted = true;
                p.my_repost_id = repost_id.map(str::to_owned);
                p.my_repost_content = Some(content.unwrap_or_default().to_owned());
                p.repost_count += 1;
                p
            },
            cx,
        );
    }

    pub fn apply_repost_opinion(
        &mut self,
        post_id: &str,
        content: Option<&str>,
        entities: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let normalized = content.map(|content| trimming_whitespaces_and_newlines(content).to_owned());
        let (normalized_entities, _) = CommentEntities::normalized(entities);
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                if p.my_repost_content == normalized && p.my_repost_entities == normalized_entities {
                    return p;
                }
                p.my_repost_content = normalized.clone();
                p.my_repost_entities = normalized_entities.clone();
                p
            },
            cx,
        );
    }

    pub fn patch_repost_count(
        &mut self,
        original_id: &str,
        count: i64,
        is_reposted: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        self.transform_post_everywhere(
            original_id,
            &mut |mut p| {
                p.repost_count = count.max(0);
                if let Some(is_reposted) = is_reposted {
                    p.is_reposted = is_reposted;
                }
                p
            },
            cx,
        );
    }

    pub fn post_reposted_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::PostReposted(event)) = RiseEvent::decode_payload(name::POST_REPOSTED, payload) else {
            return;
        };
        if event.post_id <= 0 {
            return;
        }
        self.patch_repost_count(&event.post_id.to_string(), event.repost_count, None, cx);
    }

    pub fn apply_created_post(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        let _ = Self::cache_own_post_snapshot(post, cx);
        Self::insert_feed_row(post, cx);

        let tag = if post.author.tag.is_empty() {
            AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.tag.clone()).unwrap_or_default()
        } else {
            post.author.tag.clone()
        };
        if !tag.is_empty()
            && let Some(row) = SaiJSONCodec::encode(post)
        {
            let cache_key = Self::user_posts_cache_key(&tag, NEW_SORT);
            self.known_user_posts_keys.insert(cache_key.clone());

            Self::put_row_at_start(&cache_key, &post.id, row, cx);

            self.upsert_own_user_post_projection(post, &tag, NEW_SORT, cx);
        }

        Self::bump_profile_posts_count(cx);
    }

    fn insert_feed_row(post: &GetPostFeedResponse, cx: &mut App) {
        let Some(row) = SaiJSONCodec::encode(post) else {
            return;
        };
        Self::put_row_at_start(&Self::feed_cache_key(), &post.id, row, cx);
    }

    fn bump_profile_posts_count(cx: &mut App) {
        let auth = AuthServicesStore::shared(cx);
        if let Some(mut profile) = auth.read(cx).profile().cloned() {
            profile.posts_count += 1;
            auth.update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
        }
    }

    fn flip_like_in_place(p: &mut GetPostFeedResponse) -> bool {
        let was_liked = p.is_liked;
        p.is_liked = !p.is_liked;
        p.likes_count = (p.likes_count + if p.is_liked { 1 } else { -1 }).max(0);
        was_liked
    }

    fn flip_favorite_in_place(p: &mut GetPostFeedResponse) -> bool {
        let was_fav = p.is_favorited;
        p.is_favorited = !p.is_favorited;
        p.favorites_count = (p.favorites_count + if p.is_favorited { 1 } else { -1 }).max(0);
        was_fav
    }

    fn transform_row_in_cache(
        cache_key: &str,
        post_id: &str,
        write_to: SaiUpdaterWriteScope,
        transform: PostTransform<'_>,
        cx: &mut App,
    ) {
        let _ = SaiUpdater::update_json_value(
            cache_key,
            |ctx| {
                let mut root = AnyDictionaryBridge::dictionary(ctx.payload.as_ref()).unwrap_or_default();
                let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST));
                let Some(idx) =
                    rows.iter().position(|row| Self::scalar_string(row.get(key::ID)).as_deref() == Some(post_id))
                else {
                    return Ok(Value::Object(root));
                };
                let Some(model) = Self::feed_response(&rows[idx]) else {
                    return Ok(Value::Object(root));
                };
                let Some(encoded) = SaiJSONCodec::encode(&transform(model)) else {
                    return Ok(Value::Object(root));
                };
                rows[idx] = encoded;
                root.insert(key::LIST.to_owned(), Value::Array(rows.into_iter().map(Value::Object).collect()));
                Ok(Value::Object(root))
            },
            write_to,
            cx,
        );
    }

    fn current_own_user_posts_cache_key(cx: &mut App) -> Option<String> {
        let raw_tag = AuthServicesStore::shared(cx).read(cx).profile()?.tag.clone();
        let tag = trimming_whitespaces_and_newlines(&raw_tag);
        if tag.is_empty() {
            return None;
        }
        Some(Self::own_user_posts_projection_cache_key(tag, NEW_SORT))
    }

    fn is_persistent_own_user_posts_key(&self, cache_key: &str, cx: &mut App) -> bool {
        self.persistent_own_user_posts_keys.contains(cache_key)
            || Self::current_own_user_posts_cache_key(cx).as_deref() == Some(cache_key)
    }

    fn activate_persistent_own_user_posts_key(&mut self, cache_key: &str, cx: &mut App) {
        self.persistent_own_user_posts_keys.insert(cache_key.to_owned());
        let _ = SaiWsResponseCache::current_payload(
            cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::Persistent,
            cx,
        );
    }

    fn upsert_own_user_post_projection(
        &mut self,
        post: &GetPostFeedResponse,
        tag: &str,
        sort: &str,
        cx: &mut Context<Self>,
    ) {
        if post.id.is_empty()
            || post.is_temp
            || post.is_uploading
            || post.is_compressing
            || !PostMediaCachePolicy::is_own_post(post, cx)
        {
            return;
        }

        let cache_key = Self::own_user_posts_projection_cache_key(tag, sort);
        self.activate_persistent_own_user_posts_key(&cache_key, cx);
        let mut page = Self::observed_cached_feed_page(&cache_key, SaiWsDiskCachePolicy::Persistent, cx).unwrap_or(
            PostServicesStoreCachedFeedPage {
                list: Vec::new(),
                is_have_more: Some(true),
                relative_id: None,
                feed_session_id: None,
                next_cursor: None,
                next_cursor_post_id: None,
                total: None,
            },
        );
        page.list.retain(|existing| existing.id != post.id);
        page.list.insert(0, post.clone());
        let Some(root) = SaiJSONCodec::encode(&page) else {
            return;
        };
        let _ = SaiUpdater::replace_root(&cache_key, root, SaiUpdaterWriteScope::All, cx);
    }

    fn user_posts_mutation_keys(&self, cx: &mut App) -> HashSet<String> {
        let mut keys: HashSet<String> =
            self.known_user_posts_keys.union(&self.persistent_own_user_posts_keys).cloned().collect();
        if let Some(current_own_user_posts_cache_key) = Self::current_own_user_posts_cache_key(cx)
            && SaiWsResponseCache::current_payload(
                &current_own_user_posts_cache_key,
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::Persistent,
                cx,
            )
            .is_some()
        {
            keys.insert(current_own_user_posts_cache_key);
        }
        keys
    }

    fn transform_post_everywhere(&mut self, post_id: &str, transform: PostTransform<'_>, cx: &mut Context<Self>) {
        Self::transform_row_in_cache(&Self::feed_cache_key(), post_id, SaiUpdaterWriteScope::Cache, transform, cx);

        let own_key = Self::own_post_cache_key(post_id);
        if let Some(own_post) = Self::own_post_from_cache(&own_key, cx) {
            let keeps = !own_post.is_temp
                && !own_post.is_uploading
                && !own_post.is_compressing
                && PostMediaCachePolicy::is_own_post(&own_post, cx);
            match keeps.then(|| SaiJSONCodec::encode(&transform(own_post))).flatten() {
                Some(encoded) => {
                    let _ = SaiUpdater::replace_root(&own_key, encoded, SaiUpdaterWriteScope::All, cx);
                }
                None => SaiWsResponseCache::remove(&own_key, SaiCacheCleanLayer::Everywhere, cx),
            }
        }

        if let Some(selected) = self.selected_post.take() {
            self.selected_post = Some(if selected.id == post_id { transform(selected) } else { selected });
        }

        if let Some(index) = self.detail_context_posts.iter().position(|post| post.id == post_id) {
            let current = self.detail_context_posts[index].clone();
            self.detail_context_posts[index] = transform(current);
        }

        for cache_key in self.user_posts_mutation_keys(cx) {
            let write_to = if self.is_persistent_own_user_posts_key(&cache_key, cx) {
                SaiUpdaterWriteScope::All
            } else {
                SaiUpdaterWriteScope::Cache
            };
            Self::transform_row_in_cache(&cache_key, post_id, write_to, transform, cx);
        }

        let shared_transform = RefCell::new(transform);
        ShortsServicesStore::shared(cx).update(cx, |shorts, cx| {
            shorts.transform_short(post_id, |post| (*shared_transform.borrow_mut())(post), cx)
        });
        cx.notify();
    }

    pub fn patch_post_everywhere(
        &mut self,
        post: &GetPostFeedResponse,
        includes_media: bool,
        keeps_download_flags: bool,
        cx: &mut Context<Self>,
    ) {
        self.patch_post_switches_everywhere(post, includes_media, keeps_download_flags, false, cx);
    }

    /// like patch_post_everywhere but keeps_card_less_switches keeps every count and
    /// sparks switch the row already has and takes only download switches from post
    fn patch_post_switches_everywhere(
        &mut self,
        post: &GetPostFeedResponse,
        includes_media: bool,
        keeps_download_flags: bool,
        keeps_card_less_switches: bool,
        cx: &mut Context<Self>,
    ) {
        self.transform_post_everywhere(
            &post.id,
            &mut |existing| {
                let mut p = existing;
                let kept = p.count_visibility;
                p.title = post.title.clone();
                p.content = post.content.clone();
                p.original_content = post.original_content.clone();
                p.hashtags = post.hashtags.clone();
                p.tags = post.tags.clone();
                p.updated_at = post.updated_at.clone();
                p.can_comment = post.can_comment;
                p.count_visibility = post.count_visibility;
                if keeps_card_less_switches {
                    p.count_visibility = kept;
                    p.count_visibility.allows_download = post.count_visibility.allows_download;
                    p.count_visibility.allows_download_without_watermark =
                        post.count_visibility.allows_download_without_watermark;
                }
                if keeps_download_flags {
                    p.count_visibility.allows_download = kept.allows_download;
                    p.count_visibility.allows_download_without_watermark = kept.allows_download_without_watermark;
                }
                p.images = post.images.clone();
                p.video_thumbnail = post.video_thumbnail.clone();
                p.is_short = post.is_short;
                p.is_image_reel = post.is_image_reel;
                p.is_riff = post.is_riff;
                p.show_in_publications = post.show_in_publications;
                p.video_position = post.video_position;
                p.track_id = post.track_id.clone();
                p.track = post.track.clone();
                p.track_start_ms = post.track_start_ms;
                p.track_end_ms = post.track_end_ms;
                p.track_plate = post.track_plate.clone();
                p.audio_baked = post.audio_baked;
                if let Some(video) = post.video.as_ref().filter(|video| !video.is_empty()) {
                    p.video = Some(video.clone());
                }
                p.media_status = post.media_status.clone();
                if includes_media {
                    p.video = post.video.clone();
                    p.video_size = post.video_size.clone();
                    p.image_sizes = post.image_sizes.clone();
                    p.tagged_users = post.tagged_users.clone();
                    p.tagged_users_count = post.tagged_users_count;
                }
                p
            },
            cx,
        );
    }

    pub fn persist_post_mutation(&mut self, post_id: &str, cx: &mut Context<Self>) {
        self.transform_post_everywhere(post_id, &mut |post| post, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_post_edit_optimistic(
        &mut self,
        post_id: &str,
        title: &str,
        content: &str,
        manual_hashtags: &str,
        can_comment: bool,
        track_id: Option<String>,
        track: Option<RiseStoredMusicTrack>,
        track_start_ms: Option<i64>,
        track_end_ms: Option<i64>,
        track_plate: Option<PostTrackPlateDTO>,
        cx: &mut Context<Self>,
    ) -> Option<GetPostFeedResponse> {
        let previous = self.resolved_post(post_id, cx)?;

        let trimmed_title = trimming_whitespaces_and_newlines(title).to_owned();
        let trimmed_content = trimming_whitespaces_and_newlines(content).to_owned();
        let hashtags = PostHashtagParsing::merge(manual_hashtags, &format!("{trimmed_content} {trimmed_title}"));
        let mut updated = previous.clone();
        updated.title = trimmed_title;
        updated.content = trimmed_content.clone();
        updated.original_content = trimmed_content;
        updated.hashtags = hashtags;
        updated.can_comment = can_comment;
        updated.track_id = track_id;
        updated.track = track;
        updated.track_start_ms = track_start_ms;
        updated.track_end_ms = track_end_ms;
        updated.track_plate = track_plate;
        updated.updated_at = ISO8601DateFormatter::new().string_from(&Date::now());
        self.patch_post_everywhere(&updated, false, false, cx);
        Some(previous)
    }

    pub fn confirm_post_edit(&mut self, post_id: &str, payload: Option<&CreatePostGatewayDTO>, cx: &mut Context<Self>) {
        match payload.and_then(|dto| dto.post.as_ref()).and_then(GetFeedPostItemDTOExt::as_feed_response) {
            Some(post) => self.patch_post_everywhere(&post, false, false, cx),
            None => self.persist_post_mutation(post_id, cx),
        }
    }

    pub fn confirm_post_edit_post(&mut self, post: &GetPostFeedResponse, includes_media: bool, cx: &mut Context<Self>) {
        self.patch_post_everywhere(post, includes_media, false, cx);
    }

    pub fn apply_share_count(&mut self, post_id: &str, share_count: i64, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            post_id,
            &mut |mut updated| {
                updated.share_count = updated.share_count.max(share_count);
                updated
            },
            cx,
        );
    }

    pub fn apply_link_open_count(&mut self, post_id: &str, link_open_count: i64, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            post_id,
            &mut |mut updated| {
                updated.link_open_count = updated.link_open_count.max(link_open_count);
                updated
            },
            cx,
        );
    }

    pub fn restore_post_edit(&mut self, post: &GetPostFeedResponse, cx: &mut Context<Self>) {
        self.patch_post_everywhere(post, false, false, cx);
    }

    pub fn post_deleted_event_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::PostDeleted(event)) = RiseEvent::decode_payload(name::POST_DELETED, payload) else {
            return;
        };
        if event.post_id <= 0 {
            return;
        }
        self.apply_live_deletion(&event.post_id.to_string(), cx);
    }

    pub fn apply_live_deletion(&mut self, post_id: &str, cx: &mut Context<Self>) {
        self.live_deleted_post_ids.insert(post_id.to_owned());
        cx.notify();
        let Some(post) = self.resolved_post(post_id, cx) else {
            return;
        };
        let _ = self.apply_delete_optimistic(&post, cx);
    }

    pub fn post_updated_event_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::PostUpdated(event)) = RiseEvent::decode_payload(name::POST_UPDATED, payload) else {
            return;
        };
        let Ok(card) = serde_json::to_value(&event.post) else {
            return;
        };
        let Some(post) = AnyDictionaryBridge::dictionary(Some(&card)).and_then(|row| Self::feed_response(&row)) else {
            return;
        };
        self.patch_post_switches_everywhere(&post, true, event.post.allow_download.is_none(), true, cx);
    }

    pub fn apply_delete_optimistic(
        &mut self,
        post: &GetPostFeedResponse,
        cx: &mut Context<Self>,
    ) -> Option<PostDeleteOptimisticSnapshot> {
        let post_id = post.id.clone();
        let resolved_post = self.resolved_post(&post_id, cx).unwrap_or_else(|| post.clone());
        let (shorts_feed_post, shorts_feed_index, external_short_post) = ShortsServicesStore::shared(cx)
            .update(cx, |shorts, cx| shorts.remove_short_everywhere_snapshot(&post_id, cx));

        let mut candidate_keys = self.user_posts_mutation_keys(cx);
        candidate_keys.insert(Self::feed_cache_key());
        let mut placements = Vec::new();
        for cache_key in candidate_keys {
            let persists_to_disk = self.is_persistent_own_user_posts_key(&cache_key, cx);
            let policy =
                if persists_to_disk { SaiWsDiskCachePolicy::Persistent } else { SaiWsDiskCachePolicy::MemoryOnly };
            let Some(page) = Self::observed_cached_feed_page(&cache_key, policy, cx) else {
                continue;
            };
            let Some(index) = page.list.iter().position(|row| row.id == post_id) else {
                continue;
            };
            placements.push(PostDeleteCachePlacement { cache_key, index, persists_to_disk });
        }
        let counts_in_profile = PostMediaCachePolicy::is_own_post(&resolved_post, cx)
            && Self::counts_as_profile_publication(&resolved_post.content_kind, resolved_post.show_in_publications);
        let auth = AuthServicesStore::shared(cx);
        let profile_posts_count =
            if counts_in_profile { auth.read(cx).profile().map(|profile| profile.posts_count) } else { None };
        let snapshot = PostDeleteOptimisticSnapshot {
            post: resolved_post,
            placements,
            was_selected: self.selected_post.as_ref().is_some_and(|selected| selected.id == post_id),
            detail_context_index: self.detail_context_posts.iter().position(|row| row.id == post_id),
            profile_posts_count,
            shorts_feed_post,
            shorts_feed_index,
            external_short_post,
        };

        self.delete_post_success_handler(&post_id, cx);
        if let Some(profile_posts_count) = profile_posts_count
            && let Some(mut profile) = auth.read(cx).profile().cloned()
        {
            profile.posts_count = (profile_posts_count - 1).max(0);
            auth.update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
        }
        Some(snapshot)
    }

    pub fn restore_delete_optimistic(&mut self, snapshot: PostDeleteOptimisticSnapshot, cx: &mut Context<Self>) {
        for placement in &snapshot.placements {
            Self::restore_deleted_post(&snapshot.post, placement, cx);
        }

        if snapshot.was_selected {
            self.selected_post = Some(snapshot.post.clone());
        }
        if let Some(index) = snapshot.detail_context_index
            && !self.detail_context_posts.iter().any(|row| row.id == snapshot.post.id)
        {
            let at = index.min(self.detail_context_posts.len());
            self.detail_context_posts.insert(at, snapshot.post.clone());
        }
        let auth = AuthServicesStore::shared(cx);
        if let Some(profile_posts_count) = snapshot.profile_posts_count
            && let Some(mut profile) = auth.read(cx).profile().cloned()
        {
            profile.posts_count = profile_posts_count;
            auth.update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));
        }
        let PostDeleteOptimisticSnapshot { post, shorts_feed_post, shorts_feed_index, external_short_post, .. } =
            snapshot;
        ShortsServicesStore::shared(cx).update(cx, |shorts, cx| {
            shorts.restore_short_everywhere_snapshot(shorts_feed_post, shorts_feed_index, external_short_post, cx)
        });
        let _ = Self::cache_own_post_snapshot(&post, cx);
        cx.notify();
    }

    fn restore_deleted_post(post: &GetPostFeedResponse, placement: &PostDeleteCachePlacement, cx: &mut App) {
        let Some(encoded) = SaiJSONCodec::encode(post) else {
            return;
        };

        let write_to = if placement.persists_to_disk { SaiUpdaterWriteScope::All } else { SaiUpdaterWriteScope::Cache };
        let _ = SaiUpdater::update_json_value(
            &placement.cache_key,
            |context| {
                let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                let mut rows = AnyDictionaryBridge::array_of_dictionaries(root.get(key::LIST));
                if rows.iter().any(|row| Self::scalar_string(row.get(key::ID)).as_deref() == Some(post.id.as_str())) {
                    return Ok(Value::Object(root));
                }
                let at = placement.index.min(rows.len());
                rows.insert(at, encoded);
                root.insert(key::LIST.to_owned(), Value::Array(rows.into_iter().map(Value::Object).collect()));
                Ok(Value::Object(root))
            },
            write_to,
            cx,
        );
    }

    pub fn delete_post_success_handler(&mut self, post_id: &str, cx: &mut Context<Self>) {
        SaiWsResponseCache::remove(&Self::own_post_cache_key(post_id), SaiCacheCleanLayer::Everywhere, cx);

        let feed_key = Self::feed_cache_key();
        let _ = SaiUpdater::remove_row(&feed_key, key::LIST, key::ID, post_id, SaiUpdaterWriteScope::Cache, cx);

        for cache_key in self.user_posts_mutation_keys(cx) {
            let write_to = if self.is_persistent_own_user_posts_key(&cache_key, cx) {
                SaiUpdaterWriteScope::All
            } else {
                SaiUpdaterWriteScope::Cache
            };
            let _ = SaiUpdater::remove_row(&cache_key, key::LIST, key::ID, post_id, write_to, cx);
        }

        if self.selected_post.as_ref().is_some_and(|selected| selected.id == post_id) {
            self.selected_post = None;
        }
        self.detail_context_posts.retain(|row| row.id != post_id);
        ShortsServicesStore::shared(cx).update(cx, |shorts, cx| shorts.remove_short_everywhere(post_id, cx));
        cx.notify();
    }

    pub fn post_media_ready_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::PostMediaReady(event)) = RiseEvent::decode_payload(name::POST_MEDIA_READY, payload) else {
            return;
        };
        if event.post_id <= 0 {
            return;
        }
        let post_id = event.post_id.to_string();
        let video = RemoteMediaURL::optional_normalized(event.video.as_deref());
        let audio_baked = Some(event.audio_baked);
        let media_status = if event.media_status.is_empty() { "ready" } else { event.media_status.as_str() };
        let track = event
            .track
            .as_ref()
            .and_then(|track| AnyDictionaryBridge::dictionary(Some(track)))
            .and_then(|track| SaiJSONCodec::decode::<RiseStoredMusicTrack>(Some(&Value::Object(track))));

        self.apply_media_ready(&post_id, video, track, audio_baked, media_status, cx);
    }

    pub fn apply_media_ready(
        &mut self,
        post_id: &str,
        video: Option<String>,
        track: Option<RiseStoredMusicTrack>,
        audio_baked: Option<bool>,
        media_status: &str,
        cx: &mut Context<Self>,
    ) {
        if post_id.is_empty() {
            return;
        }
        self.transform_post_everywhere(
            post_id,
            &mut |existing| {
                let mut p = existing;
                if let Some(video) = video.as_ref().filter(|video| !video.is_empty()) {
                    p.video = Some(video.clone());
                }
                if let Some(track) = track.as_ref() {
                    p.track = Some(track.clone());
                    if let Some(tid) = track.id.clone() {
                        p.track_id = Some(tid);
                    }
                }
                if let Some(audio_baked) = audio_baked {
                    p.audio_baked = audio_baked;
                }
                p.media_status = media_status.to_owned();
                p
            },
            cx,
        );
    }

    pub fn apply_moderation_status(&mut self, post_id: &str, status: &str, cx: &mut Context<Self>) {
        let Some(wire) = PostModerationStatus::from_wire(status).map(PostModerationStatus::wire) else {
            return;
        };
        if let Some(domain) = RisePostDomainRegistry::shared(cx).read(cx).active().cloned() {
            let task_post_id = post_id.to_owned();
            cx.spawn(async move |_, _| {
                let _ = domain.apply_post_moderation_status(&task_post_id, wire).await;
            })
            .detach();
        }
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                p.moderation_status = wire.to_owned();
                p
            },
            cx,
        );
    }

    pub fn sync_like_acknowledged(&mut self, post_id: &str, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                p.old_is_liked = p.is_liked;
                p
            },
            cx,
        );
    }

    pub fn sync_favorite_acknowledged(&mut self, post_id: &str, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                p.old_is_favorited = p.is_favorited;
                p
            },
            cx,
        );
    }

    pub fn apply_like_optimistic(&mut self, post_id: &str, cx: &mut Context<Self>) -> Option<bool> {
        let mut captured: Option<bool> = None;
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                let was = Self::flip_like_in_place(&mut p);
                if captured.is_none() {
                    captured = Some(was);
                }
                p
            },
            cx,
        );
        captured
    }

    pub fn apply_favorite_optimistic(&mut self, post_id: &str, cx: &mut Context<Self>) -> Option<bool> {
        let mut captured: Option<bool> = None;
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                let was = Self::flip_favorite_in_place(&mut p);
                if captured.is_none() {
                    captured = Some(was);
                }
                p
            },
            cx,
        );
        captured
    }

    pub fn adjust_comments_count(&mut self, post_id: &str, delta: i64, cx: &mut Context<Self>) {
        self.transform_post_everywhere(
            post_id,
            &mut |mut p| {
                p.comments_count = (p.comments_count + delta).max(0);
                p
            },
            cx,
        );
        if self.selected_post.as_ref().is_some_and(|selected| selected.id == post_id) {
            self.comments_post_taken_at_milliseconds = self
                .comments_post_taken_at_milliseconds
                .max((Date::now().time_interval_since_1970() * 1000.0) as i64 + 1);
        }
        PostActionsStore::shared(cx).update(cx, |actions, cx| actions.adjust_post_comments_count_action(post_id, delta, cx));
    }

    pub fn toggle_like_success_handler(&mut self, post_id: &str, cx: &mut Context<Self>) {
        self.sync_like_acknowledged(post_id, cx);
    }

    pub fn revert_toggle_like(&mut self, post_id: &str, error: &anyhow::Error, cx: &mut Context<Self>) {
        let message = format!("toggle_like error postId={post_id}: {error}");
        Sai::log("Post", move || message, SaiLogKind::Network, cx);
        let _ = self.apply_like_optimistic(post_id, cx);
    }

    pub fn toggle_favorite_success_handler(&mut self, post_id: &str, cx: &mut Context<Self>) {
        self.sync_favorite_acknowledged(post_id, cx);
    }

    pub fn revert_toggle_favorite(&mut self, post_id: &str, error: &anyhow::Error, cx: &mut Context<Self>) {
        let message = format!("toggle_favorite error postId={post_id}: {error}");
        Sai::log("Post", move || message, SaiLogKind::Network, cx);
        let _ = self.apply_favorite_optimistic(post_id, cx);
    }
}
