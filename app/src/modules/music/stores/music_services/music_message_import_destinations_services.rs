use gpui::App;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::music::stores::music_services::music_guide_sandbox_hooks::MusicGuideSandboxHooks;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::MusicLocalPlaylistDestination;
use crate::modules::music::stores::music_actions::music_wire_types::{
    WireLocalPlaylistDestinationDTOExt, WireLocalPlaylistDestinationsDTO,
};
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

impl MusicServicesStore {
    pub fn local_playlist_destinations_cache_key() -> String {
        SaiCache::new(["listLocalPlaylistDestinations"]).key
    }

    fn local_playlist_destinations_dto(&self, cx: &mut App) -> Option<WireLocalPlaylistDestinationsDTO> {
        let payload = SaiWsResponseCache::observed_payload(
            &Self::local_playlist_destinations_cache_key(),
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        let real = SaiJSONCodec::decode::<WireLocalPlaylistDestinationsDTO>(payload.as_ref()).or_else(|| {
            MusicActionsStore::slices(cx)
                .list_local_playlist_destinations_sai
                .data()
        });
        let Some(staged) = MusicGuideSandboxHooks::playlist_destination(cx) else {
            return real;
        };
        let staged_id = staged.id.clone();
        let mut playlists = vec![staged];
        playlists.extend(
            real.map(|real| real.playlists)
                .unwrap_or_default()
                .into_iter()
                .filter(|playlist| playlist.id != staged_id),
        );
        Some(WireLocalPlaylistDestinationsDTO {
            has_more: false,
            playlists,
            ..WireLocalPlaylistDestinationsDTO::default()
        })
    }

    pub fn local_playlist_destinations(&self, cx: &mut App) -> Vec<MusicLocalPlaylistDestination> {
        self.local_playlist_destinations_dto(cx)
            .map(|dto| dto.playlists)
            .unwrap_or_default()
            .iter()
            .filter_map(|playlist| playlist.as_domain())
            .filter(|destination| destination.can_add_tracks)
            .collect()
    }

    pub fn local_playlist_destinations_has_data(&self, cx: &mut App) -> bool {
        self.local_playlist_destinations_dto(cx).is_some()
    }

    pub fn local_playlist_destinations_loading(&self, cx: &mut App) -> bool {
        !self.local_playlist_destinations_has_data(cx)
            && MusicActionsStore::slices(cx)
                .list_local_playlist_destinations_sai
                .is_pending()
    }

    pub fn local_playlist_destinations_loading_more(&self, cx: &mut App) -> bool {
        if !self.local_playlist_destinations_has_data(cx) {
            return false;
        }
        if MusicActionsStore::slices(cx)
            .list_local_playlist_destinations_sai
            .is_pending()
        {
            return true;
        }
        SaiWs::pagination_state(&Self::local_playlist_destinations_cache_key(), cx)
            .read(cx)
            .trailing()
            .is_loading
    }

    pub fn local_playlist_destinations_page_failed(&self, cx: &mut App) -> bool {
        if self.local_playlist_destinations_error_text(cx).is_some() {
            return true;
        }
        SaiWs::pagination_state(&Self::local_playlist_destinations_cache_key(), cx)
            .read(cx)
            .trailing()
            .error
            .is_some()
    }

    pub fn local_playlist_destinations_error_text(&self, cx: &mut App) -> Option<String> {
        MusicActionsStore::slices(cx)
            .list_local_playlist_destinations_sai
            .error()
    }

    pub fn local_playlist_destinations_next_cursor(&self, cx: &mut App) -> Option<String> {
        let cursor = self
            .local_playlist_destinations_dto(cx)?
            .next_cursor
            .map(|cursor| trimming_whitespaces_and_newlines(&cursor).to_owned())?;
        (!cursor.is_empty()).then_some(cursor)
    }

    pub fn local_playlist_destinations_has_more(&self, cx: &mut App) -> bool {
        let page = self.local_playlist_destinations_dto(cx);
        page.is_some_and(|page| page.has_more) && self.local_playlist_destinations_next_cursor(cx).is_some()
    }
}
