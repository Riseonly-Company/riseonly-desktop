use std::collections::HashSet;

use rise_core::foundation::foundation_character::{characters, scalar_is_control_character};
use rise_core::foundation::foundation_text::{
    character_count, character_prefix, drop_first_characters, trimming_whitespaces_and_newlines,
};
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::stores::http::http_service::HttpService;
use crate::modules::chat::components::chat::composer::remote_media_url_type_hooks::RemoteMediaURL;

const PRIVATE_MUSIC_PATH_MAX_UTF8: usize = 1_024;
const LEGACY_COVER_MARKER: &str = "/public/music/";
const UUID_STRING_LENGTH: usize = 36;
const LEGACY_SEGMENT_MIN_LENGTH: usize = 38;
const SAFE_EXTENSION_MAX_LENGTH: usize = 8;
const TRUSTED_FILE_SUFFIXES: [&str; 5] = [".mp3", ".jpg", ".jpeg", ".png", ".webp"];

pub enum MusicMediaURL {}

impl MusicMediaURL {
    pub fn cover_display_source(raw: Option<&str>) -> String {
        Self::resolved_url_string(raw).unwrap_or_default()
    }

    pub fn cover_url_strings_for_fetch(raw: Option<&str>) -> Vec<String> {
        Self::cover_url_strings_for_fetch_api_base_url(raw, &HttpService::resolved_api_base_url())
    }

    pub fn resolved_url_string(raw: Option<&str>) -> Option<String> {
        Self::resolved_url_string_api_base_url(raw, &HttpService::resolved_api_base_url())
    }

    pub fn cover_url_strings_for_fetch_api_base_url(raw: Option<&str>, api_base_url: &Url) -> Vec<String> {
        let Some(primary) = Self::resolved_url_string_api_base_url(raw, api_base_url) else {
            return Vec::new();
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        let mut add = |value: String| {
            if seen.insert(value.clone()) {
                out.push(value);
            }
        };
        let alternate = Self::legacy_cover_alternate_url(&primary);
        add(primary);
        if let Some(alt) = alternate {
            add(alt);
        }
        out
    }

    pub fn resolved_url_string_api_base_url(raw: Option<&str>, api_base_url: &Url) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw?);
        if value.is_empty() {
            return None;
        }

        if value.starts_with('/') {
            return Self::trusted_private_music_url(value, api_base_url).map(|url| url.absolute_string().to_owned());
        }

        let normalized = RemoteMediaURL::optional_normalized(Some(value))?;
        let components = UrlComponents::from_string(&normalized)?;
        let scheme = components.scheme()?.to_lowercase();
        if scheme == "file" {
            return components.url().map(|url| url.absolute_string().to_owned());
        }
        if !(scheme == "http" || scheme == "https")
            || components.host().is_none()
            || components.user().is_some()
            || components.password().is_some()
        {
            return None;
        }
        components.url().map(|url| url.absolute_string().to_owned())
    }

    pub fn trusted_private_music_url(path: &str, api_base_url: &Url) -> Option<Url> {
        if path.len() > PRIVATE_MUSIC_PATH_MAX_UTF8
            || path != trimming_whitespaces_and_newlines(path)
            || path.contains("..")
            || path.contains('%')
            || path.contains('?')
            || path.contains('#')
            || path.contains('\\')
            || path.chars().any(scalar_is_control_character)
        {
            return None;
        }

        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 7
            || !parts[0].is_empty()
            || parts[1] != "api"
            || parts[2] != "file"
            || parts[3] != "secure"
            || parts[4] != "music"
        {
            return None;
        }

        let owner = parts[5];
        if owner.len() != UUID_STRING_LENGTH {
            return None;
        }
        let owner_id = Uuid::from_uuid_string(owner)?;
        if owner_id.uuid_string().to_lowercase() != owner {
            return None;
        }

        let filename = parts[6];
        if filename.is_empty()
            || !filename
                .bytes()
                .all(|byte| matches!(byte, 45..=46 | 48..=57 | 65..=90 | 95 | 97..=122))
        {
            return None;
        }
        let lowercased_filename = filename.to_lowercase();
        if !TRUSTED_FILE_SUFFIXES
            .iter()
            .any(|suffix| lowercased_filename.ends_with(suffix))
        {
            return None;
        }

        let base = UrlComponents::from_url(api_base_url, false)?;
        let scheme = base.scheme()?.to_lowercase();
        if !(scheme == "http" || scheme == "https")
            || base.host().is_none()
            || base.user().is_some()
            || base.password().is_some()
        {
            return None;
        }

        let mut resolved = UrlComponents::new();
        resolved.set_scheme(Some(scheme.as_str()));
        resolved.set_host(base.host().as_deref());
        resolved.set_port(base.port());
        resolved.set_path(path);
        resolved.url()
    }

    pub fn legacy_cover_alternate_url(normalized: &str) -> Option<String> {
        let marker_start = normalized.find(LEGACY_COVER_MARKER)?;
        let marker_end = marker_start + LEGACY_COVER_MARKER.len();
        let after_marker = &normalized[marker_end..];
        let segment_end = after_marker.find(&['?', '#'][..]).unwrap_or(after_marker.len());
        let segment = &after_marker[..segment_end];
        if character_count(segment) < LEGACY_SEGMENT_MIN_LENGTH {
            return None;
        }
        let uuid = character_prefix(segment, UUID_STRING_LENGTH);
        Uuid::from_uuid_string(uuid)?;
        let rest = drop_first_characters(segment, UUID_STRING_LENGTH);
        if !rest.starts_with('.') || rest.starts_with("-cover.") {
            return None;
        }
        let ext = drop_first_characters(rest, 1);
        let safe_ext: String = characters(ext)
            .filter(|character| character.is_ascii() && (character.is_letter() || character.is_number()))
            .take(SAFE_EXTENSION_MAX_LENGTH)
            .map(|character| character.as_str())
            .collect();
        if safe_ext.is_empty() {
            return None;
        }
        let suffix = &after_marker[segment_end..];
        let alt = format!("{}{uuid}-cover.{safe_ext}{suffix}", &normalized[..marker_end]);
        if alt == normalized {
            return None;
        }
        Some(alt)
    }
}
