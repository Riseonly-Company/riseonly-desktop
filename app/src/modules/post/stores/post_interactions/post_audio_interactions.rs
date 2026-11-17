use gpui::{App, Context};

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

impl PostInteractionsStore {
    pub fn load_audio_posts_if_needed(&mut self, track_id: &str, cx: &mut Context<Self>) {
        if track_id.is_empty() {
            return;
        }

        let services = PostServicesStore::shared(cx);
        let state = services.update(cx, |services, cx| services.audio_posts(track_id, cx));

        if state.did_load {
            return;
        }

        if services.read(cx).is_audio_posts_pending(track_id) {
            return;
        }

        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_posts_by_track_action(track_id, None, true, true, cx);
        });
    }

    pub fn load_more_audio_posts(&mut self, track_id: &str, cx: &mut Context<Self>) {
        if track_id.is_empty() {
            return;
        }

        let services = PostServicesStore::shared(cx);
        let state = services.update(cx, |services, cx| services.audio_posts(track_id, cx));

        let Some(anchor) = state.relative_id.filter(|anchor| state.has_more && !anchor.is_empty()) else {
            return;
        };

        let pagination = SaiWs::pagination_state(&PostServicesStore::audio_posts_cache_key(track_id), cx);
        if !pagination.read(cx).trailing().can_request_automatically() {
            return;
        }

        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.get_posts_by_track_action(track_id, Some(&anchor), false, true, cx);
        });
    }

    pub fn audio_posts_items(&self, track_id: &str, cx: &mut App) -> Vec<GetPostFeedResponse> {
        let services = PostServicesStore::shared(cx);
        services.update(cx, |services, cx| services.audio_posts(track_id, cx).items)
    }

    pub fn audio_posts_status(&self, track_id: &str, cx: &mut App) -> SaiWsSliceStatus {
        let services = PostServicesStore::shared(cx);
        services.update(cx, |services, cx| services.audio_posts_status(track_id, cx))
    }
}
