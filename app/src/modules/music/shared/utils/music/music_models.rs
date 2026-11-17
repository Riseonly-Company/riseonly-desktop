use std::hash::{Hash, Hasher};

use rise_catalog::{MusicGenre, MusicTrackStatus};
use rise_core::foundation::foundation_character::characters;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{tr, tr_plural};
use rise_music::{PlaylistVisibility, REPEAT_MODES, RepeatMode};
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_format::RiseFormat;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MusicLocalTrackTransferPhase {
    #[serde(rename = "queued")]
    Queued,
    #[serde(rename = "downloading")]
    Downloading,
    #[serde(rename = "preparing")]
    Preparing,
    #[serde(rename = "uploading")]
    Uploading,
    #[serde(rename = "waiting_for_connection")]
    WaitingForConnection,
    #[serde(rename = "failed")]
    Failed,
}

impl MusicLocalTrackTransferPhase {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Preparing => "preparing",
            Self::Uploading => "uploading",
            Self::WaitingForConnection => "waiting_for_connection",
            Self::Failed => "failed",
        }
    }

    pub fn from_raw(raw_value: &str) -> Option<Self> {
        match raw_value {
            "queued" => Some(Self::Queued),
            "downloading" => Some(Self::Downloading),
            "preparing" => Some(Self::Preparing),
            "uploading" => Some(Self::Uploading),
            "waiting_for_connection" => Some(Self::WaitingForConnection),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// gives (completed_bytes, total_bytes) or none
    /// none if the update goes back a phase or revives a failed transfer
    pub fn reconciled_progress_update(
        current_phase: Option<Self>,
        next_phase: Self,
        current_bytes: i64,
        current_total_bytes: i64,
        incoming_bytes: i64,
        incoming_total_bytes: i64,
    ) -> Option<(i64, i64)> {
        if let Some(current_phase) = current_phase {
            if current_phase == Self::Failed && next_phase != Self::Failed {
                return None;
            }
            if !(next_phase == Self::Failed || next_phase.progress_order() >= current_phase.progress_order()) {
                return None;
            }
        }

        let same_phase = current_phase == Some(next_phase);
        let total = if same_phase {
            0_i64.max(current_total_bytes.max(incoming_total_bytes))
        } else {
            0_i64.max(incoming_total_bytes)
        };
        let completed = if same_phase {
            0_i64.max(current_bytes.max(incoming_bytes))
        } else {
            0_i64.max(incoming_bytes)
        };
        Some((if total > 0 { completed.min(total) } else { completed }, total))
    }

    fn progress_order(&self) -> i64 {
        match self {
            Self::Queued => 0,
            Self::Downloading => 1,
            Self::Preparing => 2,
            Self::Uploading => 3,
            Self::WaitingForConnection => 4,
            Self::Failed => 5,
        }
    }
}

/// equality is memberwise since rows diff inputs with ==, hash uses id only
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackModel {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: String,
    pub album_title: String,
    pub album_id: String,
    pub duration_seconds: i64,
    pub cover_colors: Vec<String>,
    pub cover_image_url: String,
    pub audio_url: String,
    pub is_exclusive: bool,
    pub status: String,
    pub description: Option<String>,
    pub source_chat_id: Option<String>,
    pub source_message_id: Option<String>,
    pub artist_is_registered: bool,
    pub remote_audio_url: Option<String>,
    pub transfer_phase: Option<MusicLocalTrackTransferPhase>,
    pub transferred_bytes: i64,
    pub transfer_total_bytes: i64,
    pub transfer_error: Option<String>,
    pub library_owner_user_id: Option<String>,
    pub playback_cache_id: Option<String>,
    pub is_liked: bool,
}

impl TrackModel {
    pub fn playback_cache_key(&self) -> String {
        let trimmed = self
            .playback_cache_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or_default();
        if trimmed.is_empty() {
            return self.id.clone();
        }
        trimmed.to_owned()
    }

    pub fn stable_audio_url(&self) -> String {
        let remote = self
            .remote_audio_url
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or_default();
        if remote.is_empty() {
            self.audio_url.clone()
        } else {
            remote.to_owned()
        }
    }

    pub fn formatted_duration(&self) -> String {
        RiseFormat::duration(self.duration_seconds)
    }

    pub fn is_local_transfer_active(&self) -> bool {
        match self.transfer_phase {
            Some(MusicLocalTrackTransferPhase::Queued)
            | Some(MusicLocalTrackTransferPhase::Downloading)
            | Some(MusicLocalTrackTransferPhase::Preparing)
            | Some(MusicLocalTrackTransferPhase::Uploading) => true,
            Some(MusicLocalTrackTransferPhase::WaitingForConnection)
            | Some(MusicLocalTrackTransferPhase::Failed)
            | None => false,
        }
    }

    pub fn is_device_only_draft(&self) -> bool {
        self.transfer_phase.is_some() && self.source_chat_id.is_none()
    }

    /// chat mp3 import can still be dropped before the server upload starts
    pub fn can_remove_pending_local_track(&self) -> bool {
        match self.transfer_phase {
            Some(MusicLocalTrackTransferPhase::WaitingForConnection) | Some(MusicLocalTrackTransferPhase::Failed) => {
                true
            }
            Some(MusicLocalTrackTransferPhase::Queued)
            | Some(MusicLocalTrackTransferPhase::Downloading)
            | Some(MusicLocalTrackTransferPhase::Preparing) => !self.is_device_only_draft(),
            Some(MusicLocalTrackTransferPhase::Uploading) | None => false,
        }
    }

    pub fn normalized_transfer_progress(&self) -> f64 {
        if self.transfer_total_bytes <= 0 {
            return 0.0;
        }
        (self.transferred_bytes as f64 / self.transfer_total_bytes as f64).clamp(0.0, 1.0)
    }
}

impl Hash for TrackModel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlbumModel {
    pub id: String,
    pub title: String,
    pub artist_id: String,
    pub artist_name: String,
    pub year: i64,
    pub cover_colors: Vec<String>,
    pub cover_image_url: String,
    pub tracks: Vec<TrackModel>,
    pub status: String,
    pub scheduled_at_ms: Option<i64>,
}

impl AlbumModel {
    pub fn is_scheduled(&self) -> bool {
        self.status == MusicTrackStatus::Scheduled.wire()
    }

    pub fn scheduled_date(&self) -> Option<Date> {
        let ms = self.scheduled_at_ms?;
        if ms <= 0 {
            return None;
        }
        Some(Date::from_time_interval_since_1970(ms as f64 / 1000.0))
    }
}

impl Hash for AlbumModel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// server makes one default local playlist per account with an english name and is_default_local
/// shown name is the client one, flag drops on rename so a typed name never gets translated
pub enum MusicDefaultLocalPlaylist {}

impl MusicDefaultLocalPlaylist {
    /// title with the default translate for the default local playlist
    pub fn resolved_title(title: Option<&str>, is_default_local: Option<bool>) -> String {
        Self::resolved_title_with(title, is_default_local, &tr)
    }

    pub fn resolved_title_with(
        title: Option<&str>,
        is_default_local: Option<bool>,
        translate: &dyn Fn(&str) -> String,
    ) -> String {
        if is_default_local == Some(true) {
            translate("music_default_local_playlist_title")
        } else {
            title.unwrap_or_default().to_owned()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistModel {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub cover_colors: Vec<String>,
    pub cover_image_url: String,
    pub tracks: Vec<TrackModel>,
    pub visibility: String,
    pub owner_user_id: String,
}

impl PlaylistModel {
    pub fn list_subtitle(&self) -> String {
        let vis_label = match PlaylistVisibility::from_wire(&self.visibility) {
            Some(PlaylistVisibility::PublicPlaylist) => tr("music_visibility_public"),
            Some(PlaylistVisibility::LocalPlaylist) => tr("music_visibility_local"),
            Some(PlaylistVisibility::PrivatePlaylist) | None => tr("music_visibility_private"),
        };
        let count = i64::try_from(self.tracks.len()).unwrap_or(i64::MAX);
        format!("{vis_label} \u{2022} {}", tr_plural("music_track_count", count))
    }
}

impl Hash for PlaylistModel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtistModel {
    pub id: String,
    pub name: String,
    pub cover_colors: Vec<String>,
    pub cover_image_url: String,
    pub monthly_listeners: i64,
    pub genres: Vec<String>,
    pub top_track_ids: Vec<String>,
    pub album_ids: Vec<String>,
}

impl Hash for ArtistModel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// canon calls the off position off, old peers may still send none and the alias table reads it
pub type MusicRepeatMode = RepeatMode;

/// extension methods for the repeat mode, next lives on the canon type already
pub trait MusicRepeatModeExt: Sized {
    fn from_wire(value: &str) -> Self;
    fn icon(self) -> &'static str;
    fn accessibility_value(self) -> String;
}

impl MusicRepeatModeExt for MusicRepeatMode {
    fn from_wire(value: &str) -> Self {
        let normalized = value.to_lowercase();
        REPEAT_MODES
            .iter()
            .find(|spec| spec.mode.wire() == normalized || spec.aliases.iter().any(|alias| *alias == normalized))
            .map(|spec| spec.mode)
            .unwrap_or(RepeatMode::Off)
    }

    fn icon(self) -> &'static str {
        match self {
            RepeatMode::Off => "repeat",
            RepeatMode::One => "repeat.1",
            RepeatMode::All => "repeat",
        }
    }

    fn accessibility_value(self) -> String {
        match self {
            RepeatMode::Off => tr("music_player_repeat_off"),
            RepeatMode::One => tr("music_player_repeat_one"),
            RepeatMode::All => tr("music_player_repeat_all"),
        }
    }
}

/// canon genre for a stored value, none if unknown
/// old records have free spellings like Pop or Hip-Hop
pub fn canon_genre(raw: &str) -> Option<MusicGenre> {
    if let Some(exact) = MusicGenre::from_wire(raw) {
        return Some(exact);
    }
    let fold = |value: &str| -> String {
        characters(&value.to_lowercase())
            .filter(|character| character.is_letter() || character.is_number())
            .map(|character| character.as_str().to_owned())
            .collect::<String>()
    };
    let needle = fold(raw);
    if needle.is_empty() {
        return None;
    }
    MusicGenre::ALL
        .into_iter()
        .find(|genre| fold(genre.wire()) == needle || fold(&tr(genre.i18n_key())) == needle)
}

/// localized genre title, unknown wire values are shown as sent
pub fn display_genre(wire: &str) -> String {
    canon_genre(wire).map_or_else(|| wire.to_owned(), |genre| tr(genre.i18n_key()))
}
