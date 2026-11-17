use std::collections::HashSet;
use std::sync::LazyLock;

use rise_core::foundation::foundation_character::characters;
use rise_core::foundation::foundation_text::{character_count, character_prefix, drop_first_characters};
use rise_core::foundation::foundation_url::{Url, UrlAllowedCharacters, UrlComponents, adding_percent_encoding};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::file_manager::FileManager;

use super::music_media_url::MusicMediaURL;
use super::music_models::TrackModel;
use crate::core::stores::http::http_service::HttpService;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;

const LEGACY_MUSIC_MARKER: &str = "/public/music/";
const UUID_STRING_LENGTH: usize = 36;
const LEGACY_SEGMENT_MIN_LENGTH: usize = 38;
const SAFE_EXTENSION_MAX_LENGTH: usize = 8;

static PATH_ENCODING_ALLOWED: LazyLock<UrlAllowedCharacters> = LazyLock::new(|| {
    UrlAllowedCharacters::characters_in("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
});

pub enum MusicPlaybackUrl {}

impl MusicPlaybackUrl {
    pub fn path_encoding_allowed() -> UrlAllowedCharacters {
        *PATH_ENCODING_ALLOWED
    }

    pub fn legacy_music_safe_url(raw: &str) -> Option<String> {
        let normalized = RemoteMediaURL::optional_normalized(Some(raw))?;
        let marker_start = normalized.find(LEGACY_MUSIC_MARKER)?;
        let marker_end = marker_start + LEGACY_MUSIC_MARKER.len();
        let after_marker = &normalized[marker_end..];
        let segment_end = after_marker.find(&['?', '#'][..]).unwrap_or(after_marker.len());
        let segment = &after_marker[..segment_end];
        if character_count(segment) < LEGACY_SEGMENT_MIN_LENGTH {
            return None;
        }
        let uuid = character_prefix(segment, UUID_STRING_LENGTH);
        Uuid::from_uuid_string(uuid)?;
        let rest = drop_first_characters(segment, UUID_STRING_LENGTH);
        let ext = if rest.starts_with('.') {
            drop_first_characters(rest, 1)
        } else if rest.starts_with('-') {
            let dot = segment.rfind('.')?;
            &segment[dot + 1..]
        } else {
            return None;
        };
        let safe_ext: String = characters(ext)
            .filter(|character| character.is_ascii() && (character.is_letter() || character.is_number()))
            .take(SAFE_EXTENSION_MAX_LENGTH)
            .map(|character| character.as_str())
            .collect();
        if safe_ext.is_empty() {
            return None;
        }
        let canonical = format!("{}{uuid}.{safe_ext}", &normalized[..marker_end]);
        if canonical == normalized {
            return None;
        }
        Some(canonical)
    }

    pub fn playback_url_candidates_from(raw: &str) -> Vec<Url> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut urls: Vec<Url> = Vec::new();
        let mut add = |value: &str| {
            let Some(url) = Self::playback_url(value) else {
                return;
            };
            if seen.insert(url.absolute_string().to_owned()) {
                urls.push(url);
            }
        };
        add(raw);
        if let Some(legacy) = Self::legacy_music_safe_url(raw) {
            add(&legacy);
        }
        urls
    }

    pub fn playback_url_candidates_for(track: &TrackModel) -> Vec<Url> {
        let mut seen: HashSet<String> = HashSet::new();
        [track.audio_url.as_str(), track.remote_audio_url.as_deref().unwrap_or_default()]
            .into_iter()
            .flat_map(Self::playback_url_candidates_from)
            .filter(|url| seen.insert(url.absolute_string().to_owned()))
            .collect()
    }

    pub fn playback_url(raw: &str) -> Option<Url> {
        let normalized = RemoteMediaURL::optional_normalized(Some(raw))?;
        let trimmed = normalized.as_str();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.starts_with('/') {
            let local = Self::file_url_with_path(trimmed);
            if let Some(local) = local.filter(|local| local.to_file_path().is_some_and(|path| FileManager.file_exists(&path))) {
                return Some(local);
            }
            return MusicMediaURL::trusted_private_music_url(trimmed, &HttpService::resolved_api_base_url());
        }
        if let Some(local) = Url::from_string(trimmed).filter(Url::is_file_url) {
            return local.to_file_path().is_some_and(|path| FileManager.file_exists(&path)).then_some(local);
        }
        let scheme_start = trimmed.find("://")?;
        let scheme = trimmed[..scheme_start].to_lowercase();
        if !(scheme == "http" || scheme == "https") {
            return None;
        }
        let after_scheme = &trimmed[scheme_start + "://".len()..];
        let Some(path_start) = after_scheme.find('/') else {
            return Url::from_string_encoding_invalid_characters(trimmed, true);
        };
        let host = &after_scheme[..path_start];
        let path = &after_scheme[path_start..];
        let encoded_path = Self::encode_path_segments(path);
        Url::from_string(&format!("{scheme}://{host}{encoded_path}"))
    }

    fn encode_path_segments(path: &str) -> String {
        path.split('/')
            .map(|segment| {
                if segment.is_empty() {
                    return String::new();
                }
                adding_percent_encoding(segment, Self::path_encoding_allowed())
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    fn file_url_with_path(path: &str) -> Option<Url> {
        let mut components = UrlComponents::new();
        components.set_scheme(Some("file"));
        components.set_host(Some(""));
        components.set_path(path);
        components.url()
    }
}
