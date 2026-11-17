use std::any::Any;

use gpui::{App, Context};
use rise_canon::rise_key::post as post_keys;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsDiskCachePolicy;
use crate::core::lib::sai::sai::Sai;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_services::post_guide_sandbox_hooks::PostGuideSandboxHooks;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

#[derive(Clone, Debug, PartialEq)]
pub struct PostServicesStoreAudioPostsState {
    pub items: Vec<GetPostFeedResponse>,
    pub has_more: bool,
    pub relative_id: Option<String>,
    pub last_error_text: Option<String>,
    pub did_load: bool,
    pub total_count: i64,
}

impl Default for PostServicesStoreAudioPostsState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            has_more: true,
            relative_id: None,
            last_error_text: None,
            did_load: false,
            total_count: 0,
        }
    }
}

impl PostServicesStore {
    pub fn audio_posts_cache_key(track_id: &str) -> String {
        SaiCache::new(["getPostsByTrack", track_id]).key
    }

    pub fn audio_posts(&self, track_id: &str, cx: &mut App) -> PostServicesStoreAudioPostsState {
        if let Some(invented) = PostGuideSandboxHooks::sound_posts(track_id, cx) {
            return invented;
        }
        let key = Self::audio_posts_cache_key(track_id);
        let last_error_text = self.audio_posts_error_by_track.get(track_id).cloned();
        let Some(page) = self.cached_feed_page(&key, SaiWsDiskCachePolicy::MemoryOnly, cx) else {
            return PostServicesStoreAudioPostsState {
                last_error_text,
                did_load: false,
                ..PostServicesStoreAudioPostsState::default()
            };
        };

        let loaded_count = i64::try_from(page.list.len()).unwrap_or(i64::MAX);
        let total_count = page.total.unwrap_or(0).max(loaded_count);
        PostServicesStoreAudioPostsState {
            items: page.list,
            has_more: page.is_have_more.unwrap_or(false),
            relative_id: page.relative_id,
            last_error_text,
            did_load: true,
            total_count,
        }
    }

    pub fn mark_audio_posts_pending(&mut self, track_id: &str, cx: &mut Context<Self>) {
        if track_id.is_empty() {
            return;
        }
        self.audio_posts_pending_tracks.insert(track_id.to_owned());
        cx.notify();
    }

    pub fn is_audio_posts_pending(&self, track_id: &str) -> bool {
        self.audio_posts_pending_tracks.contains(track_id)
    }

    pub fn audio_posts_status(&self, track_id: &str, cx: &mut App) -> SaiWsSliceStatus {
        if track_id.is_empty() {
            return SaiWsSliceStatus::Idle;
        }
        if self.audio_posts(track_id, cx).did_load {
            return SaiWsSliceStatus::Fulfilled;
        }
        if self.audio_posts_pending_tracks.contains(track_id) {
            return SaiWsSliceStatus::Pending;
        }
        if self.audio_posts_error_by_track.contains_key(track_id) {
            return SaiWsSliceStatus::Rejected;
        }
        SaiWsSliceStatus::Idle
    }

    pub fn audio_posts_first_track_wire(&self, track_id: &str, cx: &mut App) -> Option<RiseStoredMusicTrack> {
        self.audio_posts(track_id, cx)
            .items
            .into_iter()
            .find(|post| post.track.as_ref().and_then(|track| track.id.as_deref()).unwrap_or("") == track_id)
            .and_then(|post| post.track)
    }

    pub fn apply_created_audio_post(
        &mut self,
        post: &GetPostFeedResponse,
        fallback_track: Option<&RiseStoredMusicTrack>,
        cx: &mut Context<Self>,
    ) {
        let track_id = trimming_whitespaces_and_newlines(post.track_id.as_deref().unwrap_or(""));
        if track_id.is_empty() || post.id.is_empty() || post.is_temp {
            return;
        }

        let key = Self::audio_posts_cache_key(track_id);
        if self.cached_feed_page(&key, SaiWsDiskCachePolicy::MemoryOnly, cx).is_none() {
            return;
        }

        let mut row = post.clone();
        if row.track.is_none() {
            row.track = fallback_track.cloned();
        }
        let Some(encoded) = SaiJSONCodec::encode(&row) else {
            return;
        };

        let _ = SaiUpdater::remove_row(
            &key,
            post_keys::LIST,
            post_keys::ID,
            &row.id,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
        let _ = SaiUpdater::insert_row_if_missing(
            &key,
            post_keys::LIST,
            post_keys::ID,
            encoded,
            true,
            SaiUpdaterWriteScope::Cache,
            cx,
        );
    }

    pub fn get_posts_by_track_success_handler(
        &mut self,
        track_id: &str,
        payload: Option<Box<dyn Any>>,
        cx: &mut Context<Self>,
    ) {
        self.audio_posts_pending_tracks.remove(track_id);
        self.audio_posts_error_by_track.remove(track_id);
        let key = Self::audio_posts_cache_key(track_id);
        self.known_audio_posts_keys.insert(key);
        cx.notify();
        self.cache_own_posts_from(payload.as_deref(), cx);
    }

    pub fn get_posts_by_track_error_handler(&mut self, track_id: &str, error: &anyhow::Error, cx: &mut Context<Self>) {
        self.audio_posts_pending_tracks.remove(track_id);
        let description = error.to_string();
        self.audio_posts_error_by_track.insert(track_id.to_owned(), description.clone());
        cx.notify();
        let track_id = track_id.to_owned();
        Sai::log(
            "Post",
            move || format!("get_posts_by_track track={track_id} error: {description}"),
            SaiLogKind::Network,
            cx,
        );
    }
}
