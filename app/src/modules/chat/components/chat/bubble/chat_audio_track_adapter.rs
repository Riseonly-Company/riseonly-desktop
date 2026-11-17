use std::borrow::Borrow;
use std::path::Path;

use rise_core::foundation::foundation_regex::{HEX_32_TO_128, range_of_regular_expression};
use rise_core::foundation::foundation_text::{is_newline, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;

use crate::modules::chat::shared::utils::chat_document_open::ChatDocumentOpen;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageMediaItem, MessageMediaKind, RemoteMediaURL,
};
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_playback_url::MusicPlaybackUrl;

/// adapter turning a chat attachment into a music track
/// used by bubble player, audio queue and profile music tab
pub enum ChatAudioTrackAdapter {}

const LOCAL_AUDIO_EXTENSIONS: [&str; 11] = [
    "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "caf", "aif", "aiff", "mp4",
];

impl ChatAudioTrackAdapter {
    /// bubble keeps its optimistic id until refetch while profile tab lists the server id
    /// both mean the same file by playback cache key
    pub fn is_same_audio(current: Option<&TrackModel>, track: &TrackModel) -> bool {
        let Some(current) = current else {
            return false;
        };
        current.id == track.id || current.playback_cache_key() == track.playback_cache_key()
    }

    pub fn tracks<M: Borrow<GetMessageMessage>>(chat_id: &str, messages: &[M]) -> Vec<TrackModel> {
        messages
            .iter()
            .flat_map(|message| {
                let message: &GetMessageMessage = message.borrow();
                let cache_message_id = message.sai_server_id();
                message
                    .media_items
                    .iter()
                    .filter_map(|item| {
                        if item.kind == MessageMediaKind::Audio
                            || (message.content_type == MessageContentType::Audio
                                && item.kind == MessageMediaKind::Voice)
                        {
                            return Some(Self::track(&message.id, Some(&cache_message_id), chat_id, item, &message.text));
                        }
                        let is_document = message.content_type == MessageContentType::Document
                            || item.kind == MessageMediaKind::Document;
                        (is_document && ChatDocumentOpen::plays_in_app_player(item))
                            .then(|| Self::track(&message.id, Some(&cache_message_id), chat_id, item, ""))
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    pub fn track(
        message_id: &str,
        cache_message_id: Option<&str>,
        chat_id: &str,
        item: &MessageMediaItem,
        caption: &str,
    ) -> TrackModel {
        let (title, artist) = Self::title_and_artist(item, caption);
        let local_source = item
            .local_preview_url
            .as_deref()
            .map(trimming_whitespaces_and_newlines);
        let local_playback_url = local_source.and_then(|candidate| {
            let url = MusicPlaybackUrl::playback_url(candidate)?;
            if !url.is_file_url() {
                return None;
            }
            if !LOCAL_AUDIO_EXTENSIONS.contains(&url.path_extension().to_lowercase().as_str()) {
                return None;
            }
            Some(url)
        });
        let remote_source = RemoteMediaURL::optional_normalized(Some(item.url.as_str())).filter(|value| {
            let lowered = value.to_lowercase();
            lowered.starts_with("https://") || lowered.starts_with("http://")
        });
        let playback_source = local_playback_url
            .map(|url| url.absolute_string().to_owned())
            .or_else(|| remote_source.clone())
            .unwrap_or_else(|| item.url.clone());
        let trimmed_cache_message_id = cache_message_id
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or("");
        let resolved_cache_message_id = if trimmed_cache_message_id.is_empty() {
            message_id
        } else {
            trimmed_cache_message_id
        };
        let mut track = TrackModel {
            id: format!("chat-audio:{message_id}:{}", item.id),
            title,
            artist,
            artist_id: String::new(),
            album_title: String::new(),
            album_id: String::new(),
            duration_seconds: (item.duration.unwrap_or(0.0) as i64).max(0),
            cover_colors: vec!["#FF5A61".to_owned(), "#D72837".to_owned()],
            cover_image_url: item.thumbnail_url.clone().unwrap_or_default(),
            audio_url: playback_source,
            is_exclusive: false,
            status: String::new(),
            description: None,
            source_chat_id: Some(chat_id.to_owned()),
            source_message_id: Some(message_id.to_owned()),
            artist_is_registered: false,
            remote_audio_url: remote_source,
            transfer_phase: None,
            transferred_bytes: 0,
            transfer_total_bytes: 0,
            transfer_error: None,
            library_owner_user_id: None,
            playback_cache_id: None,
            is_liked: false,
        };
        track.playback_cache_id = Some(format!("chat-audio:{resolved_cache_message_id}:{}", item.id));
        track
    }

    pub fn title_and_artist(item: &MessageMediaItem, caption: &str) -> (String, String) {
        let caption_metadata = Self::metadata_from_caption(caption);
        let fallback = || (tr("chat_audio_track"), tr("chat_audio_unknown_artist"));
        let Some(raw_name) = item
            .file_name
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|name| !name.is_empty())
        else {
            return caption_metadata.unwrap_or_else(fallback);
        };
        let suffix = Path::new(raw_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();
        let cleaned = if LOCAL_AUDIO_EXTENSIONS.contains(&suffix.as_str()) {
            Self::deleting_path_extension(raw_name)
        } else {
            raw_name
        };
        if Self::is_technical_file_name(cleaned) {
            return caption_metadata.unwrap_or_else(fallback);
        }
        for separator in [" \u{2014} ", " - "] {
            let parts: Vec<&str> = cleaned.split(separator).collect();
            if parts.len() >= 2 {
                let title = trimming_whitespaces_and_newlines(&parts[1..].join(separator)).to_owned();
                let artist = trimming_whitespaces_and_newlines(parts[0]).to_owned();
                if Self::is_technical_or_uuid(&title) || Self::is_technical_or_uuid(&artist) {
                    if let Some(caption_metadata) = caption_metadata {
                        return caption_metadata;
                    }
                }
                return (
                    if Self::is_technical_or_uuid(&title) { tr("chat_audio_track") } else { title },
                    if Self::is_technical_or_uuid(&artist) { tr("chat_audio_unknown_artist") } else { artist },
                );
            }
        }
        if Uuid::from_uuid_string(cleaned).is_some() {
            return caption_metadata.unwrap_or_else(fallback);
        }
        (cleaned.to_owned(), tr("chat_audio_unknown_artist"))
    }

    fn deleting_path_extension(name: &str) -> &str {
        match Path::new(name).extension().and_then(|extension| extension.to_str()) {
            Some(extension) => name
                .strip_suffix(extension)
                .and_then(|stem| stem.strip_suffix('.'))
                .unwrap_or(name),
            None => name,
        }
    }

    fn is_technical_or_uuid(value: &str) -> bool {
        value.is_empty() || Self::is_technical_file_name(value) || Uuid::from_uuid_string(value).is_some()
    }

    fn is_technical_file_name(value: &str) -> bool {
        let normalized = trimming_whitespaces_and_newlines(value);
        let lowered = normalized.to_lowercase();
        if lowered.starts_with("http://") || lowered.starts_with("https://") || lowered.starts_with("botmedia_") {
            return true;
        }
        HEX_32_TO_128
            .as_ref()
            .is_some_and(|regex| range_of_regular_expression(normalized, regex).is_some())
    }

    fn metadata_from_caption(caption: &str) -> Option<(String, String)> {
        let lines: Vec<String> = caption
            .split(is_newline)
            .map(|line| trimming_whitespaces_and_newlines(line).to_owned())
            .filter(|line| !line.is_empty())
            .collect();
        let first = lines.first()?;
        let title = trimming_whitespaces_and_newlines(&first.replace('\u{1F3B5}', "")).to_owned();
        let artist = lines
            .get(1)
            .map(|line| trimming_whitespaces_and_newlines(&line.replace('\u{1F464}', "")).to_owned());
        if title.is_empty() {
            return None;
        }
        let artist = artist
            .filter(|artist| !artist.is_empty())
            .unwrap_or_else(|| tr("chat_audio_unknown_artist"));
        Some((title, artist))
    }
}
