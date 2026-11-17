use std::rc::Rc;

use gpui::App;
use rise_canon::rise_key::music as music_key;
use rise_canon::rise_request::music::ListLocalPlaylistDestinations;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use super::music_actions::MusicActionsStore;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationDuplicatePolicy, SaiWsPaginationOptions, SaiWsPaginationRelativeIdArgs,
    SaiWsPaginationStateStore,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsPendingCacheBinding, SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

impl MusicActionsStore {
    pub fn list_local_playlist_destinations_action(
        &self,
        limit: i64,
        cursor: Option<&str>,
        reflect_pending_in_ui: bool,
        reload_if_cached: bool,
        cx: &mut App,
    ) {
        let has_user = AuthServicesStore::shared(cx)
            .read(cx)
            .effective_user_id_for_wire()
            .is_some_and(|user_id| !user_id.is_empty());
        if !has_user {
            return;
        }

        let mut body = ListLocalPlaylistDestinations {
            limit: Some(limit.clamp(1, 100)),
            ..ListLocalPlaylistDestinations::default()
        };
        if let Some(cursor) = cursor.map(trimming_whitespaces_and_newlines).filter(|cursor| !cursor.is_empty()) {
            body.cursor = Some(cursor.to_owned());
        }

        let cache_key = MusicServicesStore::local_playlist_destinations_cache_key();
        let is_continuation = body.cursor.is_some();
        let mut options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_canon::rise_wire_methods::music::SERVICE.to_owned(),
            method: rise_canon::rise_wire_methods::music::LIST_LOCAL_PLAYLIST_DESTINATIONS.to_owned(),
            cache_key: Some(cache_key),
            reload_if_cached,
            allow_concurrent_while_pending: false,
            reflect_pending_in_ui: cursor.is_none() && reflect_pending_in_ui,
            send_immediately: true,
            store_response_in_cache: true,
            persist_cache_to_disk: true,
            reload_when_only_disk_cached: false,
            preferred_cache_layer: SaiWsCachePreferredLayer::DiskFirst,
            path_to_array: Some(music_key::PLAYLISTS.to_owned()),
            path_to_id: Some(music_key::ID.to_owned()),
            pagination: Some(SaiWsPaginationOptions::relative_id(SaiWsPaginationRelativeIdArgs {
                relative_id: cursor.map(str::to_owned),
                response_relative_id_path: music_key::camel::NEXT_CURSOR.to_owned(),
                has_more_path: Some(music_key::camel::HAS_MORE.to_owned()),
                duplicate_policy: SaiWsPaginationDuplicatePolicy::UseIncoming,
                ..SaiWsPaginationRelativeIdArgs::default()
            })),
            ..SaiWsRequestOptionsInit::default()
        });
        options.allows_deferred_send = !is_continuation;
        let failure_options = options.clone();

        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            if is_continuation {
                SaiWsPaginationStateStore::fail(Some(&SaiWsPendingCacheBinding::new(&failure_options)), error, cx);
            }
            Sai::log(
                "Music",
                || format!("listLocalPlaylistDestinations error: {error}"),
                SaiLogKind::Network,
                cx,
            );
        });

        SaiWs::request_slice(
            &self.list_local_playlist_destinations_sai,
            body,
            options,
            None,
            Some(on_error),
            cx,
        );
    }
}
