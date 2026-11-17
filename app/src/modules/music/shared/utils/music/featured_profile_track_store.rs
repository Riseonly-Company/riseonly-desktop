use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_canon::{RiseLimits, RiseRequestJSON, rise_proto, rise_request, rise_response, rise_wire_methods};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{
    FEATURED_TRACK_ACCOUNT_SCOPE, PROFILE_FEATURED_TRACK_LEGACY, PROFILE_FEATURED_TRACKS, PROFILE_FEATURED_TRACKS_SERVER,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiTypedSlice;
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWs, SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsRequestOptions, SaiWsRequestOptionsInit};
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::music::stores::music_actions::music_wire_types::ProtoTrackRecordExt;
use crate::modules::music::shared::utils::music::music_models::{MusicLocalTrackTransferPhase, TrackModel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeaturedProfileTrackAddResult {
    Added,
    AlreadyPinned,
    LimitReached,
    NotPinnable,
}

static LIMIT: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::client_value("profile_featured_tracks_limit")).unwrap_or(0));

pub struct FeaturedProfileTrackStore {
    tracks: Vec<TrackModel>,
    remote_tracks: HashMap<String, Vec<TrackModel>>,
    owner_user_id: String,
    needs_upload: bool,
    load_slice: Rc<SaiTypedSlice<rise_response::music::GetProfileTracks>>,
    save_slice: Rc<SaiTypedSlice<rise_response::music::SetProfileTracks>>,
    loaded_defaults_key: Option<String>,
}

impl FeaturedProfileTrackStore {
    pub fn limit() -> usize {
        *LIMIT
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |cx| {
            let mut store = Self {
                tracks: Vec::new(),
                remote_tracks: HashMap::new(),
                owner_user_id: String::new(),
                needs_upload: false,
                load_slice: SaiTypedSlice::new(cx),
                save_slice: SaiTypedSlice::new(cx),
                loaded_defaults_key: None,
            };
            store.load_account_state();
            store
        })
    }

    pub fn tracks(&self) -> &[TrackModel] {
        &self.tracks
    }

    pub fn tracks_for(&mut self, user_id: &str, cx: &mut App) -> Vec<TrackModel> {
        self.load_account_state();
        if self.owner_user_id == user_id {
            return self.tracks.clone();
        }
        if Self::is_current_account(user_id, cx) {
            return Vec::new();
        }
        self.remote_tracks.get(user_id).cloned().unwrap_or_default()
    }

    /// reads the pinned list of a profile from the server, own list too
    /// so a pin from another device shows up
    /// old local pins get uploaded once if the server list is empty
    pub fn load(&mut self, user_id: &str, cx: &mut Context<Self>) {
        self.load_account_state();
        let has_wire_user =
            AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().is_some_and(|id| !id.is_empty());
        if user_id.is_empty() || !has_wire_user {
            return;
        }
        let options = Self::request_options(rise_wire_methods::music::GET_PROFILE_TRACKS);
        let store = cx.entity().downgrade();
        let loaded_user_id = user_id.to_owned();
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let Some(response) = Self::decoded_payload::<rise_response::music::GetProfileTracks>(payload) else {
                return;
            };
            let (store, user_id) = (store.clone(), loaded_user_id.clone());
            cx.defer(move |cx| {
                let _ = store.update(cx, |store, cx| store.apply_loaded(&user_id, &response.tracks, cx));
            });
        });
        let on_error: SaiWsOnError = Rc::new(|error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("getProfileTracks error: {error}"), cx);
        });
        let body = rise_request::music::GetProfileTracks { profile_user_id: Some(user_id.to_owned()) };
        SaiWs::request_slice(&self.load_slice, body, options, Some(on_success), Some(on_error), cx);
    }

    fn apply_loaded(&mut self, user_id: &str, records: &[rise_proto::music::ProtoTrackRecord], cx: &mut Context<Self>) {
        let server = Self::models(records);
        if !Self::is_current_account(user_id, cx) {
            if self.remote_tracks.get(user_id) != Some(&server) {
                self.remote_tracks.insert(user_id.to_owned(), server);
                cx.notify();
            }
            return;
        }
        let owns_local = self.owner_user_id == user_id && !self.tracks.is_empty();
        if owns_local && (self.needs_upload || (server.is_empty() && !Self::uploaded_once())) {
            self.upload(cx);
            return;
        }
        self.owner_user_id = user_id.to_owned();
        if self.tracks != server {
            self.tracks = server;
            self.persist();
        }
        Self::mark_uploaded();
        cx.notify();
    }

    pub fn contains(&mut self, track_id: &str) -> bool {
        self.load_account_state();
        self.tracks.iter().any(|track| track.id == track_id)
    }

    /// a track that hasnt reached the server cant be pinned
    /// its file lives in the draft dir that vanishes with the upload and a temp_ id resolves nowhere
    pub fn add(&mut self, track: TrackModel, user_id: &str, cx: &mut Context<Self>) -> FeaturedProfileTrackAddResult {
        self.load_account_state();
        if user_id.is_empty() || track.id.is_empty() || track.transfer_phase.is_some() {
            return FeaturedProfileTrackAddResult::NotPinnable;
        }
        if self.owner_user_id != user_id {
            self.owner_user_id = user_id.to_owned();
            self.tracks.clear();
        }
        if let Some(index) = self.tracks.iter().position(|pinned| pinned.id == track.id) {
            self.tracks[index] = track;
            self.persist();
            cx.notify();
            return FeaturedProfileTrackAddResult::AlreadyPinned;
        }
        if self.tracks.len() >= Self::limit() {
            cx.notify();
            return FeaturedProfileTrackAddResult::LimitReached;
        }
        self.tracks.insert(0, track);
        self.persist();
        self.upload(cx);
        cx.notify();
        FeaturedProfileTrackAddResult::Added
    }

    pub fn guide_snapshot(&mut self) -> (String, Vec<TrackModel>) {
        self.load_account_state();
        (self.owner_user_id.clone(), self.tracks.clone())
    }

    pub fn restore(&mut self, user_id: String, previous: Vec<TrackModel>, cx: &mut Context<Self>) {
        self.load_account_state();
        if self.owner_user_id != user_id || self.tracks == previous {
            return;
        }
        let changed_ids = !self.tracks.iter().map(|track| &track.id).eq(previous.iter().map(|track| &track.id));
        self.tracks = previous;
        self.persist();
        if changed_ids {
            self.upload(cx);
        }
        cx.notify();
    }

    pub fn remove(&mut self, track_id: &str, cx: &mut Context<Self>) {
        self.load_account_state();
        if !self.tracks.iter().any(|track| track.id == track_id) {
            return;
        }
        self.tracks.retain(|track| track.id != track_id);
        self.persist();
        self.upload(cx);
        cx.notify();
    }

    pub fn reorder(&mut self, ids: &[String], cx: &mut Context<Self>) {
        self.load_account_state();
        let mut by_id: HashMap<&str, &TrackModel> = HashMap::new();
        for track in &self.tracks {
            by_id.entry(track.id.as_str()).or_insert(track);
        }
        let reordered: Vec<TrackModel> = ids
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).map(|track| (*track).clone()))
            .collect();
        if reordered.len() != self.tracks.len()
            || reordered.iter().map(|track| &track.id).eq(self.tracks.iter().map(|track| &track.id))
        {
            return;
        }
        self.tracks = reordered;
        self.persist();
        self.upload(cx);
        cx.notify();
    }

    /// sends the whole own list in order
    /// the answer replaces it only while the list is still the one sent so
    /// a quick second edit isnt undone by the first reply
    fn upload(&mut self, cx: &mut Context<Self>) {
        if self.owner_user_id.is_empty() || !Self::is_current_account(&self.owner_user_id, cx) {
            return;
        }
        self.needs_upload = true;
        let sent_ids: Rc<Vec<String>> = Rc::new(self.tracks.iter().map(|track| track.id.clone()).collect());
        let options = Self::request_options(rise_wire_methods::music::SET_PROFILE_TRACKS);
        let (success_store, error_store) = (cx.entity().downgrade(), cx.entity().downgrade());
        let (success_ids, error_ids) = (sent_ids.clone(), sent_ids.clone());
        let on_success: SaiWsOnSuccess = Rc::new(move |payload: Option<Box<dyn Any>>, cx: &mut App| {
            let response = Self::decoded_payload::<rise_response::music::SetProfileTracks>(payload);
            let (store, sent_ids) = (success_store.clone(), success_ids.clone());
            cx.defer(move |cx| {
                let _ = store.update(cx, |store, cx| {
                    if !store.has_track_ids(&sent_ids) {
                        return;
                    }
                    store.needs_upload = false;
                    Self::mark_uploaded();
                    let Some(response) = response else { return };
                    let server = Self::models(&response.tracks);
                    if server.iter().map(|track| &track.id).eq(sent_ids.iter()) {
                        return;
                    }
                    store.tracks = server;
                    store.persist();
                    cx.notify();
                });
            });
        });
        let on_error: SaiWsOnError = Rc::new(move |error: &anyhow::Error, cx: &mut App| {
            Self::log_network(format!("setProfileTracks error: {error}"), cx);
            if CanonErrors::is_connection_loss(error) || CanonErrors::is_maintenance_refusal(error) {
                return;
            }
            let (store, sent_ids) = (error_store.clone(), error_ids.clone());
            cx.defer(move |cx| {
                let _ = store.update(cx, |store, cx| {
                    if !store.has_track_ids(&sent_ids) {
                        return;
                    }
                    store.needs_upload = false;
                    Self::mark_uploaded();
                    let owner_user_id = store.owner_user_id.clone();
                    store.load(&owner_user_id, cx);
                });
            });
        });
        let body = rise_request::music::SetProfileTracks {
            track_ids: Some(sent_ids.iter().map(|id| RiseRequestJSON::String(id.clone())).collect()),
        };
        SaiWs::request_slice(&self.save_slice, body, options, Some(on_success), Some(on_error), cx);
    }

    fn has_track_ids(&self, ids: &[String]) -> bool {
        self.tracks.iter().map(|track| &track.id).eq(ids.iter())
    }

    fn is_current_account(user_id: &str, cx: &mut App) -> bool {
        !user_id.is_empty()
            && AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().as_deref() == Some(user_id)
    }

    fn models(records: &[rise_proto::music::ProtoTrackRecord]) -> Vec<TrackModel> {
        records.iter().filter_map(ProtoTrackRecordExt::as_track_model).take(Self::limit()).collect()
    }

    fn uploaded_once() -> bool {
        UserDefaults::standard().bool(&SaiAccountCacheScope::scoped_key(PROFILE_FEATURED_TRACKS_SERVER))
    }

    fn mark_uploaded() {
        if Self::uploaded_once() {
            return;
        }
        UserDefaults::standard().set(&SaiAccountCacheScope::scoped_key(PROFILE_FEATURED_TRACKS_SERVER), true);
    }

    fn request_options(method: &str) -> SaiWsRequestOptions {
        SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: rise_wire_methods::music::SERVICE.to_owned(),
            method: method.to_owned(),
            cache_key: None,
            reload_if_cached: true,
            allow_concurrent_while_pending: true,
            reflect_pending_in_ui: false,
            send_immediately: true,
            store_response_in_cache: false,
            ..SaiWsRequestOptionsInit::default()
        })
    }

    fn decoded_payload<T: DeserializeOwned + 'static>(payload: Option<Box<dyn Any>>) -> Option<T> {
        match payload?.downcast::<T>() {
            Ok(decoded) => Some(*decoded),
            Err(payload) => SaiJSONCodec::decode::<T>(payload.downcast_ref::<Value>()),
        }
    }

    fn log_network(text: String, cx: &mut App) {
        Sai::log("Music", move || text, SaiLogKind::Network, cx);
    }

    pub fn refresh_if_matching(&mut self, updated: &TrackModel, cx: &mut Context<Self>) {
        self.load_account_state();
        if self.owner_user_id.is_empty() || updated.transfer_phase.is_some() {
            return;
        }
        let Some(index) = self.tracks.iter().position(|track| track.id == updated.id) else {
            return;
        };
        if !Self::has_metadata_difference(&self.tracks[index], updated) {
            return;
        }
        self.tracks[index] = updated.clone();
        self.persist();
        cx.notify();
    }

    fn has_metadata_difference(lhs: &TrackModel, rhs: &TrackModel) -> bool {
        lhs.title != rhs.title
            || lhs.artist != rhs.artist
            || lhs.cover_image_url != rhs.cover_image_url
            || lhs.stable_audio_url() != rhs.stable_audio_url()
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.tracks.clear();
        self.remote_tracks.clear();
        self.needs_upload = false;
        self.owner_user_id.clear();
        self.loaded_defaults_key = None;
        cx.notify();
    }

    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        self.load_account_state();
        cx.notify();
    }

    pub fn remove_persisted_account(&self, user_id: &str) {
        let prefix = SaiAccountCacheScope::prefix(user_id);
        let defaults = UserDefaults::standard();
        defaults.remove_object(&(prefix.clone() + PROFILE_FEATURED_TRACKS));
        defaults.remove_object(&(prefix.clone() + PROFILE_FEATURED_TRACK_LEGACY));
        defaults.remove_object(&(prefix + PROFILE_FEATURED_TRACKS_SERVER));
    }

    fn load_account_state(&mut self) {
        Self::migrate_legacy_state_if_needed();
        let key = Self::defaults_key();
        if self.loaded_defaults_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.tracks.clear();
        self.remote_tracks.clear();
        self.needs_upload = false;
        self.owner_user_id.clear();
        self.loaded_defaults_key = Some(key.clone());

        if let Some(saved) = UserDefaults::standard()
            .data(&key)
            .and_then(|data| serde_json::from_slice::<SavedFeaturedTracks>(&data).ok())
        {
            self.owner_user_id = saved.owner_user_id;
            self.tracks = saved
                .tracks
                .iter()
                .filter_map(SavedTrack::track)
                .take(Self::limit())
                .collect();
            return;
        }
        let Some(legacy) = Self::take_legacy_saved_track() else {
            return;
        };
        let Some(track) = legacy.track() else {
            return;
        };
        self.owner_user_id = legacy.owner_user_id.unwrap_or_default();
        self.tracks = if self.owner_user_id.is_empty() { Vec::new() } else { vec![track] };
        self.persist();
    }

    fn defaults_key() -> String {
        SaiAccountCacheScope::scoped_key(PROFILE_FEATURED_TRACKS)
    }

    fn legacy_defaults_key() -> String {
        SaiAccountCacheScope::scoped_key(PROFILE_FEATURED_TRACK_LEGACY)
    }

    fn persist(&self) {
        let key = Self::defaults_key();
        let defaults = UserDefaults::standard();
        if self.tracks.is_empty() || self.owner_user_id.is_empty() {
            defaults.remove_object(&key);
            return;
        }
        let saved = SavedFeaturedTracks {
            owner_user_id: self.owner_user_id.clone(),
            tracks: self.tracks.iter().map(SavedTrack::new).collect(),
        };
        let Ok(data) = serde_json::to_vec(&saved) else {
            return;
        };
        defaults.set(&key, data);
    }

    fn take_legacy_saved_track() -> Option<SavedTrack> {
        let defaults = UserDefaults::standard();
        let key = Self::legacy_defaults_key();
        let data = defaults.data(&key)?;
        defaults.remove_object(&key);
        serde_json::from_slice::<SavedTrack>(&data).ok()
    }

    fn migrate_legacy_state_if_needed() {
        let defaults = UserDefaults::standard();
        if SaiAccountCacheScope::active_user_id().is_none() || defaults.bool(FEATURED_TRACK_ACCOUNT_SCOPE) {
            return;
        }
        let legacy_key = Self::legacy_defaults_key();
        if defaults.object(&legacy_key).is_none()
            && let Some(legacy) = defaults.data(PROFILE_FEATURED_TRACK_LEGACY)
        {
            defaults.set(&legacy_key, legacy);
        }
        defaults.remove_object(PROFILE_FEATURED_TRACK_LEGACY);
        defaults.set(FEATURED_TRACK_ACCOUNT_SCOPE, true);
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedFeaturedTracks {
    owner_user_id: String,
    tracks: Vec<SavedTrack>,
}

/// field names and optional omission follow the swift codable record
/// so a file written by either side reads on the other
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedTrack {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_user_id: Option<String>,
    title: String,
    artist: String,
    duration_seconds: i64,
    cover_image_url: String,
    audio_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_message_id: Option<String>,
    artist_is_registered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    library_owner_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transfer_phase: Option<String>,
}

impl SavedTrack {
    fn new(track: &TrackModel) -> Self {
        Self {
            id: track.id.clone(),
            owner_user_id: None,
            title: track.title.clone(),
            artist: track.artist.clone(),
            duration_seconds: track.duration_seconds,
            cover_image_url: track.cover_image_url.clone(),
            audio_url: track.stable_audio_url(),
            source_chat_id: track.source_chat_id.clone(),
            source_message_id: track.source_message_id.clone(),
            artist_is_registered: track.artist_is_registered,
            library_owner_user_id: track.library_owner_user_id.clone(),
            status: Some(track.status.clone()),
            transfer_phase: track
                .transfer_phase
                .as_ref()
                .map(MusicLocalTrackTransferPhase::raw_value)
                .map(str::to_owned),
        }
    }

    /// a record whose upload never landed isnt restored
    /// status and transferPhase werent persisted before so a pinned
    /// draft looked like a normal track after relaunch
    fn track(&self) -> Option<TrackModel> {
        if self.id.is_empty() || self.transfer_phase.is_some() {
            return None;
        }
        Some(TrackModel {
            id: self.id.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            artist_id: String::new(),
            album_title: String::new(),
            album_id: String::new(),
            duration_seconds: self.duration_seconds,
            cover_colors: vec!["#25242D".to_owned(), "#111116".to_owned()],
            cover_image_url: self.cover_image_url.clone(),
            audio_url: self.audio_url.clone(),
            is_exclusive: false,
            status: self.status.clone().unwrap_or_default(),
            description: None,
            source_chat_id: self.source_chat_id.clone(),
            source_message_id: self.source_message_id.clone(),
            artist_is_registered: self.artist_is_registered,
            remote_audio_url: Some(self.audio_url.clone()),
            transfer_phase: None,
            transferred_bytes: 0,
            transfer_total_bytes: 0,
            transfer_error: None,
            library_owner_user_id: self.library_owner_user_id.clone(),
            playback_cache_id: None,
            is_liked: false,
        })
    }
}
