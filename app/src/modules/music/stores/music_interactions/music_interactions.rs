use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, Context, Entity, Task};
use rise_canon::RiseGuideSignal;
use rise_canon::rise_key::music as key;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_text::{trimming_whitespaces, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{SaiSliceState, SaiTypedSlice};
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;
use crate::core::push_notifications::universal_link_parser::RiseonlyUniversalLink;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::shared::music_field_rules::MusicFieldRules;
use crate::modules::music::shared::utils::music::featured_profile_track_store::{
    FeaturedProfileTrackAddResult, FeaturedProfileTrackStore,
};
use crate::modules::music::shared::utils::music::music_audio_effect_settings::MusicAudioEffectKind;
use crate::modules::music::shared::utils::music::music_liked_tracks_queue_continuation::MusicLikedTracksQueueContinuation;
use crate::modules::music::shared::utils::music::music_local_state::MusicLocalState;
use crate::modules::music::shared::utils::music::music_local_folder_import::MusicLocalFolderImport;
use crate::modules::music::shared::utils::music::music_local_track_draft_store::{
    MusicLocalTrackDraftCreationObserver, MusicLocalTrackDraftStore,
};
use crate::modules::music::shared::utils::music::music_local_tracks_queue_continuation::MusicLocalTracksQueueContinuation;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::{MusicPlayOptions, MusicPlayerState};
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::{
    MusicListeningAudioEffect, MusicListeningParticipantPermissions, MusicListeningSessionCommand,
    MusicPlaylistJoinRequest, MusicPlaylistListeningSession, MusicPlaylistMember, MusicPlaylistMemberPermissions,
    MusicPlaylistTrackLinkError, MusicResolvedPlaylistTrackLink, MusicTrackLinkError,
};
use crate::modules::music::stores::music_actions::music_wire_types::{
    RiseStoredMusicPlaylist, RiseStoredMusicTrack, WireAlbumDTOExt,
};
use crate::modules::music::stores::music_services::music_guide_sandbox_hooks::{
    MusicGuideSandboxHooks, MusicGuideSandboxReaderTrack, MusicGuideSandboxUpdateSessionState,
};
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

/// the twelve values the session state update takes
/// kept together so the player builds one request per publish
#[derive(Clone, Debug)]
pub struct MusicPlaylistListeningSessionStateRequest {
    pub session_id: String,
    pub command: MusicListeningSessionCommand,
    pub track_id: String,
    pub position_ms: i64,
    pub is_playing: bool,
    pub shuffle_enabled: bool,
    pub shuffle_seed: i64,
    pub repeat_mode: String,
    pub audio_effect_enabled: bool,
    pub audio_effect_rate_milli: i64,
    pub audio_effects: Vec<MusicListeningAudioEffect>,
    pub expected_revision: i64,
}

pub struct MusicInteractionsStore {
    bootstrapped_user_id: Option<String>,
    local_track_delete_request_ids: HashMap<String, String>,
    artists_paged_past_first_page: bool,
    artist_tracks_artist_id: Option<String>,
    artist_tracks_paged_past_first_page: bool,
    album_playback_request_id: Option<String>,
    expanded_search_artists_query: Option<String>,
    playlist_moves: Vec<PlaylistMove>,
    is_sending_playlist_move: bool,
    playlist_move_revisions: HashMap<String, i64>,
}

struct PlaylistMove {
    playlist_id: String,
    track_id: String,
    after_track_id: Option<String>,
    previous: Option<RiseStoredMusicPlaylist>,
    on_error: Rc<dyn Fn(&mut App)>,
}

impl MusicInteractionsStore {
    pub const SEARCH_ARTISTS_PAGE_SIZE: i64 = 20;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            bootstrapped_user_id: None,
            local_track_delete_request_ids: HashMap::new(),
            artists_paged_past_first_page: false,
            artist_tracks_artist_id: None,
            artist_tracks_paged_past_first_page: false,
            album_playback_request_id: None,
            expanded_search_artists_query: None,
            playlist_moves: Vec::new(),
            is_sending_playlist_move: false,
            playlist_move_revisions: HashMap::new(),
        }
    }

    fn actions(cx: &mut App) -> Entity<MusicActionsStore> {
        MusicActionsStore::shared(cx)
    }

    fn service(cx: &mut App) -> Entity<MusicServicesStore> {
        MusicServicesStore::shared(cx)
    }

    fn has_wire_user(cx: &mut App) -> bool {
        AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().is_some_and(|user_id| !user_id.is_empty())
    }

    pub fn load_sound_track(&mut self, track_id: &str, force: bool, cx: &mut Context<Self>) -> Task<()> {
        if track_id.is_empty() {
            return Task::ready(());
        }
        if MusicGuideSandboxHooks::owns_sound(track_id, cx) {
            return Task::ready(());
        }
        let track_id = track_id.to_owned();
        Self::actions(cx).update(cx, |actions, cx| actions.load_sound_track_action(track_id, force, cx))
    }

    pub fn bootstrap_music_tab(&mut self, cx: &mut Context<Self>) {
        let user_id =
            AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().filter(|user_id| !user_id.is_empty());
        let Some(user_id) = user_id else {
            self.bootstrapped_user_id = None;
            self.local_track_delete_request_ids.clear();
            self.artists_paged_past_first_page = false;
            return;
        };
        if self.bootstrapped_user_id.as_deref() == Some(user_id.as_str()) {
            self.refresh_music_tab_data(cx);
            return;
        }
        self.local_track_delete_request_ids.clear();
        self.artists_paged_past_first_page = false;
        self.bootstrapped_user_id = Some(user_id);
        self.refresh_music_tab_data(cx);
    }

    pub fn refresh_music_tab_data(&mut self, cx: &mut Context<Self>) {
        self.load_tracks_if_needed(None, None, None, cx);
        self.load_artists_if_needed(cx);
        self.load_playlists_if_needed(cx);
        self.load_recent_if_needed(cx);
        self.load_liked_if_needed(cx);
        self.load_my_local_tracks_if_needed(cx);
    }

    fn reflect_pending_in_ui<T: Clone>(slice: &SaiTypedSlice<T>) -> bool {
        slice.data().is_none()
    }

    fn continuation_cursor(cache_key: &str, cx: &mut App) -> Option<String> {
        let page = Self::service(cx).update(cx, |service, cx| service.pagination_state(cache_key, cx));
        if !page.has_more {
            return None;
        }
        let cursor = page.cursor?;
        let can_request = SaiWs::pagination_state(cache_key, cx)
            .read(cx)
            .state(SaiWsPaginationEdge::Trailing)
            .can_request_automatically();
        can_request.then_some(cursor)
    }

    pub fn load_tracks_if_needed(
        &mut self,
        genre: Option<&str>,
        artist_id: Option<&str>,
        album_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).list_tracks_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = MusicServicesStore::tracks_cache_key(genre, artist_id, album_id);
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| {
            actions.list_tracks_action(None, genre, artist_id, album_id, None, reflect_pending_in_ui, true, cx);
        });
    }

    pub fn refresh_tracks(
        &mut self,
        genre: Option<&str>,
        artist_id: Option<&str>,
        album_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.list_tracks_action(None, genre, artist_id, album_id, None, false, true, cx);
        });
    }

    pub fn load_more_tracks(
        &mut self,
        genre: Option<&str>,
        artist_id: Option<&str>,
        album_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let cache_key = MusicServicesStore::tracks_cache_key(genre, artist_id, album_id);
        let Some(cursor) = Self::continuation_cursor(&cache_key, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.list_tracks_action(None, genre, artist_id, album_id, Some(&cursor), false, true, cx);
        });
    }

    pub fn load_liked_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_liked_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = MusicServicesStore::liked_cache_key();
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.get_liked_action(None, reflect_pending_in_ui, true, cx));
    }

    pub fn refresh_liked(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_liked_action(None, false, true, cx));
    }

    pub fn load_more_liked(&mut self, cx: &mut Context<Self>) {
        let Some(cursor) = Self::continuation_cursor(&MusicServicesStore::liked_cache_key(), cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| actions.get_liked_action(Some(&cursor), false, true, cx));
    }

    pub fn load_recent_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_recent_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = SaiCache::new(["getRecent"]).key;
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.get_recent_action(reflect_pending_in_ui, true, cx));
    }

    pub fn refresh_recent(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_recent_action(false, true, cx));
    }

    pub fn load_artists_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).list_artists_sai);
        if slice.is_pending() {
            return;
        }
        if self.artists_paged_past_first_page && slice.data().is_some() {
            return;
        }
        let cache_key = MusicServicesStore::artists_cache_key();
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.list_artists_action(None, reflect_pending_in_ui, true, cx));
    }

    pub fn refresh_artists(&mut self, cx: &mut Context<Self>) {
        self.artists_paged_past_first_page = false;
        Self::actions(cx).update(cx, |actions, cx| actions.list_artists_action(None, false, true, cx));
    }

    pub fn load_more_artists(&mut self, cx: &mut Context<Self>) {
        let Some(cursor) = Self::continuation_cursor(&MusicServicesStore::artists_cache_key(), cx) else {
            return;
        };
        self.artists_paged_past_first_page = true;
        Self::actions(cx).update(cx, |actions, cx| actions.list_artists_action(Some(&cursor), false, true, cx));
    }

    pub fn open_artist(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        self.load_artist_tracks_if_needed(artist_id, cx);
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_artist_sai);
        if slice.is_pending() {
            return;
        }
        let has_data = slice.data().and_then(|data| data.artist).is_some_and(|artist| artist.id == artist_id);
        if has_data {
            actions.update(cx, |actions, cx| actions.get_artist_action(artist_id, false, cx));
            return;
        }
        let cache_key = SaiCache::new(["getArtist", artist_id]).key;
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.get_artist_action(artist_id, reflect_pending_in_ui, cx));
    }

    pub fn open_now_playing_artist(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).now_playing_artist_sai);
        let has_data = slice.data().and_then(|data| data.artist).is_some_and(|artist| artist.id == artist_id);
        if !has_data {
            let cache_key = SaiCache::new(["getArtist", artist_id]).key;
            Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        }
        actions.update(cx, |actions, cx| actions.get_now_playing_artist_action(artist_id, cx));
    }

    pub fn load_artist_tracks_if_needed(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).list_artist_tracks_sai);
        let same_artist = self.artist_tracks_artist_id.as_deref() == Some(artist_id);
        if same_artist && slice.is_pending() {
            return;
        }
        if same_artist && self.artist_tracks_paged_past_first_page && slice.data().is_some() {
            return;
        }
        self.artist_tracks_artist_id = Some(artist_id.to_owned());
        self.artist_tracks_paged_past_first_page = false;
        let cache_key = MusicServicesStore::tracks_cache_key(None, Some(artist_id), None);
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| {
            actions.list_tracks_action(Some(slice), None, Some(artist_id), None, None, reflect_pending_in_ui, true, cx);
        });
    }

    pub fn load_more_artist_tracks(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        let cache_key = MusicServicesStore::tracks_cache_key(None, Some(artist_id), None);
        let Some(cursor) = Self::continuation_cursor(&cache_key, cx) else {
            return;
        };
        if self.artist_tracks_artist_id.as_deref() == Some(artist_id) {
            self.artist_tracks_paged_past_first_page = true;
        }
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).list_artist_tracks_sai);
        actions.update(cx, |actions, cx| {
            actions.list_tracks_action(Some(slice), None, Some(artist_id), None, Some(&cursor), false, true, cx);
        });
    }

    pub fn refresh_artist(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).get_artist_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_artist_action(artist_id, false, cx));
    }

    pub fn toggle_like(&mut self, track_id: &str, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.apply_optimistic_toggle_like(track_id, cx));
        let debounced_track_id = track_id.to_owned();
        SaiDebouncer::debounced_action_delay_milliseconds(
            track_id,
            Box::new(move |cx: &mut App| {
                let liked = Self::service(cx).update(cx, |service, cx| service.is_track_liked(&debounced_track_id, cx));
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.toggle_track_like_action(&debounced_track_id, liked, cx);
                });
            }),
            600,
            "music_toggle_like",
            100,
            None,
            cx,
        );
    }

    pub fn is_liked(&self, track: &TrackModel, cx: &mut App) -> bool {
        Self::service(cx).update(cx, |service, cx| service.is_track_liked_track(track, cx))
    }

    pub fn toggle_artist_follow(&mut self, artist_id: &str, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.apply_optimistic_toggle_artist_follow(artist_id, cx));
        let debounced_artist_id = artist_id.to_owned();
        SaiDebouncer::debounced_action_delay_milliseconds(
            artist_id,
            Box::new(move |cx: &mut App| {
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.toggle_artist_follow_action(&debounced_artist_id, cx);
                });
            }),
            600,
            "music_toggle_follow",
            50,
            None,
            cx,
        );
    }

    pub fn play(
        &mut self,
        track: TrackModel,
        queue: Vec<TrackModel>,
        scope_id: Option<&str>,
        record_play: bool,
        on_playback_accepted: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let audio_url = if track.audio_url.is_empty() { "(empty)".to_owned() } else { track.audio_url.clone() };
        let message = format!("Interactions.play id={} \"{}\" audioUrl={}", track.id, track.title, audio_url);
        Sai::log("Music", move || message, SaiLogKind::Custom("player".to_owned()), cx);
        let counts_play = record_play && !MusicGuideSandboxHooks::owns_track_id(&track.id, cx);
        let accepted: Option<Box<dyn FnOnce(&mut App)>> = if scope_id == Some(MusicLikedTracksQueueContinuation::SCOPE_ID)
        {
            Some(Box::new(move |cx: &mut App| {
                if let Some(on_playback_accepted) = on_playback_accepted {
                    on_playback_accepted(cx);
                }
                cx.defer(|cx| {
                    MusicLikedTracksQueueContinuation::shared(cx).update(cx, |continuation, cx| continuation.begin(cx));
                });
            }))
        } else {
            on_playback_accepted
        };
        let options = MusicPlayOptions {
            queue,
            scope_id: scope_id.map(str::to_owned),
            record_play_on_success: counts_play,
            on_playback_accepted: accepted,
            ..MusicPlayOptions::default()
        };
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.play(track, options, cx));
        GuideSignalBus::emit(RiseGuideSignal::TrackPlayed);
        if !counts_play && record_play {
            MusicGuideSandboxHooks::track_playback_started(cx);
        }
    }

    pub fn play_local_track(
        &mut self,
        track: TrackModel,
        queue: Vec<TrackModel>,
        on_playback_accepted: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let accepted: Box<dyn FnOnce(&mut App)> = Box::new(move |cx: &mut App| {
            if let Some(on_playback_accepted) = on_playback_accepted {
                on_playback_accepted(cx);
            }
            cx.defer(|cx| {
                MusicLocalTracksQueueContinuation::shared(cx).update(cx, |continuation, cx| continuation.begin(cx));
            });
        });
        self.play(track, queue, Some(MusicLocalTracksQueueContinuation::SCOPE_ID), true, Some(accepted), cx);
    }

    pub fn play_next(&mut self, track: TrackModel, cx: &mut Context<Self>) {
        if !MusicPlayerState::shared(cx).update(cx, |player, cx| player.insert_next(track, cx)) {
            return;
        }
        self.notify_queued(tr("music_queue_added_next"), cx);
    }

    pub fn add_to_queue(&mut self, track: TrackModel, cx: &mut Context<Self>) {
        if !MusicPlayerState::shared(cx).update(cx, |player, cx| player.append_to_queue(track, cx)) {
            return;
        }
        self.notify_queued(tr("music_queue_added"), cx);
    }

    fn notify_queued(&self, title: String, cx: &mut Context<Self>) {
        let position = Self::player_toast_position(cx);
        Notifier::send(
            title,
            NotifierSendOptions {
                r#type: NotifierType::Success,
                position,
                icon: NotifierIcon::System("checkmark.circle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    /// in the full player and queue sheet a bottom toast sits on the button that sent it
    /// the next press hit the toast and only dismissed it
    fn player_toast_position(cx: &mut App) -> NotifierPosition {
        let player = MusicPlayerState::shared(cx);
        let player = player.read(cx);
        if player.show_now_playing() || player.show_queue_sheet { NotifierPosition::Top } else { NotifierPosition::Bottom }
    }

    pub fn play_collection(
        &mut self,
        tracks: Vec<TrackModel>,
        scope_id: Option<&str>,
        on_playback_accepted: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let start = MusicPlayerState::shared(cx)
            .update(cx, |player, cx| player.collection_start_track(&tracks, scope_id, cx));
        let Some(start) = start else {
            return;
        };
        self.play(start, tracks, scope_id, true, on_playback_accepted, cx);
    }

    /// false when the track didnt start
    /// a player failure is already on the notifier
    pub fn preview_track(&mut self, track: TrackModel, cx: &mut Context<Self>) -> Task<bool> {
        let playback = MusicPlayerState::shared(cx).update(cx, |player, cx| player.play_and_wait(track, Vec::new(), cx));
        cx.spawn(async move |_, cx| {
            if playback.await.is_none() {
                return true;
            }
            cx.update(|cx| {
                let has_player_error = MusicPlayerState::shared(cx).read(cx).last_playback_error().is_some();
                if !has_player_error {
                    Notifier::send(
                        tr("music_preview_playback_failed"),
                        NotifierSendOptions { r#type: NotifierType::Error, ..NotifierSendOptions::default() },
                        cx,
                    );
                }
            });
            false
        })
    }

    pub fn load_playlists_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_playlists_mine_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = SaiCache::new(["getPlaylistsMine"]).key;
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.get_playlists_mine_action(reflect_pending_in_ui, true, cx));
    }

    pub fn refresh_playlists(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_playlists_mine_action(false, true, cx));
    }

    pub fn open_playlist(&mut self, playlist_id: &str, cx: &mut Context<Self>) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_playlist_sai);
        if slice.is_pending() {
            return;
        }
        Self::service(cx).update(cx, |service, cx| service.reconcile_interrupted_message_mp3_imports(playlist_id, cx));
        MusicLocalTrackDraftStore::shared(cx).update(cx, |drafts, cx| drafts.restore_drafts(playlist_id, cx));
        let cache_key = SaiCache::new(["getPlaylist", playlist_id]).key;
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| {
            actions.get_playlist_action(playlist_id, reflect_pending_in_ui, false, None, None, None, cx);
        });
    }

    pub fn resolve_track_link(
        &mut self,
        track_id: &str,
        completion: Rc<dyn Fn(Result<RiseStoredMusicTrack, MusicTrackLinkError>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let canonical_id = RiseonlyUniversalLink::canonical_resource_id(track_id);
        let Some(track_id) = canonical_id.filter(|_| Self::has_wire_user(cx)) else {
            cx.defer(move |cx| completion(Err(MusicTrackLinkError::InvalidLink), cx));
            return;
        };

        let success_completion = Rc::clone(&completion);
        let success_track_id = track_id.clone();
        let on_success: Rc<dyn Fn(Option<RiseStoredMusicTrack>, &mut App)> =
            Rc::new(move |track: Option<RiseStoredMusicTrack>, cx: &mut App| {
                let Some(track) = track else {
                    success_completion(Err(MusicTrackLinkError::NotFound), cx);
                    return;
                };
                let resolved_id = RiseonlyUniversalLink::canonical_resource_id(track.id.as_deref().unwrap_or(""));
                if resolved_id.as_deref() != Some(success_track_id.as_str()) {
                    success_completion(Err(MusicTrackLinkError::LoadFailed), cx);
                    return;
                }
                if !Self::service(cx).read(cx).is_public_link_track(&track) {
                    success_completion(Err(MusicTrackLinkError::AccessDenied), cx);
                    return;
                }
                let playable = track.as_track_model().is_some_and(|model| !model.stable_audio_url().is_empty());
                if !playable {
                    success_completion(Err(MusicTrackLinkError::LoadFailed), cx);
                    return;
                }
                success_completion(Ok(track), cx);
            });
        let failure_completion = completion;
        let on_failure: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let link_error = Self::service(cx).read(cx).track_link_error(error);
            failure_completion(Err(link_error), cx);
        });

        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_track_action(&track_id, true, true, Some(on_success), None, Some(on_failure), cx);
        });
    }

    pub fn resolve_playlist_link(
        &mut self,
        playlist_id: &str,
        completion: Rc<dyn Fn(Result<RiseStoredMusicPlaylist, MusicPlaylistTrackLinkError>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let canonical_id = RiseonlyUniversalLink::canonical_resource_id(playlist_id);
        let Some(playlist_id) = canonical_id.filter(|_| Self::has_wire_user(cx)) else {
            cx.defer(move |cx| completion(Err(MusicPlaylistTrackLinkError::InvalidLink), cx));
            return;
        };

        let success_completion = Rc::clone(&completion);
        let success_playlist_id = playlist_id.clone();
        let on_success: Rc<dyn Fn(Option<RiseStoredMusicPlaylist>, &mut App)> =
            Rc::new(move |playlist: Option<RiseStoredMusicPlaylist>, cx: &mut App| {
                let Some(playlist) = playlist else {
                    success_completion(Err(MusicPlaylistTrackLinkError::PlaylistNotFound), cx);
                    return;
                };
                let resolved_id = RiseonlyUniversalLink::canonical_resource_id(playlist.id.as_deref().unwrap_or(""));
                if resolved_id.as_deref() != Some(success_playlist_id.as_str()) {
                    success_completion(Err(MusicPlaylistTrackLinkError::LoadFailed), cx);
                    return;
                }
                success_completion(Ok(playlist), cx);
            });
        let failure_completion = completion;
        let on_failure: Rc<dyn Fn(&anyhow::Error, &mut App)> = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            let link_error = Self::service(cx).read(cx).playlist_track_link_error(error);
            failure_completion(Err(link_error), cx);
        });

        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_playlist_action(&playlist_id, true, true, Some(on_success), None, Some(on_failure), cx);
        });
    }

    pub fn resolve_playlist_track_link(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        completion: Rc<dyn Fn(Result<MusicResolvedPlaylistTrackLink, MusicPlaylistTrackLinkError>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let Some(track_id) = RiseonlyUniversalLink::canonical_resource_id(track_id) else {
            cx.defer(move |cx| completion(Err(MusicPlaylistTrackLinkError::InvalidLink), cx));
            return;
        };

        let this = cx.weak_entity();
        let playlist_completion: Rc<dyn Fn(Result<RiseStoredMusicPlaylist, MusicPlaylistTrackLinkError>, &mut App)> =
            Rc::new(move |result: Result<RiseStoredMusicPlaylist, MusicPlaylistTrackLinkError>, cx: &mut App| {
                match result {
                    Ok(playlist) => {
                        let track = playlist.tracks.as_ref().and_then(|tracks| {
                            tracks
                                .iter()
                                .find(|track| {
                                    RiseonlyUniversalLink::canonical_resource_id(track.id.as_deref().unwrap_or(""))
                                        .as_deref()
                                        == Some(track_id.as_str())
                                })
                                .cloned()
                        });
                        let Some(track) = track else {
                            let completion = Rc::clone(&completion);
                            let track_id = track_id.clone();
                            this.update(cx, |this, cx| {
                                this.resolve_public_track_fallback(
                                    &track_id,
                                    MusicPlaylistTrackLinkError::TrackNotFound,
                                    completion,
                                    cx,
                                );
                            })
                            .ok();
                            return;
                        };
                        completion(Ok(MusicResolvedPlaylistTrackLink { playlist: Some(playlist), track }), cx);
                    }
                    Err(error) => match error {
                        MusicPlaylistTrackLinkError::PlaylistNotFound | MusicPlaylistTrackLinkError::AccessDenied => {
                            let completion = Rc::clone(&completion);
                            let track_id = track_id.clone();
                            this.update(cx, |this, cx| {
                                this.resolve_public_track_fallback(&track_id, error, completion, cx);
                            })
                            .ok();
                        }
                        MusicPlaylistTrackLinkError::InvalidLink
                        | MusicPlaylistTrackLinkError::AccessSuspended
                        | MusicPlaylistTrackLinkError::TrackNotFound
                        | MusicPlaylistTrackLinkError::LoadFailed => completion(Err(error), cx),
                    },
                }
            });
        self.resolve_playlist_link(playlist_id, playlist_completion, cx);
    }

    fn resolve_public_track_fallback(
        &mut self,
        track_id: &str,
        original_error: MusicPlaylistTrackLinkError,
        completion: Rc<dyn Fn(Result<MusicResolvedPlaylistTrackLink, MusicPlaylistTrackLinkError>, &mut App)>,
        cx: &mut Context<Self>,
    ) {
        let track_completion: Rc<dyn Fn(Result<RiseStoredMusicTrack, MusicTrackLinkError>, &mut App)> =
            Rc::new(move |result: Result<RiseStoredMusicTrack, MusicTrackLinkError>, cx: &mut App| match result {
                Ok(track) => completion(Ok(MusicResolvedPlaylistTrackLink { playlist: None, track }), cx),
                Err(_) => completion(Err(original_error), cx),
            });
        self.resolve_track_link(track_id, track_completion, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_playlist(
        &mut self,
        title: &str,
        description: Option<&str>,
        visibility: &str,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        on_success: Option<Rc<dyn Fn(Option<RiseStoredMusicPlaylist>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if trimming_whitespaces(title).is_empty() {
            return;
        }
        if MusicGuideSandboxHooks::is_armed(cx) {
            MusicGuideSandboxHooks::create_playlist(title, cx);
            let playlist = MusicGuideSandboxHooks::playlist_id()
                .and_then(|sandbox_playlist_id| MusicGuideSandboxHooks::playlist(sandbox_playlist_id, cx));
            if let Some(on_success) = on_success {
                cx.defer(move |cx| on_success(playlist, cx));
            }
            return;
        }
        let created: Rc<dyn Fn(Option<RiseStoredMusicPlaylist>, &mut App)> =
            Rc::new(move |playlist: Option<RiseStoredMusicPlaylist>, cx: &mut App| {
                GuideSignalBus::emit(RiseGuideSignal::PlaylistCreated);
                if let Some(on_success) = &on_success {
                    on_success(playlist, cx);
                }
            });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.create_playlist_action(title, description, visibility, local_cover_file, Some(created), on_error, cx);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_playlist(
        &mut self,
        playlist_id: &str,
        title: Option<&str>,
        description: Option<&str>,
        visibility: Option<&str>,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        existing_playlist: Option<RiseStoredMusicPlaylist>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_playlist_action(
                playlist_id,
                title,
                description,
                visibility,
                None,
                local_cover_file,
                existing_playlist,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn set_playlist_new_tracks_on_top(
        &mut self,
        playlist_id: &str,
        is_on: bool,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let existing_playlist = Self::service(cx).update(cx, |service, cx| service.playlist(playlist_id, cx));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_playlist_action(
                playlist_id,
                None,
                None,
                None,
                Some(is_on),
                None,
                existing_playlist,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn delete_playlist(
        &mut self,
        playlist_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.delete_playlist_action(playlist_id, on_success, on_error, cx);
        });
    }

    pub fn add_track_to_playlist(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let added: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            GuideSignalBus::emit(RiseGuideSignal::TrackAddedToPlaylist);
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.add_track_to_playlist_action(playlist_id, track_id, Some(added), on_error, cx);
        });
    }

    pub fn remove_track_from_playlist(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let is_pending_local =
            Self::service(cx).update(cx, |service, cx| service.is_pending_local_track(playlist_id, track_id, cx));
        if is_pending_local {
            self.remove_pending_local_track(Some(playlist_id), track_id, cx);
            if let Some(on_success) = on_success {
                cx.defer(move |cx| on_success(cx));
            }
            return;
        }
        let position = Self::player_toast_position(cx);
        let failed: Rc<dyn Fn(String, &mut App)> = Rc::new(move |message: String, cx: &mut App| {
            if let Some(on_error) = &on_error {
                on_error(message, cx);
                return;
            }
            Notifier::send(
                tr("ctx_remove_from_playlist"),
                NotifierSendOptions {
                    content: Some(message),
                    r#type: NotifierType::Error,
                    position,
                    icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.remove_track_from_playlist_action(playlist_id, track_id, on_success, Some(failed), cx);
        });
    }

    pub fn move_playlist_track(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        after_track_id: Option<&str>,
        on_error: Rc<dyn Fn(&mut App)>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let previous = Self::service(cx).update(cx, |service, cx| {
            service.apply_move_playlist_track_optimistic(playlist_id, track_id, after_track_id, cx)
        });
        self.playlist_moves.push(PlaylistMove {
            playlist_id: playlist_id.to_owned(),
            track_id: track_id.to_owned(),
            after_track_id: after_track_id.map(str::to_owned),
            previous,
            on_error,
        });
        self.send_next_playlist_move(cx);
    }

    fn send_next_playlist_move(&mut self, cx: &mut Context<Self>) {
        if self.is_sending_playlist_move || self.playlist_moves.is_empty() {
            return;
        }
        let playlist_move = self.playlist_moves.remove(0);
        self.is_sending_playlist_move = true;
        let known = Self::service(cx)
            .update(cx, |service, cx| service.playlist(&playlist_move.playlist_id, cx))
            .and_then(|playlist| playlist.revision);
        let acked = self.playlist_move_revisions.get(&playlist_move.playlist_id).copied();
        let expected = [known, acked].into_iter().flatten().max();
        let position = Self::player_toast_position(cx);

        let success_target = cx.weak_entity();
        let success_playlist_id = playlist_move.playlist_id.clone();
        let on_success: Rc<dyn Fn(Option<i64>, &mut App)> = Rc::new(move |revision: Option<i64>, cx: &mut App| {
            let playlist_id = success_playlist_id.clone();
            success_target
                .update(cx, |this, cx| {
                    if let Some(revision) = revision {
                        this.playlist_move_revisions.insert(playlist_id, revision);
                    }
                    this.is_sending_playlist_move = false;
                    this.send_next_playlist_move(cx);
                })
                .ok();
        });

        let error_target = cx.weak_entity();
        let error_playlist_id = playlist_move.playlist_id.clone();
        let previous = playlist_move.previous.clone();
        let move_on_error = Rc::clone(&playlist_move.on_error);
        let on_error: Rc<dyn Fn(String, &mut App)> = Rc::new(move |message: String, cx: &mut App| {
            let playlist_id = error_playlist_id.clone();
            let previous = previous.clone();
            let move_on_error = Rc::clone(&move_on_error);
            error_target
                .update(cx, |this, cx| {
                    this.playlist_moves.retain(|queued| queued.playlist_id != playlist_id);
                    this.playlist_move_revisions.remove(&playlist_id);
                    this.is_sending_playlist_move = false;
                    if let Some(previous) = &previous {
                        Self::service(cx).update(cx, |service, cx| service.patch_playlist_detail(previous, cx));
                    }
                    move_on_error(cx);
                    this.open_playlist(&playlist_id, cx);
                    Notifier::send(
                        tr("ctx_reorder"),
                        NotifierSendOptions {
                            content: Some(message),
                            r#type: NotifierType::Error,
                            position,
                            icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                    this.send_next_playlist_move(cx);
                })
                .ok();
        });

        Self::actions(cx).update(cx, |actions, cx| {
            actions.move_playlist_track_action(
                &playlist_move.playlist_id,
                &playlist_move.track_id,
                playlist_move.after_track_id.as_deref(),
                expected,
                Some(on_success),
                Some(on_error),
                cx,
            );
        });
    }

    pub fn remove_pending_local_track(&mut self, playlist_id: Option<&str>, track_id: &str, cx: &mut Context<Self>) {
        let tid = trimming_whitespaces_and_newlines(track_id);
        if tid.is_empty() {
            return;
        }
        let pid = playlist_id.map(trimming_whitespaces_and_newlines).filter(|pid| !pid.is_empty());
        if let Some(pid) = pid {
            Self::service(cx).update(cx, |service, cx| service.cancel_pending_local_track(pid, tid, cx));
        }
        MusicLocalTrackDraftStore::shared(cx).update(cx, |drafts, cx| drafts.remove(tid, pid, cx));
    }

    pub fn retry_pending_local_track(&mut self, track_id: &str, cx: &mut Context<Self>) {
        MusicLocalTrackDraftStore::shared(cx).update(cx, |drafts, cx| drafts.retry(track_id, cx));
    }

    pub fn add_track_to_profile(&mut self, track: TrackModel, cx: &mut Context<Self>) {
        let user_id = AuthServicesStore::shared(cx).read(cx).profile().map(|profile| profile.id.clone()).unwrap_or_default();
        let position = Self::player_toast_position(cx);
        let result = FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.add(track, &user_id, cx));
        match result {
            FeaturedProfileTrackAddResult::Added | FeaturedProfileTrackAddResult::AlreadyPinned => {
                GuideSignalBus::emit(RiseGuideSignal::TrackAddedToProfile);
                Notifier::send(
                    tr("music_added_to_profile"),
                    NotifierSendOptions {
                        r#type: NotifierType::Success,
                        position,
                        icon: NotifierIcon::System("checkmark.circle.fill".to_owned()),
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }
            FeaturedProfileTrackAddResult::LimitReached => {
                let limit = i64::try_from(FeaturedProfileTrackStore::limit()).unwrap_or(i64::MAX);
                Notifier::send(
                    tr_args("music_profile_tracks_limit", &[FormatArg::Int(limit)]),
                    NotifierSendOptions {
                        r#type: NotifierType::Info,
                        position,
                        icon: NotifierIcon::System("exclamationmark.circle.fill".to_owned()),
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }
            FeaturedProfileTrackAddResult::NotPinnable => {}
        }
    }

    pub fn remove_track_from_profile(&mut self, track_id: &str, cx: &mut Context<Self>) {
        let store = FeaturedProfileTrackStore::shared(cx);
        if !store.update(cx, |store, _| store.contains(track_id)) {
            return;
        }
        store.update(cx, |store, cx| store.remove(track_id, cx));
        Notifier::send(
            tr("music_removed_from_profile"),
            NotifierSendOptions {
                r#type: NotifierType::Success,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("checkmark.circle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub fn reorder_profile_tracks(&mut self, ids: &[String], cx: &mut Context<Self>) {
        FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.reorder(ids, cx));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_local_track_in_playlist(
        &mut self,
        request_id: &str,
        temp_id: &str,
        playlist_id: &str,
        title: &str,
        description: Option<&str>,
        artist_name: &str,
        duration_ms: i64,
        local_audio_file: SaiWsOptimisticLocalFile,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_created: Option<MusicLocalTrackDraftCreationObserver>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let title_rule = MusicFieldRules::local_track_title();
        let artist_rule = MusicFieldRules::local_track_artist();
        let clean_title = title_rule.schema.map(|schema| schema.normalize(title)).unwrap_or_else(|| title.to_owned());
        let clean_artist =
            artist_rule.schema.map(|schema| schema.normalize(artist_name)).unwrap_or_else(|| artist_name.to_owned());
        if request_id.is_empty()
            || temp_id.is_empty()
            || playlist_id.is_empty()
            || !title_rule.is_satisfied(&clean_title)
            || !artist_rule.is_satisfied(&clean_artist)
            || duration_ms < 500
        {
            return;
        }

        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            if MusicGuideSandboxHooks::playlist_id() != Some(playlist_id) {
                if let Some(on_error) = on_error {
                    cx.defer(move |cx| on_error(tr("guide_sandbox_action_unavailable"), cx));
                }
                return;
            }
            MusicGuideSandboxHooks::add_reader_track(
                MusicGuideSandboxReaderTrack {
                    title: clean_title,
                    artist_name: clean_artist,
                    duration_ms,
                    audio_file: local_audio_file.url,
                    cover_file: local_cover_file.map(|file| file.url),
                },
                cx,
            );
            if let Some(on_success) = on_success {
                cx.defer(on_success);
            }
            return;
        }

        let drafts = MusicLocalTrackDraftStore::shared(cx);
        if let Some(on_created) = on_created {
            drafts.update(cx, |drafts, _| drafts.observe_creation(temp_id, on_created));
        }
        let clean_description = description.map(trimming_whitespaces_and_newlines);
        drafts.update(cx, |drafts, cx| {
            drafts.enqueue(
                request_id,
                temp_id,
                playlist_id,
                &clean_title,
                clean_description,
                &clean_artist,
                duration_ms,
                local_audio_file,
                local_cover_file,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn import_local_folder(&mut self, folder: PathBuf, playlist_id: &str, cx: &mut Context<Self>) {
        let playlist_id = playlist_id.to_owned();
        MusicLocalFolderImport::shared(cx).update(cx, |import, cx| import.start(folder, playlist_id, cx));
    }

    pub fn clear_local_track_creation_observer(&mut self, temp_id: &str, cx: &mut Context<Self>) {
        MusicLocalTrackDraftStore::shared(cx).update(cx, |drafts, _| drafts.clear_creation_observer(temp_id));
    }

    pub fn update_local_track(
        &mut self,
        track_id: &str,
        title: &str,
        artist_name: &str,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let title_rule = MusicFieldRules::local_track_title();
        let artist_rule = MusicFieldRules::local_track_artist();
        let clean_title = title_rule.schema.map(|schema| schema.normalize(title)).unwrap_or_else(|| title.to_owned());
        let clean_artist =
            artist_rule.schema.map(|schema| schema.normalize(artist_name)).unwrap_or_else(|| artist_name.to_owned());
        let source = Self::service(cx).update(cx, |service, cx| service.local_track_wire(track_id, cx));
        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let source = source.filter(|source| {
            title_rule.is_satisfied(&clean_title)
                && artist_rule.is_satisfied(&clean_artist)
                && user_id.is_some()
                && source.status.as_deref() == Some(RiseMusicTrackStatus::LocalLibrary.wire())
                && source.library_owner_user_id == user_id
        });
        let Some(source) = source else {
            if let Some(on_error) = on_error {
                cx.defer(move |cx| on_error(tr("music_local_upload_invalid_response"), cx));
            }
            return;
        };
        let request_id = Uuid::new_v4().uuid_string().to_lowercase();
        let updated: Option<Box<dyn FnOnce(RiseStoredMusicTrack, &mut App)>> = on_success.map(|on_success| {
            let updated: Box<dyn FnOnce(RiseStoredMusicTrack, &mut App)> =
                Box::new(move |_: RiseStoredMusicTrack, cx: &mut App| on_success(cx));
            updated
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.update_local_track_action(
                source,
                &request_id,
                &clean_title,
                &clean_artist,
                local_cover_file,
                updated,
                on_error,
                cx,
            );
        });
    }

    pub fn open_album(&mut self, album_id: &str, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_album_sai);
        if slice.is_pending() {
            return;
        }
        let has_data = slice.data().and_then(|data| data.album).is_some_and(|album| album.id == album_id);
        if has_data {
            actions.update(cx, |actions, cx| actions.get_album_action(album_id, false, None, None, cx));
            return;
        }
        let cache_key = SaiCache::new(["getAlbum", album_id]).key;
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.get_album_action(album_id, reflect_pending_in_ui, None, None, cx));
    }

    fn loaded_album_tracks(album_id: &str, cx: &mut App) -> Vec<TrackModel> {
        MusicActionsStore::slices(cx)
            .get_album_sai
            .data()
            .and_then(|data| data.album)
            .and_then(|album| album.as_album_model())
            .filter(|album| album.id == album_id)
            .map(|album| album.tracks)
            .unwrap_or_default()
    }

    pub fn open_full_player_on_accept(cx: &mut App) {
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_show_now_playing(true, cx));
    }

    pub fn play_album(&mut self, album_id: &str, fallback: Vec<TrackModel>, cx: &mut Context<Self>) {
        let scope_id = format!("album:{album_id}");
        let tracks = Self::loaded_album_tracks(album_id, cx);
        if !tracks.is_empty() {
            self.album_playback_request_id = None;
            self.play_collection(tracks, Some(&scope_id), Some(Box::new(Self::open_full_player_on_accept)), cx);
            return;
        }
        self.album_playback_request_id = Some(album_id.to_owned());

        let success_target = cx.weak_entity();
        let success_album_id = album_id.to_owned();
        let success_fallback = fallback.clone();
        let on_success: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let album_id = success_album_id.clone();
            let fallback = success_fallback.clone();
            let scope_id = scope_id.clone();
            success_target
                .update(cx, |this, cx| {
                    if this.album_playback_request_id.as_deref() != Some(album_id.as_str()) {
                        return;
                    }
                    this.album_playback_request_id = None;
                    let tracks = Self::loaded_album_tracks(&album_id, cx);
                    if tracks.is_empty() {
                        this.play_collection(fallback, None, Some(Box::new(Self::open_full_player_on_accept)), cx);
                    } else {
                        this.play_collection(tracks, Some(&scope_id), Some(Box::new(Self::open_full_player_on_accept)), cx);
                    }
                })
                .ok();
        });
        let error_target = cx.weak_entity();
        let error_album_id = album_id.to_owned();
        let on_error: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let album_id = error_album_id.clone();
            let fallback = fallback.clone();
            error_target
                .update(cx, |this, cx| {
                    if this.album_playback_request_id.as_deref() != Some(album_id.as_str()) {
                        return;
                    }
                    this.album_playback_request_id = None;
                    this.play_collection(fallback, None, Some(Box::new(Self::open_full_player_on_accept)), cx);
                })
                .ok();
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_album_action(album_id, false, Some(on_success), Some(on_error), cx);
        });
    }

    pub fn refresh_album(&mut self, album_id: &str, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).get_album_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_album_action(album_id, false, None, None, cx));
    }

    pub fn search(&mut self, q: &str, on_result: Option<Rc<dyn Fn(String, &mut App)>>, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces(q).to_owned();
        if trimmed.is_empty() {
            Self::service(cx).update(cx, |service, cx| service.clear_search(cx));
            return;
        }
        MusicLocalState::shared(cx).update(cx, |state, cx| state.add_search(&trimmed, cx));
        let cache_key = MusicServicesStore::search_cache_key(&trimmed, None);
        let slice = Rc::clone(&MusicActionsStore::slices(cx).search_sai);
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        SaiDebouncer::debounced_action_delay_milliseconds(
            "music_search",
            Box::new(move |cx: &mut App| {
                let reflect_pending_in_ui = slice.data().is_none();
                Self::actions(cx).update(cx, |actions, cx| {
                    actions.search_action(&trimmed, None, None, reflect_pending_in_ui, on_result, cx);
                });
            }),
            350,
            "music_search",
            1,
            None,
            cx,
        );
    }

    pub fn load_more_search(&mut self, q: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(q);
        if trimmed.is_empty() {
            return;
        }
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).search_sai);
        let data = slice.data();
        if !data.as_ref().is_some_and(|data| data.has_more) {
            return;
        }
        let Some(relative_id) = data.and_then(|data| data.next_cursor).filter(|cursor| !cursor.is_empty()) else {
            return;
        };
        if slice.is_pending() {
            return;
        }
        let cache_key = MusicServicesStore::search_cache_key(trimmed, None);
        if !SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().can_request_automatically() {
            return;
        }
        actions.update(cx, |actions, cx| {
            actions.search_action(trimmed, None, Some(&relative_id), false, None, cx);
        });
    }

    pub fn expanded_search_artists_query(&self) -> Option<&str> {
        self.expanded_search_artists_query.as_deref()
    }

    pub fn is_loading_more_search_artists(&self, q: &str, cx: &mut App) -> bool {
        let trimmed = trimming_whitespaces_and_newlines(q);
        if self.expanded_search_artists_query.as_deref() != Some(trimmed) {
            return false;
        }
        let cache_key = MusicServicesStore::search_cache_key(trimmed, Some(key::ARTIST));
        MusicActionsStore::slices(cx).search_artists_sai.is_pending()
            || SaiWs::pagination_state(&cache_key, cx).read(cx).trailing().is_loading
    }

    pub fn show_more_search_artists(&mut self, q: &str, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(q).to_owned();
        if trimmed.is_empty() {
            return;
        }
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).search_artists_sai);
        if slice.is_pending() {
            return;
        }
        let loading_query = self.expanded_search_artists_query.clone().unwrap_or_else(|| trimmed.clone());
        if self.is_loading_more_search_artists(&loading_query, cx) {
            return;
        }
        let data = slice.data();
        let has_artists = data.as_ref().and_then(|data| data.artists.as_ref()).is_some_and(|artists| !artists.is_empty());
        if self.expanded_search_artists_query.as_deref() == Some(trimmed.as_str()) && has_artists {
            if !data.as_ref().is_some_and(|data| data.has_more) {
                return;
            }
            let Some(cursor) = data.and_then(|data| data.next_cursor).filter(|cursor| !cursor.is_empty()) else {
                return;
            };
            actions.update(cx, |actions, cx| {
                actions.search_artists_action(&trimmed, Some(&cursor), Self::SEARCH_ARTISTS_PAGE_SIZE, cx);
            });
            return;
        }
        self.expanded_search_artists_query = Some(trimmed.clone());
        cx.notify();
        Self::service(cx).update(cx, |service, cx| service.clear_search_artists(cx));
        actions.update(cx, |actions, cx| {
            actions.search_artists_action(&trimmed, None, Self::SEARCH_ARTISTS_PAGE_SIZE * 2, cx);
        });
    }

    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.clear_search(cx));
    }

    pub fn load_artist_status_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_artist_status_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = SaiCache::new(["getArtistStatus"]).key;
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        actions.update(cx, |actions, cx| actions.get_artist_status_action(cx));
    }

    pub fn load_my_tracks_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).list_my_tracks_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = MusicServicesStore::my_tracks_cache_key();
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.list_my_tracks_action(None, reflect_pending_in_ui, true, cx));
    }

    pub fn refresh_my_tracks(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.list_my_tracks_action(None, false, true, cx));
    }

    pub fn load_more_my_tracks(&mut self, cx: &mut Context<Self>) {
        let Some(cursor) = Self::continuation_cursor(&MusicServicesStore::my_tracks_cache_key(), cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| actions.list_my_tracks_action(Some(&cursor), false, true, cx));
    }

    pub fn load_my_local_tracks_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).list_my_local_tracks_sai);
        if slice.is_pending() {
            return;
        }

        let cache_key = MusicServicesStore::my_local_tracks_cache_key();
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.list_my_local_tracks_action(None, reflect_pending_in_ui, true, cx));
    }

    pub fn refresh_my_local_tracks(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.list_my_local_tracks_action(None, false, true, cx));
    }

    pub fn load_more_my_local_tracks(&mut self, cx: &mut Context<Self>) {
        let Some(cursor) = Self::continuation_cursor(&MusicServicesStore::my_local_tracks_cache_key(), cx) else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| {
            actions.list_my_local_tracks_action(Some(&cursor), false, true, cx);
        });
    }

    pub fn delete_local_track(
        &mut self,
        track_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let id = trimming_whitespaces_and_newlines(track_id).to_owned();
        let actions = Self::actions(cx);
        if id.is_empty() || actions.read(cx).delete_local_tracks_sai.is_pending() {
            return;
        }
        let request_id = self
            .local_track_delete_request_ids
            .get(&id)
            .cloned()
            .unwrap_or_else(|| Uuid::new_v4().uuid_string().to_lowercase());
        self.local_track_delete_request_ids.insert(id.clone(), request_id.clone());

        let this = cx.weak_entity();
        let deleted_id = id.clone();
        let deleted: Rc<dyn Fn(Vec<String>, &mut App)> = Rc::new(move |deleted_ids: Vec<String>, cx: &mut App| {
            this.update(cx, |this, _| {
                this.local_track_delete_request_ids.remove(&deleted_id);
            })
            .ok();
            let deleted_ids: HashSet<String> = deleted_ids.into_iter().collect();
            MusicPlayerState::shared(cx)
                .update(cx, |player, cx| player.remove_permanently_deleted_tracks(&deleted_ids, cx));
            MusicLocalState::shared(cx).update(cx, |state, cx| state.remove_tracks(&deleted_ids, cx));
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        actions.update(cx, |actions, cx| {
            actions.delete_local_tracks_action(&[id], &request_id, Some(deleted), on_error, cx);
        });
    }

    pub fn load_my_albums_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).list_my_albums_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = SaiCache::new(["listMyAlbums"]).key;
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.list_my_albums_action(reflect_pending_in_ui, true, cx));
    }

    pub fn refresh_my_albums(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).list_my_albums_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.list_my_albums_action(false, true, cx));
    }

    pub fn load_analytics_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_artist_analytics_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = SaiCache::new(["getArtistAnalytics"]).key;
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.get_artist_analytics_action(reflect_pending_in_ui, true, cx));
    }

    pub fn refresh_analytics(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_artist_analytics_action(false, true, cx));
    }

    pub fn can_submit_artist_request(&self, stage_name: &str, genre: &str) -> bool {
        !trimming_whitespaces_and_newlines(stage_name).is_empty() && !genre.is_empty()
    }

    pub fn submit_artist_request(
        &mut self,
        stage_name: &str,
        genre: &str,
        bio: &str,
        instagram: Option<&str>,
        vk: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if !self.can_submit_artist_request(stage_name, genre) {
            return;
        }
        fn trimmed(value: Option<&str>) -> Option<&str> {
            value.map(trimming_whitespaces_and_newlines).filter(|value| !value.is_empty())
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.request_artist_action(
                trimming_whitespaces_and_newlines(stage_name),
                genre,
                trimming_whitespaces_and_newlines(bio),
                trimmed(instagram),
                trimmed(vk),
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_track(
        &mut self,
        title: &str,
        genre: &str,
        local_cover_file: SaiWsOptimisticLocalFile,
        local_audio_file: SaiWsOptimisticLocalFile,
        album_id: Option<&str>,
        is_exclusive: bool,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.create_track_action(
                title,
                genre,
                local_cover_file,
                local_audio_file,
                album_id,
                is_exclusive,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn toggle_track_exclusive(&mut self, track_id: &str, is_exclusive: bool, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.update_track_action(track_id, is_exclusive, None, None, cx));
    }

    pub fn request_album(
        &mut self,
        title: &str,
        genre: &str,
        local_cover_file: SaiWsOptimisticLocalFile,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.request_album_action(title, genre, local_cover_file, None, on_success, on_error, cx);
        });
    }

    pub fn load_my_album_requests_if_needed(&mut self, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_my_album_requests_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = MusicServicesStore::my_album_requests_cache_key();
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect_pending_in_ui = Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| actions.get_my_album_requests_action(None, reflect_pending_in_ui, true, cx));
    }

    pub fn refresh_my_album_requests(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_my_album_requests_action(None, false, true, cx));
    }

    pub fn load_more_my_album_requests(&mut self, cx: &mut Context<Self>) {
        let Some(cursor) = Self::continuation_cursor(&MusicServicesStore::my_album_requests_cache_key(), cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_my_album_requests_action(Some(&cursor), false, true, cx);
        });
    }

    pub fn publish_approved_track(
        &mut self,
        track_id: &str,
        scheduled_at_ms: Option<i64>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.publish_track_action(track_id, scheduled_at_ms, on_success, on_error, cx);
        });
    }

    pub fn publish_approved_album(
        &mut self,
        album_id: &str,
        scheduled_at_ms: Option<i64>,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let this = cx.weak_entity();
        let published: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            this.update(cx, |this, cx| this.refresh_my_tracks(cx)).ok();
            if let Some(on_success) = &on_success {
                on_success(cx);
            }
        });
        Self::actions(cx).update(cx, |actions, cx| {
            actions.publish_album_action(album_id, scheduled_at_ms, Some(published), on_error, cx);
        });
    }

    pub fn load_moderation_queue_metrics_if_needed(&mut self, reflect_pending_in_ui: bool, cx: &mut Context<Self>) {
        if !Self::service(cx).update(cx, |service, cx| service.music_is_staff(cx)) {
            return;
        }
        let actions = Self::actions(cx);
        let slice = Rc::clone(&actions.read(cx).get_moderation_queue_metrics_sai);
        if slice.is_pending() {
            return;
        }
        let cache_key = SaiCache::new(["getModerationQueueMetrics"]).key;
        Self::service(cx).update(cx, |service, cx| service.preload_slice_from_disk_if_needed(&slice, &cache_key, cx));
        let reflect = reflect_pending_in_ui && Self::reflect_pending_in_ui(&slice);
        actions.update(cx, |actions, cx| {
            actions.get_moderation_queue_metrics_action(reflect, true, cx);
        });
    }

    pub fn refresh_moderation_queue_metrics(&mut self, cx: &mut Context<Self>) {
        if !Self::service(cx).update(cx, |service, cx| service.music_is_staff(cx)) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_moderation_queue_metrics_action(false, true, cx));
    }

    pub fn load_moderation_artist_requests(&mut self, status: &str, fetch_if_have_data: bool, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).list_artist_requests_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.list_artist_requests_action(status, None, fetch_if_have_data, cx));
    }

    pub fn refresh_moderation_artist_requests(&mut self, status: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.list_artist_requests_action(status, None, true, cx));
    }

    pub fn load_more_moderation_artist_requests(&mut self, status: &str, cx: &mut Context<Self>) {
        let cache_key = MusicServicesStore::artist_requests_cache_key(status);
        let Some(cursor) = Self::continuation_cursor(&cache_key, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.list_artist_requests_action(status, Some(&cursor), true, cx);
        });
    }

    pub fn open_moderation_artist_request(&mut self, request_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_artist_request_action(request_id, cx));
    }

    pub fn review_moderation_artist_request(
        &mut self,
        request_id: &str,
        approve: bool,
        rejection_reason: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.review_artist_request_action(
                request_id,
                approve,
                rejection_reason,
                RiseMusicTrackStatus::Pending.wire(),
                cx,
            );
        });
    }

    pub fn load_moderation_track_requests(&mut self, status: &str, fetch_if_have_data: bool, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).list_track_requests_sai.is_pending() {
            return;
        }
        if status == RiseMusicTrackStatus::Pending.wire() && actions.read(cx).review_track_request_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.list_track_requests_action(status, None, fetch_if_have_data, cx));
    }

    pub fn refresh_moderation_track_requests(&mut self, status: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.list_track_requests_action(status, None, true, cx));
    }

    pub fn load_more_moderation_track_requests(&mut self, status: &str, cx: &mut Context<Self>) {
        let cache_key = MusicServicesStore::track_requests_cache_key(status);
        let Some(cursor) = Self::continuation_cursor(&cache_key, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.list_track_requests_action(status, Some(&cursor), true, cx);
        });
    }

    pub fn open_moderation_track_request(&mut self, track_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_track_request_action(track_id, cx));
    }

    pub fn review_moderation_track_request(
        &mut self,
        track_id: &str,
        approve: bool,
        rejection_reason: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.review_track_request_action(
                track_id,
                approve,
                rejection_reason,
                RiseMusicTrackStatus::Pending.wire(),
                cx,
            );
        });
    }

    pub fn load_moderation_album_requests(&mut self, status: &str, fetch_if_have_data: bool, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).list_album_requests_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.list_album_requests_action(status, None, fetch_if_have_data, cx));
    }

    pub fn refresh_moderation_album_requests(&mut self, status: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.list_album_requests_action(status, None, true, cx));
    }

    pub fn load_more_moderation_album_requests(&mut self, status: &str, cx: &mut Context<Self>) {
        let cache_key = MusicServicesStore::album_requests_cache_key(status);
        let Some(cursor) = Self::continuation_cursor(&cache_key, cx) else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.list_album_requests_action(status, Some(&cursor), true, cx);
        });
    }

    pub fn open_moderation_album_request(&mut self, request_id: &str, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_album_request_action(request_id, cx));
    }

    pub fn review_moderation_album_request(
        &mut self,
        request_id: &str,
        approve: bool,
        rejection_reason: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.review_album_request_action(
                request_id,
                approve,
                rejection_reason,
                RiseMusicTrackStatus::Pending.wire(),
                cx,
            );
        });
    }

    pub fn load_my_artist_request_if_needed(&mut self, fetch_if_have_data: bool, cx: &mut Context<Self>) {
        let actions = Self::actions(cx);
        if actions.read(cx).get_my_artist_request_sai.is_pending() {
            return;
        }
        actions.update(cx, |actions, cx| actions.get_my_artist_request_action(fetch_if_have_data, cx));
    }

    pub fn refresh_my_artist_request(&mut self, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_my_artist_request_action(true, cx));
    }

    pub fn load_playlist_settings(&mut self, playlist_id: &str, force_refresh: bool, cx: &mut Context<Self>) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let actions = Self::actions(cx);
        if playlist_id.is_empty() || actions.read(cx).get_playlist_settings_sai.is_pending() {
            return;
        }

        let has_data =
            Self::service(cx).update(cx, |service, cx| service.playlist_settings(playlist_id, cx)).is_some();
        if !force_refresh && has_data {
            actions.update(cx, |actions, cx| actions.get_playlist_settings_action(playlist_id, false, cx));
            return;
        }
        actions.update(cx, |actions, cx| actions.get_playlist_settings_action(playlist_id, !has_data, cx));
    }

    pub fn load_playlist_invite_link(&mut self, playlist_id: &str, force_refresh: bool, cx: &mut Context<Self>) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let actions = Self::actions(cx);
        if playlist_id.is_empty() || actions.read(cx).get_playlist_invite_link_sai.is_pending() {
            return;
        }

        let has_data =
            Self::service(cx).update(cx, |service, cx| service.playlist_invite_link(playlist_id, cx)).is_some();
        if force_refresh || !has_data {
            actions.update(cx, |actions, cx| actions.get_playlist_invite_link_action(playlist_id, !has_data, cx));
        }
    }

    pub fn load_playlist_invite_preview(&mut self, token: &str, force_refresh: bool, cx: &mut Context<Self>) {
        let normalized = trimming_whitespaces_and_newlines(token);
        let actions = Self::actions(cx);
        if normalized.is_empty() || actions.read(cx).get_playlist_invite_preview_sai.is_pending() {
            return;
        }

        let has_data =
            Self::service(cx).update(cx, |service, cx| service.playlist_invite_preview(normalized, cx)).is_some();
        if force_refresh || !has_data {
            actions.update(cx, |actions, cx| {
                actions.get_playlist_invite_preview_action(normalized, !has_data, None, None, cx);
            });
        }
    }

    pub fn request_playlist_join(
        &mut self,
        token: &str,
        on_success: Option<Rc<dyn Fn(Option<MusicPlaylistJoinRequest>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let normalized = trimming_whitespaces_and_newlines(token);
        let actions = Self::actions(cx);
        if normalized.is_empty() || actions.read(cx).request_playlist_join_sai.is_pending() {
            return;
        }

        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        actions.update(cx, |actions, cx| {
            actions.request_playlist_join_action(normalized, &client_request_id, on_success, on_error, cx);
        });
    }

    pub fn note_playlist_invite_link_copied(&mut self, playlist_id: &str, cx: &mut Context<Self>) {
        GuideSignalBus::emit(RiseGuideSignal::InviteLinkCopied);
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            MusicGuideSandboxHooks::note_invite_link_copied(cx);
        }
    }

    pub fn load_playlist_join_requests(&mut self, playlist_id: &str, force_refresh: bool, cx: &mut Context<Self>) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let actions = Self::actions(cx);
        if playlist_id.is_empty() || actions.read(cx).get_playlist_join_requests_sai.is_pending() {
            return;
        }

        let has_requests =
            !Self::service(cx).update(cx, |service, cx| service.playlist_join_requests(playlist_id, cx)).is_empty();
        let has_data = has_requests
            || SaiWsResponseCache::current_payload(
                &MusicServicesStore::playlist_join_requests_cache_key(playlist_id),
                SaiWsCachePreferredLayer::MemoryFirst,
                SaiWsDiskCachePolicy::PreserveExisting,
                cx,
            )
            .is_some();
        if force_refresh || !has_data {
            actions.update(cx, |actions, cx| {
                actions.get_playlist_join_requests_action(playlist_id, None, !has_data, cx);
            });
        }
    }

    pub fn load_more_playlist_join_requests(&mut self, playlist_id: &str, cx: &mut Context<Self>) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let cache_key = MusicServicesStore::playlist_join_requests_cache_key(playlist_id);
        let Some(cursor) = Self::continuation_cursor(&cache_key, cx) else {
            return;
        };
        let actions = Self::actions(cx);
        if actions.read(cx).get_playlist_join_requests_sai.is_pending() {
            return;
        }

        actions.update(cx, |actions, cx| {
            actions.get_playlist_join_requests_action(playlist_id, Some(&cursor), false, cx);
        });
    }

    pub fn review_playlist_join_request(
        &mut self,
        playlist_id: &str,
        request_id: &str,
        approve: bool,
        on_success: Option<Rc<dyn Fn(Option<MusicPlaylistMember>, &mut App)>>,
        on_error: Option<Rc<dyn Fn(Option<String>, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let actions = Self::actions(cx);
        if playlist_id.is_empty() || request_id.is_empty() || actions.read(cx).review_playlist_join_request_sai.is_pending()
        {
            return;
        }

        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            let member = MusicGuideSandboxHooks::review_join_request(approve, cx);
            if let Some(on_success) = on_success {
                cx.defer(move |cx| on_success(member, cx));
            }
            return;
        }

        let reviewed: Rc<dyn Fn(Option<MusicPlaylistMember>, &mut App)> =
            Rc::new(move |member: Option<MusicPlaylistMember>, cx: &mut App| {
                if approve {
                    GuideSignalBus::emit(RiseGuideSignal::JoinRequestApproved);
                }
                if let Some(on_success) = &on_success {
                    on_success(member, cx);
                }
            });
        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        actions.update(cx, |actions, cx| {
            actions.review_playlist_join_request_action(
                playlist_id,
                request_id,
                approve,
                &client_request_id,
                Some(reviewed),
                on_error,
                cx,
            );
        });
    }

    pub fn load_playlist_members(
        &mut self,
        playlist_id: &str,
        include_suspended: bool,
        force_refresh: bool,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        let actions = Self::actions(cx);
        if playlist_id.is_empty() || actions.read(cx).get_playlist_members_sai.is_pending() {
            return;
        }

        let cache_key = MusicServicesStore::playlist_members_cache_key(playlist_id, include_suspended);
        let has_data = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        )
        .is_some();
        if force_refresh || !has_data {
            actions.update(cx, |actions, cx| {
                actions.get_playlist_members_action(playlist_id, include_suspended, None, !has_data, cx);
            });
        }
    }

    pub fn load_more_playlist_members(&mut self, playlist_id: &str, include_suspended: bool, cx: &mut Context<Self>) {
        let cache_key = MusicServicesStore::playlist_members_cache_key(playlist_id, include_suspended);
        let Some(cursor) = Self::continuation_cursor(&cache_key, cx) else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_playlist_members_action(playlist_id, include_suspended, Some(&cursor), false, cx);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_playlist_member_permissions(
        &mut self,
        playlist_id: &str,
        member_user_id: &str,
        permissions: MusicPlaylistMemberPermissions,
        expected_version: Option<i64>,
        on_success: Option<Rc<dyn Fn(MusicPlaylistMember, &mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            let member = MusicGuideSandboxHooks::update_panda_permissions(permissions, cx);
            if let (Some(member), Some(on_success)) = (member, on_success) {
                cx.defer(move |cx| on_success(member, cx));
            }
            return;
        }
        let actions = Self::actions(cx);
        if playlist_id.is_empty()
            || member_user_id.is_empty()
            || actions.read(cx).update_playlist_member_permissions_sai.is_pending()
        {
            return;
        }

        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        actions.update(cx, |actions, cx| {
            actions.update_playlist_member_permissions_action(
                playlist_id,
                member_user_id,
                permissions,
                expected_version,
                &client_request_id,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn remove_playlist_member(
        &mut self,
        playlist_id: &str,
        member_user_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            MusicGuideSandboxHooks::remove_panda_from_playlist(cx);
            if let Some(on_success) = on_success {
                cx.defer(move |cx| on_success(cx));
            }
            return;
        }
        let actions = Self::actions(cx);
        if playlist_id.is_empty() || member_user_id.is_empty() || actions.read(cx).remove_playlist_member_sai.is_pending()
        {
            return;
        }

        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        actions.update(cx, |actions, cx| {
            actions.remove_playlist_member_action(
                playlist_id,
                member_user_id,
                &client_request_id,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn leave_playlist_collaboration(
        &mut self,
        playlist_id: &str,
        on_success: Option<Rc<dyn Fn(&mut App)>>,
        on_error: Option<Rc<dyn Fn(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let actions = Self::actions(cx);
        if playlist_id.is_empty() || actions.read(cx).leave_playlist_sai.is_pending() {
            return;
        }

        let client_request_id = Uuid::new_v4().uuid_string().to_lowercase();
        actions.update(cx, |actions, cx| {
            actions.leave_playlist_action(playlist_id, &client_request_id, on_success, on_error, cx);
        });
    }

    pub fn start_playlist_listening_observation(
        &mut self,
        playlist_id: &str,
        track_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        Self::service(cx)
            .update(cx, |service, cx| service.start_playlist_listening_observation(playlist_id, track_id, cx));
    }

    pub fn stop_playlist_listening_observation(
        &mut self,
        playlist_id: &str,
        track_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        Self::service(cx)
            .update(cx, |service, cx| service.stop_playlist_listening_observation(playlist_id, track_id, cx));
    }

    pub fn refresh_playlist_listening_snapshot(
        &mut self,
        playlist_id: &str,
        track_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        Self::service(cx)
            .update(cx, |service, cx| service.refresh_playlist_listening_snapshot(playlist_id, track_id, cx));
    }

    pub fn update_playlist_listening_presence(
        &mut self,
        playlist_id: &str,
        track_id: &str,
        position_ms: i64,
        is_playing: bool,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        Self::service(cx).update(cx, |service, cx| {
            service.update_playlist_listening_presence(playlist_id, track_id, position_ms, is_playing, cx);
        });
    }

    pub fn stop_playlist_listening_presence(&mut self, playlist_id: &str, cx: &mut Context<Self>) {
        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            return;
        }
        Self::service(cx).update(cx, |service, cx| service.stop_playlist_listening_presence(playlist_id, true, cx));
    }

    pub fn join_playlist_listening_session(
        &mut self,
        playlist_id: &str,
        target_user_id: &str,
        expected_session_id: Option<&str>,
        on_success: Option<Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if playlist_id.is_empty() || target_user_id.is_empty() {
            if let Some(on_error) = on_error {
                cx.defer(move |cx| on_error(tr("music_live_action_failed"), cx));
            }
            return;
        }
        if !Self::has_wire_user(cx) {
            if let Some(on_error) = on_error {
                cx.defer(move |cx| on_error(tr("error_auth_required"), cx));
            }
            return;
        }
        if MusicActionsStore::slices(cx).join_playlist_listening_session_sai.is_pending() {
            if let Some(on_error) = on_error {
                cx.defer(move |cx| on_error(tr("music_live_transition_in_progress"), cx));
            }
            return;
        }

        if MusicGuideSandboxHooks::owns_playlist_id(playlist_id, cx) {
            match MusicGuideSandboxHooks::join_listening(playlist_id, cx) {
                Some(session) => {
                    if let Some(on_success) = on_success {
                        cx.defer(move |cx| on_success(session, cx));
                    }
                }
                None => {
                    if let Some(on_error) = on_error {
                        cx.defer(move |cx| on_error(tr("guide_sandbox_action_unavailable"), cx));
                    }
                }
            }
            return;
        }

        Self::service(cx).update(cx, |service, cx| {
            service.join_playlist_listening_session(
                playlist_id,
                target_user_id,
                expected_session_id,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn start_playlist_listening_session_sync(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if MusicGuideSandboxHooks::owns_session_id(session_id, cx) {
            return;
        }
        Self::service(cx).update(cx, |service, cx| service.start_playlist_listening_session_sync(session_id, cx));
    }

    pub fn stop_playlist_listening_session_sync(&mut self, session_id: &str, cx: &mut Context<Self>) {
        Self::service(cx).update(cx, |service, cx| service.stop_playlist_listening_session_sync(session_id, cx));
    }

    pub fn update_playlist_listening_session_state(
        &mut self,
        request: MusicPlaylistListeningSessionStateRequest,
        on_success: Option<Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_session_id(&request.session_id, cx) {
            let state = MusicGuideSandboxUpdateSessionState {
                track_id: request.track_id,
                position_ms: request.position_ms,
                is_playing: request.is_playing,
                shuffle_enabled: request.shuffle_enabled,
                shuffle_seed: request.shuffle_seed,
                repeat_mode: request.repeat_mode,
                audio_effect_enabled: request.audio_effect_enabled,
                audio_effect_rate_milli: request.audio_effect_rate_milli,
                audio_effects: request.audio_effects,
            };
            match MusicGuideSandboxHooks::update_session_state(state, cx) {
                Some(updated) => {
                    if let Some(on_success) = on_success {
                        cx.defer(move |cx| on_success(updated, cx));
                    }
                }
                None => {
                    if let Some(on_error) = on_error {
                        cx.defer(move |cx| on_error(tr("guide_sandbox_action_unavailable"), cx));
                    }
                }
            }
            return;
        }
        Self::service(cx).update(cx, |service, cx| {
            service.send_playlist_listening_session_state(
                &request.session_id,
                request.command,
                &request.track_id,
                request.position_ms,
                request.is_playing,
                request.shuffle_enabled,
                request.shuffle_seed,
                &request.repeat_mode,
                request.audio_effect_enabled,
                request.audio_effect_rate_milli,
                request.audio_effects,
                request.expected_revision,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn set_audio_effect(&mut self, kind: MusicAudioEffectKind, enabled: bool, cx: &mut Context<Self>) {
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_audio_effect(kind, enabled, cx));
    }

    pub fn reset_or_restore_audio_effects(&mut self, cx: &mut Context<Self>) {
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.reset_or_restore_audio_effects(cx));
    }

    pub fn open_audio_effects(&mut self, cx: &mut Context<Self>) {
        let player = MusicPlayerState::shared(cx);
        if !player.read(cx).is_music_playback() {
            return;
        }
        player.update(cx, |player, cx| player.set_show_audio_effects_sheet(true, cx));
    }

    pub fn open_playlist_listeners(&mut self, cx: &mut Context<Self>) {
        let player = MusicPlayerState::shared(cx);
        if !player.update(cx, |player, cx| player.can_show_playlist_listeners(cx)) {
            return;
        }
        player.update(cx, |player, cx| {
            player.show_playlist_listeners_sheet = true;
            cx.notify();
        });
    }

    pub fn set_audio_effect_value(&mut self, kind: MusicAudioEffectKind, value: f64, cx: &mut Context<Self>) {
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.set_audio_effect_value(kind, value, cx));
    }

    pub fn report_blocked_audio_effect_control(&mut self, cx: &mut Context<Self>) {
        MusicPlayerState::shared(cx).update(cx, |player, cx| player.report_blocked_audio_effect_control(cx));
    }

    pub fn request_playlist_listening_permissions(
        &mut self,
        session_id: &str,
        permissions: MusicListeningParticipantPermissions,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_session_id(session_id, cx) {
            if let Some(on_success) = on_success {
                cx.defer(on_success);
            }
            return;
        }
        Self::service(cx).update(cx, |service, cx| {
            service.request_playlist_listening_permissions(session_id, permissions, on_success, on_error, cx);
        });
    }

    pub fn update_playlist_listening_participant_permissions(
        &mut self,
        session_id: &str,
        participant_user_id: &str,
        permissions: MusicListeningParticipantPermissions,
        on_success: Option<Box<dyn FnOnce(MusicPlaylistListeningSession, &mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_session_id(session_id, cx) {
            match MusicGuideSandboxHooks::update_participant_permissions(participant_user_id, permissions, cx) {
                Some(updated) => {
                    if let Some(on_success) = on_success {
                        cx.defer(move |cx| on_success(updated, cx));
                    }
                }
                None => {
                    if let Some(on_error) = on_error {
                        cx.defer(move |cx| on_error(tr("guide_sandbox_action_unavailable"), cx));
                    }
                }
            }
            return;
        }
        Self::service(cx).update(cx, |service, cx| {
            service.update_playlist_listening_participant_permissions(
                session_id,
                participant_user_id,
                permissions,
                on_success,
                on_error,
                cx,
            );
        });
    }

    pub fn leave_playlist_listening_session(
        &mut self,
        session_id: &str,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_session_id(session_id, cx) {
            MusicGuideSandboxHooks::session_ended(cx);
            if let Some(on_success) = on_success {
                cx.defer(on_success);
            }
            return;
        }
        let is_pending = MusicActionsStore::slices(cx).leave_playlist_listening_session_sai.is_pending();
        if let Some(error) = self.listening_session_exit_error(session_id, is_pending, cx) {
            if let Some(on_error) = on_error {
                cx.defer(move |cx| on_error(error, cx));
            }
            return;
        }

        Self::service(cx).update(cx, |service, cx| {
            service.leave_playlist_listening_session(session_id, on_success, on_error, cx);
        });
    }

    pub fn end_playlist_listening_session(
        &mut self,
        session_id: &str,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_error: Option<Box<dyn FnOnce(String, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if MusicGuideSandboxHooks::owns_session_id(session_id, cx) {
            MusicGuideSandboxHooks::session_ended(cx);
            if let Some(on_success) = on_success {
                cx.defer(on_success);
            }
            return;
        }
        let is_pending = MusicActionsStore::slices(cx).end_playlist_listening_session_sai.is_pending();
        if let Some(error) = self.listening_session_exit_error(session_id, is_pending, cx) {
            if let Some(on_error) = on_error {
                cx.defer(move |cx| on_error(error, cx));
            }
            return;
        }

        Self::service(cx).update(cx, |service, cx| {
            service.end_playlist_listening_session(session_id, on_success, on_error, cx);
        });
    }

    /// the three guards leave and end share, same order as the other client
    fn listening_session_exit_error(&self, session_id: &str, is_pending: bool, cx: &mut App) -> Option<String> {
        if session_id.is_empty() {
            return Some(tr("music_live_action_failed"));
        }
        if !Self::has_wire_user(cx) {
            return Some(tr("error_auth_required"));
        }
        if is_pending {
            return Some(tr("music_live_transition_in_progress"));
        }
        None
    }
}
