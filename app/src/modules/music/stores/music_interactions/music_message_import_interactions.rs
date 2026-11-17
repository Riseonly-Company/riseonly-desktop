use gpui::Context;

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_message_import_types::MusicMessageMP3ImportRequest;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

impl MusicInteractionsStore {
    pub fn enqueue_message_mp3_import(&mut self, request: MusicMessageMP3ImportRequest, cx: &mut Context<Self>) {
        let sandbox = GuideSandbox::shared(cx);
        if sandbox.read(cx).is_armed() {
            sandbox.update(cx, |sandbox, cx| sandbox.add_track_to_playlist(cx));
            return;
        }
        MusicServicesStore::shared(cx).update(cx, |service, cx| service.enqueue_message_mp3_import(request, cx));
    }

    pub fn retry_message_mp3_import(&mut self, track_id: &str, cx: &mut Context<Self>) {
        MusicServicesStore::shared(cx).update(cx, |service, cx| service.retry_message_mp3_import(track_id, cx));
    }

    pub fn load_local_playlist_destinations_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = MusicActionsStore::shared(cx);
        let slice = actions.read(cx).list_local_playlist_destinations_sai.clone();
        if slice.is_pending() {
            return;
        }

        let cache_key = MusicServicesStore::local_playlist_destinations_cache_key();
        let has_data = MusicServicesStore::shared(cx).update(cx, |service, cx| {
            service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx);
            service.local_playlist_destinations_has_data(cx)
        });
        actions.update(cx, |actions, cx| {
            actions.list_local_playlist_destinations_action(30, None, !has_data, true, cx);
        });
    }

    pub fn refresh_local_playlist_destinations(&mut self, cx: &mut Context<Self>) {
        let actions = MusicActionsStore::shared(cx);
        if actions.read(cx).list_local_playlist_destinations_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| {
            actions.list_local_playlist_destinations_action(30, None, false, true, cx);
        });
    }

    pub fn load_more_local_playlist_destinations(&mut self, cx: &mut Context<Self>) {
        let actions = MusicActionsStore::shared(cx);
        if actions.read(cx).list_local_playlist_destinations_sai.is_pending() {
            return;
        }
        let service = MusicServicesStore::shared(cx);
        if !service.update(cx, |service, cx| service.local_playlist_destinations_has_more(cx)) {
            return;
        }
        let Some(cursor) = service.update(cx, |service, cx| service.local_playlist_destinations_next_cursor(cx)) else {
            return;
        };
        let cache_key = MusicServicesStore::local_playlist_destinations_cache_key();
        if !SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().can_request_automatically() {
            return;
        }

        actions.update(cx, |actions, cx| {
            actions.list_local_playlist_destinations_action(30, Some(&cursor), false, true, cx);
        });
    }

    pub fn retry_load_more_local_playlist_destinations(&mut self, cx: &mut Context<Self>) {
        let actions = MusicActionsStore::shared(cx);
        if actions.read(cx).list_local_playlist_destinations_sai.is_pending() {
            return;
        }
        let service = MusicServicesStore::shared(cx);
        if !service.update(cx, |service, cx| service.local_playlist_destinations_has_more(cx)) {
            return;
        }
        let Some(cursor) = service.update(cx, |service, cx| service.local_playlist_destinations_next_cursor(cx)) else {
            return;
        };

        let cache_key = MusicServicesStore::local_playlist_destinations_cache_key();
        SaiWs::pagination_state(&cache_key, cx)
            .update(cx, |state, cx| state.clear_error(SaiWsPaginationEdge::Trailing, cx));
        actions.update(cx, |actions, cx| {
            actions.list_local_playlist_destinations_action(30, Some(&cursor), false, true, cx);
        });
    }
}
