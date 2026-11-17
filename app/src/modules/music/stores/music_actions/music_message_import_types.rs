use std::fmt;
use std::hash::{Hash, Hasher};

use rise_canon::RiseLimits;
use rise_core::foundation::foundation_character::characters;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;

use super::music_types::MusicLocalPlaylistDestination;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::modules::music::shared::music_field_rules::MusicFieldRules;

pub enum MusicMessageMP3ImportLimits {}

impl MusicMessageMP3ImportLimits {
    pub fn description_utf16_count() -> i64 {
        RiseLimits::client_value("music_message_m_p3_import_limits_description_u_t_f16_count")
    }

    pub fn clamped(value: &str, max_utf16_count: i64) -> String {
        let utf16_count = |text: &str| i64::try_from(text.encode_utf16().count()).unwrap_or(i64::MAX);
        if utf16_count(value) <= max_utf16_count {
            return value.to_owned();
        }
        let mut result = String::new();
        let mut used_utf16_count: i64 = 0;
        for character in characters(value) {
            let character_utf16_count = utf16_count(character.as_str());
            if used_utf16_count + character_utf16_count > max_utf16_count {
                break;
            }
            result.push_str(character.as_str());
            used_utf16_count += character_utf16_count;
        }
        result
    }

    pub fn title(value: &str) -> String {
        MusicFieldRules::local_track_title().clamped(value)
    }

    pub fn artist(value: &str) -> String {
        MusicFieldRules::local_track_artist().clamped(value)
    }

    pub fn description(value: &str) -> String {
        Self::clamped(value, Self::description_utf16_count())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MusicMessageMP3ImportSource {
    pub message_id: String,
    pub chat_id: String,
    pub media_id: String,
    pub remote_url: String,
    pub local_preview_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub mime_type: Option<String>,
    pub duration_seconds: Option<TimeInterval>,
    pub caption: String,
    pub suggested_title: String,
    pub suggested_artist: String,
}

impl MusicMessageMP3ImportSource {
    pub fn id(&self) -> String {
        format!("{}:{}:{}", self.chat_id, self.message_id, self.media_id)
    }
}

impl Eq for MusicMessageMP3ImportSource {}

impl Hash for MusicMessageMP3ImportSource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.message_id.hash(state);
        self.chat_id.hash(state);
        self.media_id.hash(state);
        self.remote_url.hash(state);
        self.local_preview_url.hash(state);
        self.thumbnail_url.hash(state);
        self.file_name.hash(state);
        self.file_size.hash(state);
        self.mime_type.hash(state);
        self.duration_seconds.map(f64::to_bits).hash(state);
        self.caption.hash(state);
        self.suggested_title.hash(state);
        self.suggested_artist.hash(state);
    }
}

#[derive(Clone, Debug)]
pub struct MusicMessageMP3ImportRequest {
    pub request_id: String,
    pub temp_id: String,
    pub source: MusicMessageMP3ImportSource,
    pub destination: MusicLocalPlaylistDestination,
    pub title: String,
    pub description: Option<String>,
    pub artist_name: String,
    pub replacement_cover_file: Option<SaiWsOptimisticLocalFile>,
}

impl MusicMessageMP3ImportRequest {
    pub fn new(
        source: MusicMessageMP3ImportSource,
        destination: MusicLocalPlaylistDestination,
        title: &str,
        description: Option<&str>,
        artist_name: &str,
        replacement_cover_file: Option<SaiWsOptimisticLocalFile>,
    ) -> Self {
        Self::new_with_ids(
            Uuid::new_v4().uuid_string().to_lowercase(),
            format!("temp_message_mp3_{}", Uuid::new_v4().uuid_string()),
            source,
            destination,
            title,
            description,
            artist_name,
            replacement_cover_file,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_ids(
        request_id: String,
        temp_id: String,
        source: MusicMessageMP3ImportSource,
        destination: MusicLocalPlaylistDestination,
        title: &str,
        description: Option<&str>,
        artist_name: &str,
        replacement_cover_file: Option<SaiWsOptimisticLocalFile>,
    ) -> Self {
        Self {
            request_id,
            temp_id,
            source,
            destination,
            title: MusicMessageMP3ImportLimits::title(title),
            description: description.map(MusicMessageMP3ImportLimits::description),
            artist_name: MusicMessageMP3ImportLimits::artist(artist_name),
            replacement_cover_file,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MusicMessageMP3ImportJob {
    pub request: MusicMessageMP3ImportRequest,
    pub account_user_id: String,
    pub account_epoch: String,
}

#[derive(Clone, Debug)]
pub struct MusicMessageMP3PendingCompletion {
    pub playlist_id: String,
    pub playlist_title: String,
    pub temp_id: String,
    pub created_at: Date,
}

#[derive(Clone, Debug)]
pub struct MusicMessageMP3ProgressPersistence {
    pub phase: String,
    pub completed_bytes: i64,
    pub persisted_at: TimeInterval,
}

#[derive(Clone, Debug)]
pub enum MusicMessageMP3CreateOutcome {
    Success,
    Failure(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MusicMessageMP3ImportError {
    AccountChanged,
    InvalidSource,
    DownloadFailed(String),
    FileTooLarge,
    Mp3Required,
    InvalidDuration,
}

impl MusicMessageMP3ImportError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::AccountChanged => tr("music_message_import_account_changed"),
            Self::InvalidSource => tr("music_message_import_invalid_source"),
            Self::DownloadFailed(detail) => {
                if detail.is_empty() {
                    tr("music_message_import_download_failed")
                } else {
                    detail.clone()
                }
            }
            Self::FileTooLarge => tr("music_local_upload_too_large"),
            Self::Mp3Required => tr("music_local_upload_mp3_required"),
            Self::InvalidDuration => tr("music_upload_invalid_duration"),
        })
    }
}

impl fmt::Display for MusicMessageMP3ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for MusicMessageMP3ImportError {}
