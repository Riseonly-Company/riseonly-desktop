use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{
    AnyElement, App, AsyncApp, Bounds, ClipboardItem, Context, ElementId, Entity, FocusHandle, FontWeight, Hsla, Render,
    SharedString, StyleRefinement, Subscription, Task, WeakEntity, Window, black, div, point, px, size,
};

use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_canon::rise_key::events;
use rise_canon::rise_request::message::SharePayload as SharePayloadBody;
use rise_canon::rise_request::message::share_payload::{
    DestinationsElement as SharePayloadDestination, MediaItemsElement,
};
use rise_canon::rise_wire_canon::key;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_errors::{ErrorCode, ErrorParam};
use rise_i18n::{FormatArg, tr, tr_args};
use rise_music::PlaylistVisibility as CanonPlaylistVisibility;
use rise_platform::file_manager::{CocoaError, FileManager};
use rise_platform::haptics::{NotificationKind, notification, selection_changed};
use rise_platform::system_share::{ShareFinished, ShareItem, ShareOutcome, ShareRequest, share};
use rise_theme::{AppTheme, FONT_FAMILY, ThemeStore};
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::main_text::{IgnoresTextSizeDeltaKey, MainText, TextAlignT};
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use serde_json::Value;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingDebounceExt};
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::lib::sai::sai_auth_hooks::sai_auth_hooks;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::push_notifications::universal_link_parser::RiseonlyUniversalLink;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::formatted_text_ui::FormattedText;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter::HoldContextMenuPresenter;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::message_external_share_materializer::MessageExternalShareCoordinator;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::chat_logo::ChatFavouritesLogo;
use crate::modules::chat::components::chat::topic_icon_glyph::TopicIconGlyph;
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageForwardError, MessageForwardLimits, MessageMediaKind,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    ForwardMessageDestinationBody, ForwardMessageResultWireDTO, ForwardMessagesGatewayDTO,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::comment::stores::comment_actions::comment_types::GetCommentsResponse;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::shared::utils::music::music_models::{PlaylistModel, TrackModel};
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_interactions::post_interactions::{PostInteractionsStore, ShareReceiptChannel};
use crate::modules::share_extension::send_handoff::share_send_handoff_actions::ShareSendHandoffActionsStore;
use crate::modules::share_extension::send_handoff::share_send_handoff_models::{
    RiseStoredSharePayloadReply, RiseStoredSharePayloadResult, ShareSendHandoffError,
};
use crate::modules::share_extension::services::rise_share_app_bridge::RiseShareAppBridge;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareTarget, RiseShareTargetKind, RiseShareTopic, RiseShareTopicGroup, RiseShareTopicGrouping,
};
use crate::modules::share_extension::shared::share_target_presentation::ShareTargetPresentation;
use crate::modules::share_extension::stores::share_targets::share_targets_interactions::ShareTargetsInteractionsStore;
use crate::modules::share_extension::stores::share_targets::share_targets_services::ShareTargetsServicesStore;
use crate::modules::story::stores::story::story_actions::story_wire_types::{HighlightDTO, StoryDTO};
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;

const TRACK_STATUS_UPLOADING: &str = "uploading";
const PRIVATE_MUSIC_UPLOAD_CONTEXT: &str = "music_audio";
const PRIVATE_MUSIC_FILE_SUFFIXES: [&str; 5] = [".mp3", ".jpg", ".jpeg", ".png", ".webp"];
const PRIVATE_MUSIC_FILE_STEM_FALLBACK: &str = "track";
const PRIVATE_MUSIC_FILE_EXTENSION: &str = ".mp3";
const PRIVATE_MUSIC_MIME_TYPE: &str = "audio/mpeg";
const LINK_PREVIEW_TYPE_BIG: &str = "BIG";
const ARTIST_TITLE_SEPARATOR: &str = " - ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicSharePlaylistContext {
    pub id: String,
    pub title: String,
    pub visibility: String,
}

impl MusicSharePlaylistContext {
    pub fn is_local(&self) -> bool {
        trimming_whitespaces_and_newlines(&self.visibility).to_lowercase()
            == CanonPlaylistVisibility::LocalPlaylist.wire()
    }
}

pub enum MusicTrackSharePolicy {}

impl MusicTrackSharePolicy {
    pub fn is_pending(track: &TrackModel) -> bool {
        let status = track.status.to_lowercase();
        status.contains(RiseMusicTrackStatus::Pending.wire()) || status == TRACK_STATUS_UPLOADING
    }

    pub fn is_public(track: &TrackModel) -> bool {
        let status = track.status.to_lowercase();
        status == RiseMusicTrackStatus::Published.wire()
            || status == RiseMusicTrackStatus::OriginalAudio.wire()
    }

    pub fn can_share(track: &TrackModel, playlist_id: Option<&str>) -> bool {
        if RiseonlyUniversalLink::track(&track.id, None, AppEnvironment::current()).is_none()
            || Self::is_pending(track)
        {
            return false;
        }
        if Self::is_public(track) {
            return true;
        }
        let Some(playlist_id) = playlist_id else { return false };
        RiseonlyUniversalLink::playlist(playlist_id, AppEnvironment::current()).is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareSheetMediaItem {
    pub media_type: String,
    pub file_url: String,
    pub thumbnail_url: Option<String>,
    pub file_name: String,
    pub mime_type: String,
    pub duration: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub file_size: Option<i64>,
}

impl ShareSheetMediaItem {
    pub fn new(
        media_type: String,
        file_url: String,
        thumbnail_url: Option<String>,
        file_name: String,
        mime_type: String,
        duration: Option<i64>,
    ) -> Self {
        Self {
            media_type,
            file_url,
            thumbnail_url,
            file_name,
            mime_type,
            duration,
            width: None,
            height: None,
            file_size: None,
        }
    }

    pub fn wire_value(&self) -> MediaItemsElement {
        MediaItemsElement {
            duration: self.duration,
            file_name: Some(self.file_name.clone()),
            file_size: self.file_size.filter(|value| *value > 0),
            file_url: Some(self.file_url.clone()),
            height: self.height.filter(|value| *value > 0),
            media_type: Some(self.media_type.clone()),
            mime_type: Some(self.mime_type.clone()),
            thumbnail_url: self.thumbnail_url.clone(),
            width: self.width.filter(|value| *value > 0),
            ..MediaItemsElement::default()
        }
    }
}

/// a photo going into a chat as a new media message
/// carries a local file not the view address, album removal hard deletes the object
/// a message with that url dies for every recipient
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareSheetPhotoDraft {
    pub file_url: Url,
    pub filename: String,
    pub mime_type: String,
    pub width: i64,
    pub height: i64,
}

pub enum ShareTagPolicy {}

impl ShareTagPolicy {
    pub fn normalized(raw_value: &str) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(raw_value);
        let value = trimmed.trim_start_matches('@');
        if value.is_empty() {
            return None;
        }
        Some(format!("@{value}"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SharePayload {
    Post(GetPostFeedResponse),
    Comment { post_id: String, comment_id: String, text: String },
    Profile(ProfileModel),
    Link(Url),
    Story(StoryDTO),
    Highlight(HighlightDTO),
    StickerPack { link: String, title: String },
    Track { track: TrackModel, playlist: Option<MusicSharePlaylistContext> },
    Playlist(PlaylistModel),
    Messages(Vec<GetMessageMessage>),
    Photo(ShareSheetPhotoDraft),
}

impl SharePayload {
    pub fn share_url(&self) -> Option<Url> {
        self.share_url_for(AppEnvironment::current())
    }

    pub fn share_url_for(&self, environment: AppEnvironment) -> Option<Url> {
        match self {
            Self::Post(post) => {
                RiseonlyPublicLink::Post { id: post.rise_view_target_post_id() }.web_url(environment)
            }
            Self::Comment { post_id, comment_id, .. } => RiseonlyPublicLink::PostComment {
                post_id: post_id.clone(),
                comment_id: comment_id.clone(),
            }
            .web_url(environment),
            Self::Profile(profile) => {
                let tag = trimming_whitespaces_and_newlines(&profile.tag);
                let slug = if tag.is_empty() { profile.id.clone() } else { tag.to_owned() };
                RiseonlyPublicLink::Profile { tag: slug }.web_url(environment)
            }
            Self::Story(story) => {
                RiseonlyPublicLink::Story { id: story.id.to_string() }.web_url(environment)
            }
            Self::Highlight(highlight) => {
                RiseonlyPublicLink::Highlight { id: highlight.id.to_string() }.web_url(environment)
            }
            Self::StickerPack { link, .. } => {
                let slug = trimming_whitespaces_and_newlines(link).to_owned();
                RiseonlyPublicLink::StickerPack { slug }.web_url(environment)
            }
            Self::Track { track, playlist } => RiseonlyPublicLink::Track {
                id: track.id.clone(),
                playlist_id: playlist.as_ref().map(|playlist| playlist.id.clone()),
            }
            .web_url(environment),
            Self::Playlist(playlist) => {
                RiseonlyPublicLink::Playlist { reference: playlist.id.clone() }.web_url(environment)
            }
            Self::Link(url) => Some(url.clone()),
            Self::Messages(_) | Self::Photo(_) => None,
        }
    }

    fn share_link_string(&self) -> Option<String> {
        self.share_url().map(|url| url.absolute_string().to_owned())
    }

    pub fn share_text(&self) -> String {
        match self {
            Self::Post(post) => {
                let body = Self::post_preview_text(post);
                let Some(link) = self.share_link_string() else { return body };
                if body.is_empty() { link } else { format!("{body}\n{link}") }
            }
            Self::Comment { text, .. } => {
                let body = trimming_whitespaces_and_newlines(text).to_owned();
                let Some(link) = self.share_link_string() else { return body };
                if body.is_empty() { link } else { format!("{body}\n{link}") }
            }
            Self::Profile(profile) => {
                let name = trimming_whitespaces_and_newlines(&profile.name).to_owned();
                let Some(link) = self.share_link_string() else { return name };
                if name.is_empty() { link } else { format!("{name}\n{link}") }
            }
            Self::Link(url) => url.absolute_string().to_owned(),
            Self::Story(story) => {
                let name = trimming_whitespaces_and_newlines(
                    story.author.as_ref().map(|author| author.name.as_str()).unwrap_or_default(),
                )
                .to_owned();
                let label = if name.is_empty() {
                    tr("story_share_text")
                } else {
                    format!("{} {name}", tr("story_share_text"))
                };
                let Some(link) = self.share_link_string() else { return label };
                format!("{label}\n{link}")
            }
            Self::Highlight(highlight) => {
                let title = trimming_whitespaces_and_newlines(&highlight.title).to_owned();
                let label = if title.is_empty() {
                    tr("highlight_share_text")
                } else {
                    format!("{} \u{ab}{title}\u{bb}", tr("highlight_share_text"))
                };
                let Some(link) = self.share_link_string() else { return label };
                format!("{label}\n{link}")
            }
            Self::StickerPack { title, .. } => {
                let name = trimming_whitespaces_and_newlines(title).to_owned();
                let label = if name.is_empty() { tr("sticker_pack_share_title") } else { name };
                let Some(link) = self.share_link_string() else { return label };
                format!("{label}\n{link}")
            }
            Self::Track { track, playlist } => {
                let title = if track.status == RiseMusicTrackStatus::OriginalAudio.wire() {
                    tr("original_audio_title")
                } else {
                    track.title.clone()
                };
                let label = if track.artist.is_empty() {
                    title
                } else {
                    format!("{}{ARTIST_TITLE_SEPARATOR}{title}", track.artist)
                };
                let Some(link) = self.share_link_string() else { return label };
                if playlist.as_ref().is_some_and(MusicSharePlaylistContext::is_local) {
                    return link;
                }
                let mut lines = vec![link, label];
                if let Some(playlist) = playlist {
                    let title = trimming_whitespaces_and_newlines(&playlist.title);
                    if !title.is_empty() {
                        lines.push(tr_args(
                            "music_track_share_from_playlist",
                            &[FormatArg::Text(title.into())],
                        ));
                    }
                    if playlist.visibility != CanonPlaylistVisibility::PublicPlaylist.wire() {
                        lines.push(tr("music_track_share_access_required"));
                    }
                }
                lines.join("\n")
            }
            Self::Playlist(playlist) => {
                let title = trimming_whitespaces_and_newlines(&playlist.title).to_owned();
                let Some(link) = self.share_link_string() else { return title };
                if playlist.visibility == CanonPlaylistVisibility::PublicPlaylist.wire() {
                    return if title.is_empty() { link } else { format!("{link}\n{title}") };
                }
                let access = tr("music_playlist_share_access_required");
                if title.is_empty() {
                    format!("{link}\n{access}")
                } else {
                    format!("{link}\n{title}\n{access}")
                }
            }
            Self::Messages(messages) => messages
                .iter()
                .filter_map(Self::message_share_text)
                .collect::<Vec<_>>()
                .join("\n\n"),
            Self::Photo(_) => String::new(),
        }
    }

    pub fn share_items(&self) -> Vec<ShareItem> {
        if let Self::Messages(messages) = self {
            return messages.iter().filter_map(Self::message_share_text).map(ShareItem::Text).collect();
        }
        if let Self::Photo(draft) = self {
            return draft.file_url.to_file_path().map(ShareItem::File).into_iter().collect();
        }
        if let Self::Track { playlist, .. } = self
            && playlist.as_ref().is_some_and(MusicSharePlaylistContext::is_local)
            && let Some(url) = self.share_url()
        {
            return vec![ShareItem::Link(url.absolute_string().to_owned())];
        }
        if let Some(url) = self.share_url() {
            return vec![
                ShareItem::Text(self.share_text()),
                ShareItem::Link(url.absolute_string().to_owned()),
            ];
        }
        vec![ShareItem::Text(self.share_text())]
    }

    pub fn in_app_media_items(&self) -> Vec<ShareSheetMediaItem> {
        let Self::Track { track, playlist } = self else { return Vec::new() };
        if !playlist.as_ref().is_some_and(MusicSharePlaylistContext::is_local) {
            return Vec::new();
        }
        let Some(file_url) = Self::private_music_stable_url(&track.stable_audio_url()) else {
            return Vec::new();
        };

        let title = trimming_whitespaces_and_newlines(&track.title);
        let artist = trimming_whitespaces_and_newlines(&track.artist);
        let stem = if artist.is_empty() {
            if title.is_empty() { PRIVATE_MUSIC_FILE_STEM_FALLBACK.to_owned() } else { title.to_owned() }
        } else if title.is_empty() {
            artist.to_owned()
        } else {
            format!("{artist}{ARTIST_TITLE_SEPARATOR}{title}")
        };
        vec![ShareSheetMediaItem::new(
            MessageContentType::Audio.raw_value().to_owned(),
            file_url,
            Self::private_music_stable_url(&track.cover_image_url),
            format!("{stem}{PRIVATE_MUSIC_FILE_EXTENSION}"),
            PRIVATE_MUSIC_MIME_TYPE.to_owned(),
            (track.duration_seconds > 0).then_some(track.duration_seconds),
        )]
    }

    fn private_music_stable_url(raw_value: &str) -> Option<String> {
        let raw_value = trimming_whitespaces_and_newlines(raw_value);
        if raw_value.is_empty() {
            return None;
        }
        let path = if raw_value.starts_with('/') {
            raw_value.to_owned()
        } else {
            let components = UrlComponents::from_string(raw_value)?;
            let scheme = components.scheme()?.to_lowercase();
            if !(scheme == "http" || scheme == "https")
                || components.host().is_none()
                || components.user().is_some()
                || components.password().is_some()
                || components.query().is_some()
                || components.fragment().is_some()
            {
                return None;
            }
            components.path()
        };
        let secure_route = rise_limits::upload_context(PRIVATE_MUSIC_UPLOAD_CONTEXT)?.secure_route?;
        let max_length = usize::try_from(RiseLimits::service_value("private_music_path_max_bytes")).ok()?;
        if !path.starts_with(secure_route)
            || path.chars().count() > max_length
            || path.contains("..")
            || path.contains('%')
            || path.contains('\\')
        {
            return None;
        }
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 7 {
            return None;
        }
        let owner = parts[5];
        let owner_id = Uuid::from_uuid_string(owner)?;
        if owner_id.uuid_string().to_lowercase() != owner {
            return None;
        }
        let file = parts[6];
        if file.is_empty()
            || !file.bytes().all(|byte| {
                matches!(byte, 45..=46 | 48..=57 | 65..=90 | 95 | 97..=122)
            })
        {
            return None;
        }
        let filename = file.to_lowercase();
        if !PRIVATE_MUSIC_FILE_SUFFIXES.iter().any(|suffix| filename.ends_with(suffix)) {
            return None;
        }
        Some(path)
    }

    pub fn in_app_content_type(&self) -> String {
        self.in_app_media_items()
            .into_iter()
            .next()
            .map(|item| item.media_type)
            .unwrap_or_else(|| MessageContentType::Text.raw_value().to_owned())
    }

    pub fn copy_text(&self) -> Option<String> {
        match self {
            Self::Link(url) => Some(url.absolute_string().to_owned()),
            Self::Messages(messages) => {
                if messages.iter().any(|message| message.protects_content_locally()) {
                    return None;
                }
                let text = messages
                    .iter()
                    .map(|message| trimming_whitespaces_and_newlines(&message.text))
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.is_empty() { None } else { Some(text) }
            }
            _ => None,
        }
    }

    pub fn copied_link_text(&self) -> Option<String> {
        let link = self.share_link_string()?;
        match self {
            Self::Track { .. } | Self::Playlist(_) => Some(self.share_text()),
            _ => Some(link),
        }
    }

    pub fn requires_link_permission(&self) -> bool {
        match self {
            Self::Post(post) => FormattedText::contains_link(&Self::post_preview_text(post)),
            Self::Comment { .. }
            | Self::Profile(_)
            | Self::Link(_)
            | Self::Story(_)
            | Self::Highlight(_)
            | Self::StickerPack { .. }
            | Self::Track { .. }
            | Self::Playlist(_) => true,
            Self::Photo(_) => false,
            Self::Messages(messages) => messages.iter().any(|message| {
                FormattedText::contains_link(&message.text) || message.link_preview.is_some()
            }),
        }
    }

    pub fn requires_media_permission(&self) -> bool {
        if matches!(self, Self::Photo(_)) {
            return true;
        }
        if !self.in_app_media_items().is_empty() {
            return true;
        }
        let Self::Messages(messages) = self else { return false };
        messages.iter().any(|message| {
            if message.media_items.iter().any(|item| item.kind != MessageMediaKind::Document) {
                return true;
            }
            matches!(
                message.content_type,
                MessageContentType::Image
                    | MessageContentType::Video
                    | MessageContentType::Animation
                    | MessageContentType::Voice
                    | MessageContentType::Audio
                    | MessageContentType::Sticker
                    | MessageContentType::VideoNote
            )
        })
    }

    pub fn requires_documents_permission(&self) -> bool {
        let Self::Messages(messages) = self else { return false };
        messages.iter().any(|message| {
            message.content_type == MessageContentType::Document
                || message.media_items.iter().any(|item| item.kind == MessageMediaKind::Document)
        })
    }

    pub fn forwarded_messages(&self) -> Option<&[GetMessageMessage]> {
        let Self::Messages(messages) = self else { return None };
        Some(messages)
    }

    pub fn link_preview_type(&self) -> Option<&'static str> {
        match self {
            Self::Track { .. } | Self::Playlist(_) => Some(LINK_PREVIEW_TYPE_BIG),
            _ => None,
        }
    }

    fn post_preview_text(post: &GetPostFeedResponse) -> String {
        let title = trimming_whitespaces_and_newlines(&post.title);
        if title.is_empty() {
            trimming_whitespaces_and_newlines(&post.content).to_owned()
        } else {
            title.to_owned()
        }
    }

    pub fn message_share_text(message: &GetMessageMessage) -> Option<String> {
        let text = if message.content_type == MessageContentType::Call {
            tr("chat_call_generic")
        } else {
            trimming_whitespaces_and_newlines(&message.text).to_owned()
        };
        if text.is_empty() {
            return None;
        }
        let sender = trimming_whitespaces_and_newlines(&message.sender_name);
        if sender.is_empty() { Some(text) } else { Some(format!("{sender}:\n{text}")) }
    }
}

/// message service canonicalizes chat and topic and message ids before echoing a forward result
/// so compare canonical forms, else a delivered batch is reported as not sent
pub enum ShareForwardIdentity {}

impl ShareForwardIdentity {
    pub fn canonical(raw: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw?);
        if value.is_empty() {
            return None;
        }
        Some(
            Uuid::from_uuid_string(value)
                .map(|uuid| uuid.uuid_string().to_lowercase())
                .unwrap_or_else(|| value.to_owned()),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShareSheetDestination {
    pub chat_id: String,
    pub topic_id: Option<String>,
}

impl ShareSheetDestination {
    pub fn new(chat_id: &str, topic_id: Option<&str>) -> Self {
        Self {
            chat_id: ShareForwardIdentity::canonical(Some(chat_id)).unwrap_or_else(|| chat_id.to_owned()),
            topic_id: ShareForwardIdentity::canonical(topic_id),
        }
    }

    pub fn id(&self) -> String {
        format!("{}:{}", self.chat_id, self.topic_id.as_deref().unwrap_or_default())
    }
}

static MAX_DESTINATIONS_PER_REQUEST: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("max_share_destinations"));

pub enum ShareSheetTextBatchPlanner {}

impl ShareSheetTextBatchPlanner {
    pub fn max_destinations_per_request() -> i64 {
        *MAX_DESTINATIONS_PER_REQUEST
    }

    pub fn chunks(destinations: &[ShareSheetDestination]) -> Vec<Vec<ShareSheetDestination>> {
        if destinations.is_empty() {
            return Vec::new();
        }
        let size = usize::try_from(Self::max_destinations_per_request()).unwrap_or(1).max(1);
        destinations.chunks(size).map(<[ShareSheetDestination]>::to_vec).collect()
    }
}

pub enum ShareSheetForwardBatchPlanner {}

impl ShareSheetForwardBatchPlanner {
    pub fn chunks(
        destinations: &[ShareSheetDestination],
        message_count: i64,
    ) -> Vec<Vec<ShareSheetDestination>> {
        let normalized_message_count = MessageForwardLimits::max_messages().min(message_count.max(1));
        let size = MessageForwardLimits::max_destinations()
            .min((MessageForwardLimits::max_copies() / normalized_message_count.max(1)).max(1));
        if destinations.is_empty() {
            return Vec::new();
        }
        let size = usize::try_from(size).unwrap_or(1).max(1);
        destinations.chunks(size).map(<[ShareSheetDestination]>::to_vec).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareSheetSendOutcome {
    pub succeeded: bool,
    pub reason: Option<String>,
    pub slow_mode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShareSheetSendError {
    InvalidDestination,
    InvalidResponse,
    AccountUnavailable,
}

impl fmt::Display for ShareSheetSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDestination => formatter.write_str("invalid share destination"),
            Self::InvalidResponse => formatter.write_str("invalid share response"),
            Self::AccountUnavailable => formatter.write_str("share account unavailable"),
        }
    }
}

impl std::error::Error for ShareSheetSendError {}

struct ShareSheetPhotoFileLease {
    file_url: Url,
}

impl ShareSheetPhotoFileLease {
    fn new(file_url: Url) -> Self {
        Self { file_url }
    }
}

impl Drop for ShareSheetPhotoFileLease {
    fn drop(&mut self) {
        if let Some(path) = self.file_url.to_file_path() {
            let _ = FileManager.remove_item(&path);
        }
    }
}

const MAX_CONCURRENT_POST_SENDS: usize = 4;
const MAX_CONCURRENT_FORWARD_SENDS: usize = 1;
const MAX_PRESENTABLE_REASON_LENGTH: usize = 140;
const TEMP_ID_PREFIX: &str = "temp_";

type ShareSendResult<'a> = Result<(), &'a anyhow::Error>;
type SendContextRef = Rc<RefCell<SendContext>>;

struct SendContext {
    generation: Uuid,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    total_count: usize,
    pending_count: usize,
    failed_destinations: HashSet<ShareSheetDestination>,
    failure_reason: Option<String>,
    retry_key_by_destination: HashMap<ShareSheetDestination, String>,
    task: Option<Task<()>>,
    pending_post_destinations: VecDeque<ShareSheetDestination>,
    active_post_send_count: usize,
    shared_post_chat_ids: HashSet<String>,
    completed_post_destinations: HashSet<ShareSheetDestination>,
    did_report_post_share_engagement: bool,
    pending_forward_batches: VecDeque<ForwardShareBatch>,
    active_forward_send_count: usize,
    photo_lease: Option<Rc<ShareSheetPhotoFileLease>>,
}

impl SendContext {
    fn new(total_count: usize) -> Self {
        Self {
            generation: Uuid::new_v4(),
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            total_count,
            pending_count: total_count,
            failed_destinations: HashSet::new(),
            failure_reason: None,
            retry_key_by_destination: HashMap::new(),
            task: None,
            pending_post_destinations: VecDeque::new(),
            active_post_send_count: 0,
            shared_post_chat_ids: HashSet::new(),
            completed_post_destinations: HashSet::new(),
            did_report_post_share_engagement: false,
            pending_forward_batches: VecDeque::new(),
            active_forward_send_count: 0,
            photo_lease: None,
        }
    }

    fn is_current_account(&self) -> bool {
        self.account_user_id.is_some()
            && self.account_epoch.is_some()
            && SaiAccountCacheScope::is_current_account_epoch(
                self.account_user_id.as_deref(),
                self.account_epoch.as_deref(),
            )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TextShareBatch {
    idempotency_key: String,
    destinations: Vec<ShareSheetDestination>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ForwardShareBatch {
    idempotency_key: String,
    destinations: Vec<ShareSheetDestination>,
}

fn lowercase_uuid() -> String {
    Uuid::new_v4().uuid_string().to_lowercase()
}

fn sorted_destinations(destinations: &HashSet<ShareSheetDestination>) -> Vec<ShareSheetDestination> {
    let mut sorted: Vec<ShareSheetDestination> = destinations.iter().cloned().collect();
    sorted.sort_by_key(ShareSheetDestination::id);
    sorted
}

fn partition_by_retry_key(
    destinations: &HashSet<ShareSheetDestination>,
    context: &SendContext,
) -> (BTreeMap<String, Vec<ShareSheetDestination>>, Vec<ShareSheetDestination>) {
    let mut existing: BTreeMap<String, Vec<ShareSheetDestination>> = BTreeMap::new();
    let mut fresh: Vec<ShareSheetDestination> = Vec::new();
    for destination in sorted_destinations(destinations) {
        match context.retry_key_by_destination.get(&destination) {
            Some(key) => existing.entry(key.clone()).or_default().push(destination),
            None => fresh.push(destination),
        }
    }
    (existing, fresh)
}

pub struct ShareSheetStore {
    is_visible: bool,
    payload: Option<SharePayload>,
    query: String,
    topic_query: String,
    is_search_active: bool,
    selected_destinations: HashSet<ShareSheetDestination>,
    is_sending: bool,
    retried_failed_targets: bool,
    active_sends: HashMap<Uuid, SendContextRef>,
    external_share_generation: Option<Uuid>,
    photo_lease: Option<Rc<ShareSheetPhotoFileLease>>,
}

impl ShareSheetStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            is_visible: false,
            payload: None,
            query: String::new(),
            topic_query: String::new(),
            is_search_active: false,
            selected_destinations: HashSet::new(),
            is_sending: false,
            retried_failed_targets: false,
            active_sends: HashMap::new(),
            external_share_generation: None,
            photo_lease: None,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.is_visible
    }

    pub fn set_visible(&mut self, is_visible: bool, cx: &mut Context<Self>) {
        self.is_visible = is_visible;
        cx.notify();
    }

    pub fn payload(&self) -> Option<&SharePayload> {
        self.payload.as_ref()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.query = query;
        cx.notify();
    }

    pub fn topic_query(&self) -> &str {
        &self.topic_query
    }

    pub fn set_topic_query(&mut self, topic_query: String, cx: &mut Context<Self>) {
        self.topic_query = topic_query;
        cx.notify();
    }

    pub fn is_search_active(&self) -> bool {
        self.is_search_active
    }

    pub fn set_search_active(&mut self, is_search_active: bool, cx: &mut Context<Self>) {
        self.is_search_active = is_search_active;
        cx.notify();
    }

    pub fn selected_destinations(&self) -> &HashSet<ShareSheetDestination> {
        &self.selected_destinations
    }

    pub fn is_sending(&self) -> bool {
        self.is_sending
    }

    pub fn selected_count(&self) -> usize {
        self.selected_destinations.len()
    }

    pub fn requires_link_permission(&self) -> bool {
        self.payload.as_ref().is_some_and(SharePayload::requires_link_permission)
    }

    pub fn requires_media_permission(&self) -> bool {
        self.payload.as_ref().is_some_and(SharePayload::requires_media_permission)
    }

    pub fn requires_documents_permission(&self) -> bool {
        self.payload.as_ref().is_some_and(SharePayload::requires_documents_permission)
    }

    pub fn present_post(&mut self, post: GetPostFeedResponse, cx: &mut Context<Self>) {
        self.present(SharePayload::Post(post), cx);
    }

    pub fn present_comment(&mut self, comment: &GetCommentsResponse, post_id: &str, cx: &mut Context<Self>) {
        let text = if comment.original_content.is_empty() {
            comment.content.clone()
        } else {
            comment.original_content.clone()
        };
        let payload = SharePayload::Comment {
            post_id: post_id.to_owned(),
            comment_id: comment.id.clone(),
            text,
        };
        if payload.share_url().is_none() {
            return;
        }
        self.present(payload, cx);
    }

    pub fn present_profile(&mut self, profile: ProfileModel, cx: &mut Context<Self>) {
        self.present(SharePayload::Profile(profile), cx);
    }

    pub fn present_link(&mut self, link: Url, cx: &mut Context<Self>) {
        self.present(SharePayload::Link(link), cx);
    }

    pub fn present_story(&mut self, story: StoryDTO, cx: &mut Context<Self>) {
        self.present(SharePayload::Story(story), cx);
    }

    pub fn present_highlight(&mut self, highlight: HighlightDTO, cx: &mut Context<Self>) {
        self.present(SharePayload::Highlight(highlight), cx);
    }

    pub fn present_sticker_pack(&mut self, link: String, title: String, cx: &mut Context<Self>) {
        self.present(SharePayload::StickerPack { link, title }, cx);
    }

    fn warn_track_share(key: &str, cx: &mut App) {
        let options = NotifierSendOptions {
            r#type: NotifierType::Warning,
            position: NotifierPosition::Bottom,
            ..NotifierSendOptions::default()
        };
        Notifier::send(tr(key), options, cx);
    }

    pub fn present_track(&mut self, track: TrackModel, playlist_id: Option<String>, cx: &mut Context<Self>) {
        let environment = AppEnvironment::current();
        if RiseonlyUniversalLink::track(&track.id, None, environment).is_none() {
            let key = if MusicTrackSharePolicy::is_pending(&track) || track.id.starts_with(TEMP_ID_PREFIX) {
                "music_track_share_pending"
            } else {
                "music_track_share_unavailable"
            };
            Self::warn_track_share(key, cx);
            return;
        }
        if MusicTrackSharePolicy::is_pending(&track) {
            Self::warn_track_share("music_track_share_pending", cx);
            return;
        }

        let resolved_playlist_id = playlist_id.or_else(|| {
            let player_state = MusicPlayerState::shared(cx);
            let player = player_state.read(cx);
            if player.current_track.as_ref().map(|current| current.id.as_str()) != Some(track.id.as_str()) {
                return None;
            }
            player.listening_playlist_id().map(str::to_owned)
        });
        let playlist_context = match resolved_playlist_id {
            Some(resolved_playlist_id) => {
                if RiseonlyUniversalLink::playlist(&resolved_playlist_id, environment).is_none() {
                    Self::warn_track_share("music_track_share_unavailable", cx);
                    return;
                }
                let playlist = MusicServicesStore::shared(cx)
                    .update(cx, |services, cx| services.playlist(&resolved_playlist_id, cx));
                let title = playlist.as_ref().and_then(|playlist| playlist.title.clone()).unwrap_or_default();
                let visibility = playlist
                    .and_then(|playlist| playlist.visibility)
                    .unwrap_or_else(|| CanonPlaylistVisibility::PrivatePlaylist.wire().to_owned());
                Some(MusicSharePlaylistContext { id: resolved_playlist_id, title, visibility })
            }
            None => None,
        };
        let context_id = playlist_context.as_ref().map(|context| context.id.as_str());
        if !MusicTrackSharePolicy::can_share(&track, context_id) {
            Self::warn_track_share("music_track_share_unavailable", cx);
            return;
        }
        self.present(SharePayload::Track { track, playlist: playlist_context }, cx);
    }

    pub fn present_playlist(&mut self, playlist: PlaylistModel, cx: &mut Context<Self>) {
        if RiseonlyUniversalLink::playlist(&playlist.id, AppEnvironment::current()).is_none() {
            Self::warn_track_share("music_playlist_share_unavailable", cx);
            return;
        }
        self.present(SharePayload::Playlist(playlist), cx);
    }

    pub fn present_photo(&mut self, draft: ShareSheetPhotoDraft, cx: &mut Context<Self>) {
        self.present(SharePayload::Photo(draft), cx);
    }

    pub fn present_messages(&mut self, messages: Vec<GetMessageMessage>, cx: &mut Context<Self>) {
        let count = messages.len();
        let values: Vec<GetMessageMessage> = messages
            .into_iter()
            .filter(|message| !message.is_system() && !message.protects_content_locally())
            .collect();
        let max_messages = usize::try_from(MessageForwardLimits::max_messages()).unwrap_or(0);
        if values.len() != count || values.is_empty() || values.len() > max_messages {
            return;
        }
        self.present(SharePayload::Messages(values), cx);
    }

    fn present(&mut self, payload: SharePayload, cx: &mut Context<Self>) {
        HoldContextMenuPresenter::dismiss(cx);
        if let SharePayload::Photo(draft) = &payload {
            if self.photo_lease.as_ref().map(|lease| &lease.file_url) != Some(&draft.file_url) {
                let reused = self.active_sends.values().find_map(|context| {
                    context.borrow().photo_lease.clone().filter(|lease| lease.file_url == draft.file_url)
                });
                self.photo_lease =
                    Some(reused.unwrap_or_else(|| Rc::new(ShareSheetPhotoFileLease::new(draft.file_url.clone()))));
            }
        } else {
            self.photo_lease = None;
        }
        let has_media = payload.requires_media_permission();
        let has_link = payload.requires_link_permission();
        let has_documents = payload.requires_documents_permission();
        self.payload = Some(payload);
        self.reset_presentation_state();
        self.retried_failed_targets = false;
        ShareTargetsInteractionsStore::shared(cx).update(cx, |targets, cx| {
            targets.reset_picker_pages(cx);
            targets.load_picker_targets(None, has_media, has_link, has_documents, cx);
        });
        self.is_visible = true;
        cx.notify();
    }

    /// the list asked for while the connection was coming back fails on the dropped socket
    /// asked again when the socket is back and once if it failed with the socket up
    pub fn retry_failed_targets(&mut self, reconnected: bool, cx: &mut Context<Self>) {
        if !self.is_visible {
            return;
        }
        let Some(payload) = self.payload.as_ref() else { return };
        if !reconnected {
            if self.retried_failed_targets || !SaiWebSocketManager::shared(cx).read(cx).is_connected() {
                return;
            }
            self.retried_failed_targets = true;
        }
        let has_media = payload.requires_media_permission();
        let has_link = payload.requires_link_permission();
        let has_documents = payload.requires_documents_permission();
        ShareTargetsInteractionsStore::shared(cx).update(cx, |targets, cx| {
            targets.load_picker_targets(None, has_media, has_link, has_documents, cx);
        });
    }

    /// closing cancels nothing, the send was handed off already and reports through the notifier
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.is_visible = false;
        cx.notify();
    }

    pub fn finish_dismissal(&mut self, cx: &mut Context<Self>) {
        if self.is_visible {
            return;
        }
        self.payload = None;
        self.photo_lease = None;
        self.reset_presentation_state();
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.external_share_generation = None;
        MessageExternalShareCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.reset_account_state(cx));
        for context in self.active_sends.values() {
            context.borrow_mut().task = None;
        }
        self.active_sends.clear();
        self.dismiss(cx);
        self.finish_dismissal(cx);
    }

    fn reset_presentation_state(&mut self) {
        self.query.clear();
        self.topic_query.clear();
        self.is_search_active = false;
        self.selected_destinations.clear();
        self.is_sending = false;
    }

    pub fn toggle(&mut self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        if self.is_sending {
            return;
        }
        let destination = ShareSheetDestination::new(chat_id, topic_id);
        if !self.selected_destinations.remove(&destination) {
            self.selected_destinations.insert(destination);
        }
        let _ = selection_changed();
        cx.notify();
    }

    pub fn is_selected(&self, chat_id: &str, topic_id: Option<&str>) -> bool {
        self.selected_destinations.contains(&ShareSheetDestination::new(chat_id, topic_id))
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.is_sending || self.selected_destinations.is_empty() {
            return;
        }
        self.selected_destinations.clear();
        let _ = selection_changed();
        cx.notify();
    }

    pub fn selected_count_in(&self, chat_id: &str) -> usize {
        self.selected_destinations.iter().filter(|destination| destination.chat_id == chat_id).count()
    }

    fn is_active(&self, context: &SendContextRef) -> bool {
        let generation = context.borrow().generation;
        self.active_sends.get(&generation).is_some_and(|active| Rc::ptr_eq(active, context))
    }

    pub fn send_to_selected(&mut self, cx: &mut Context<Self>) {
        let Some(payload) = self.payload.clone() else { return };
        if self.selected_destinations.is_empty() || self.is_sending {
            return;
        }

        let destinations = self.selected_destinations.clone();
        let context: SendContextRef = Rc::new(RefCell::new(SendContext::new(destinations.len())));
        context.borrow_mut().photo_lease = self.photo_lease.clone();
        let generation = context.borrow().generation;
        self.active_sends.insert(generation, Rc::clone(&context));
        self.is_sending = true;
        let _ = notification(NotificationKind::Success);

        match payload {
            SharePayload::Post(post) => self.send_post(Rc::new(post), &destinations, &context, cx),
            text @ (SharePayload::Comment { .. }
            | SharePayload::Profile(_)
            | SharePayload::Link(_)
            | SharePayload::Story(_)
            | SharePayload::Highlight(_)
            | SharePayload::StickerPack { .. }
            | SharePayload::Track { .. }
            | SharePayload::Playlist(_)) => self.send_text_payload(text, &destinations, &context, None, cx),
            SharePayload::Photo(draft) => {
                let photo = SharePayload::Photo(draft.clone());
                self.send_text_payload(photo, &destinations, &context, Some(draft), cx);
            }
            SharePayload::Messages(messages) => {
                self.send_forwarded_messages(&messages, &destinations, &context, cx);
            }
        }

        self.is_sending = false;
        self.selected_destinations.clear();
        self.dismiss(cx);
    }

    fn send_post(
        &mut self,
        post: Rc<GetPostFeedResponse>,
        destinations: &HashSet<ShareSheetDestination>,
        context: &SendContextRef,
        cx: &mut Context<Self>,
    ) {
        {
            let mut send = context.borrow_mut();
            for destination in destinations {
                send.retry_key_by_destination
                    .insert(destination.clone(), format!("{TEMP_ID_PREFIX}{}", lowercase_uuid()));
            }
            send.pending_post_destinations = sorted_destinations(destinations).into();
            send.active_post_send_count = 0;
        }
        self.pump_post_sends(post, context, cx);
    }

    fn pump_post_sends(&mut self, post: Rc<GetPostFeedResponse>, context: &SendContextRef, cx: &mut Context<Self>) {
        if !self.is_active(context) || !context.borrow().is_current_account() {
            return;
        }

        loop {
            let (destination, idempotency_key) = {
                let mut send = context.borrow_mut();
                if send.active_post_send_count >= MAX_CONCURRENT_POST_SENDS {
                    break;
                }
                let Some(destination) = send.pending_post_destinations.pop_front() else { break };
                send.active_post_send_count += 1;
                let key = send.retry_key_by_destination.get(&destination).cloned();
                (destination, key)
            };

            let weak = cx.weak_entity();
            let completion_post = Rc::clone(&post);
            let completion_context = Rc::clone(context);
            let completion_destination = destination.clone();
            let completion: Rc<dyn Fn(anyhow::Result<()>, &mut App)> = Rc::new(move |result, cx| {
                let weak = weak.clone();
                let post = Rc::clone(&completion_post);
                let context = Rc::clone(&completion_context);
                let destination = completion_destination.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |store, cx| {
                        store.finish_post_send(post, destination, result, &context, cx);
                    });
                });
            });
            let forwarded = Rc::clone(&completion);
            let temp_id = MessageInteractionsStore::shared(cx).update(cx, |messages, cx| {
                messages.share_post_to_chat(
                    &destination.chat_id,
                    &post,
                    destination.topic_id.as_deref(),
                    idempotency_key,
                    false,
                    Some(Box::new(move |result: anyhow::Result<()>, cx: &mut App| forwarded(result, cx))),
                    cx,
                )
            });
            if temp_id.is_none() {
                completion(Err(anyhow::Error::new(ShareSheetSendError::InvalidDestination)), cx);
            }
        }
    }

    fn finish_post_send(
        &mut self,
        post: Rc<GetPostFeedResponse>,
        destination: ShareSheetDestination,
        result: anyhow::Result<()>,
        context: &SendContextRef,
        cx: &mut Context<Self>,
    ) {
        if !self.is_active(context) || !context.borrow().is_current_account() {
            return;
        }
        if !context.borrow_mut().completed_post_destinations.insert(destination.clone()) {
            return;
        }
        {
            let mut send = context.borrow_mut();
            send.active_post_send_count = send.active_post_send_count.saturating_sub(1);
        }
        if result.is_ok() {
            let reports_engagement = {
                let mut send = context.borrow_mut();
                !std::mem::replace(&mut send.did_report_post_share_engagement, true)
            };
            if reports_engagement {
                PostInteractionsStore::shared(cx).update(cx, |posts, cx| posts.report_share_engagement(&post, cx));
            }
            let chat_count = {
                let mut send = context.borrow_mut();
                send.shared_post_chat_ids
                    .insert(destination.chat_id.clone())
                    .then(|| send.shared_post_chat_ids.len())
            };
            if let Some(chat_count) = chat_count {
                let chat_count = i64::try_from(chat_count).unwrap_or(i64::MAX);
                PostInteractionsStore::shared(cx).update(cx, |posts, cx| posts.note_chat_shares(&post, chat_count, cx));
            }
        }
        self.handle_send_result(result.as_ref().map(|_| ()), &destination, None, context, cx);
        self.pump_post_sends(post, context, cx);
    }

    fn send_forwarded_messages(
        &mut self,
        messages: &[GetMessageMessage],
        destinations: &HashSet<ShareSheetDestination>,
        context: &SendContextRef,
        cx: &mut Context<Self>,
    ) {
        let message_ids: Vec<String> = messages
            .iter()
            .map(|message| trimming_whitespaces_and_newlines(&message.sai_server_id()).to_owned())
            .collect();
        let max_messages = usize::try_from(MessageForwardLimits::max_messages()).unwrap_or(0);
        let unique: HashSet<&String> = message_ids.iter().collect();
        let is_valid = (1..=max_messages).contains(&message_ids.len())
            && unique.len() == message_ids.len()
            && message_ids.iter().all(|id| !id.is_empty() && !GetSaiServerId::is_temp_id(Some(id)));
        if !is_valid {
            let error = anyhow::Error::new(MessageForwardError::InvalidRequest);
            for destination in destinations {
                self.handle_send_result(Err(&error), destination, None, context, cx);
            }
            return;
        }

        let message_count = i64::try_from(message_ids.len()).unwrap_or(i64::MAX);
        let batches = Self::make_forward_share_batches(destinations, message_count, &mut context.borrow_mut());
        {
            let mut send = context.borrow_mut();
            send.pending_forward_batches = batches.into();
            send.active_forward_send_count = 0;
        }
        self.pump_forward_sends(Rc::new(message_ids), context, cx);
    }

    fn pump_forward_sends(&mut self, message_ids: Rc<Vec<String>>, context: &SendContextRef, cx: &mut Context<Self>) {
        if !self.is_active(context) {
            return;
        }

        loop {
            let batch = {
                let mut send = context.borrow_mut();
                if send.active_forward_send_count >= MAX_CONCURRENT_FORWARD_SENDS {
                    break;
                }
                let Some(batch) = send.pending_forward_batches.pop_front() else { break };
                send.active_forward_send_count += 1;
                batch
            };
            let wire_destinations: Vec<ForwardMessageDestinationBody> = batch
                .destinations
                .iter()
                .map(|destination| ForwardMessageDestinationBody {
                    chat_id: Some(destination.chat_id.clone()),
                    topic_id: destination.topic_id.clone(),
                })
                .collect();
            let weak = cx.weak_entity();
            let completion_context = Rc::clone(context);
            let completion_ids = Rc::clone(&message_ids);
            let idempotency_key = batch.idempotency_key.clone();
            MessageInteractionsStore::shared(cx).update(cx, |messages, cx| {
                messages.forward_messages(
                    message_ids.as_ref().clone(),
                    wire_destinations,
                    idempotency_key,
                    Box::new(move |result: anyhow::Result<ForwardMessagesGatewayDTO>, cx: &mut App| {
                        cx.defer(move |cx| {
                            let _ = weak.update(cx, |store, cx| {
                                store.finish_forward_batch(batch, completion_ids, result, &completion_context, cx);
                            });
                        });
                    }),
                    cx,
                );
            });
        }
    }

    fn finish_forward_batch(
        &mut self,
        batch: ForwardShareBatch,
        message_ids: Rc<Vec<String>>,
        result: anyhow::Result<ForwardMessagesGatewayDTO>,
        context: &SendContextRef,
        cx: &mut Context<Self>,
    ) {
        if !self.is_active(context) {
            return;
        }
        {
            let mut send = context.borrow_mut();
            send.active_forward_send_count = send.active_forward_send_count.saturating_sub(1);
        }
        let outcomes =
            result.and_then(|response| Self::validated_forward_outcomes(&response, &batch, &message_ids));
        match outcomes {
            Ok(outcomes) => {
                let invalid = anyhow::Error::new(MessageForwardError::InvalidResponse);
                for destination in &batch.destinations {
                    let outcome = outcomes.get(destination);
                    let succeeded = outcome.is_some_and(|outcome| outcome.succeeded);
                    let mut reason = outcome.and_then(|outcome| outcome.reason.clone());
                    if outcome.is_some_and(|outcome| outcome.slow_mode) {
                        reason = Self::slow_mode_reason(&destination.chat_id, cx).or(reason);
                    }
                    let result = if succeeded { Ok(()) } else { Err(&invalid) };
                    self.handle_send_result(result, destination, reason, context, cx);
                }
            }
            Err(error) => {
                for destination in &batch.destinations {
                    self.handle_send_result(Err(&error), destination, None, context, cx);
                }
            }
        }
        self.pump_forward_sends(message_ids, context, cx);
    }

    fn make_forward_share_batches(
        destinations: &HashSet<ShareSheetDestination>,
        message_count: i64,
        context: &mut SendContext,
    ) -> Vec<ForwardShareBatch> {
        let (existing, fresh) = partition_by_retry_key(destinations, context);
        let mut batches: Vec<ForwardShareBatch> = existing
            .into_iter()
            .filter(|(_, values)| !values.is_empty())
            .map(|(idempotency_key, destinations)| ForwardShareBatch { idempotency_key, destinations })
            .collect();
        for values in ShareSheetForwardBatchPlanner::chunks(&fresh, message_count) {
            let key = lowercase_uuid();
            for destination in &values {
                context.retry_key_by_destination.insert(destination.clone(), key.clone());
            }
            batches.push(ForwardShareBatch { idempotency_key: key, destinations: values });
        }
        batches
    }

    fn validated_forward_outcomes(
        response: &ForwardMessagesGatewayDTO,
        batch: &ForwardShareBatch,
        message_ids: &[String],
    ) -> anyhow::Result<HashMap<ShareSheetDestination, ShareSheetSendOutcome>> {
        let key_matches = response
            .idempotency_key
            .as_deref()
            .is_some_and(|key| key.to_lowercase() == batch.idempotency_key.to_lowercase());
        let Some(results) = response.results.as_ref().filter(|_| key_matches) else {
            return Err(anyhow::Error::new(MessageForwardError::InvalidResponse));
        };
        if let Some(all_succeeded) = response.all_succeeded
            && all_succeeded != results.iter().all(|result| result.success)
        {
            return Err(anyhow::Error::new(MessageForwardError::InvalidResponse));
        }

        let mut grouped: HashMap<ShareSheetDestination, Vec<&ForwardMessageResultWireDTO>> = HashMap::new();
        for result in results {
            let Some(wire_destination) = result.destination.as_ref() else { continue };
            let destination =
                ShareSheetDestination::new(&wire_destination.chat_id, wire_destination.topic_id.as_deref());
            grouped.entry(destination).or_default().push(result);
        }

        let expected_message_ids: HashSet<String> =
            message_ids.iter().filter_map(|id| ShareForwardIdentity::canonical(Some(id))).collect();
        let mut outcomes: HashMap<ShareSheetDestination, ShareSheetSendOutcome> = HashMap::new();
        for destination in &batch.destinations {
            let values: &[&ForwardMessageResultWireDTO] = grouped.get(destination).map(Vec::as_slice).unwrap_or_default();
            let source_ids: Vec<String> = values
                .iter()
                .filter_map(|value| ShareForwardIdentity::canonical(Some(&value.source_message_id)))
                .collect();
            let source_set: HashSet<String> = source_ids.iter().cloned().collect();
            if values.len() != message_ids.len()
                || source_set != expected_message_ids
                || source_set.len() != source_ids.len()
            {
                outcomes.insert(destination.clone(), ShareSheetSendOutcome { succeeded: false, reason: None, slow_mode: false });
                continue;
            }
            let refused = values.iter().find(|value| !value.success);
            let refusal = refused.and_then(|value| value.refusal());
            let canon = refusal.as_ref().and_then(CanonErrors::text_for);
            let slow_mode = refused.is_some() && canon.is_none() && Self::is_slow_mode(refusal.as_ref());
            let reason = canon.or_else(|| refused.and_then(|value| value.error.clone()));
            outcomes.insert(
                destination.clone(),
                ShareSheetSendOutcome { succeeded: values.iter().all(|value| value.success), reason, slow_mode },
            );
        }
        Ok(outcomes)
    }

    fn send_text_payload(
        &mut self,
        payload: SharePayload,
        destinations: &HashSet<ShareSheetDestination>,
        context: &SendContextRef,
        photo: Option<ShareSheetPhotoDraft>,
        cx: &mut Context<Self>,
    ) {
        let batches = Self::make_text_share_batches(destinations, &mut context.borrow_mut());
        let user_id = sai_auth_hooks()
            .and_then(|hooks| (hooks.effective_user_id_for_wire)(cx))
            .filter(|user_id| !user_id.is_empty());
        if user_id.is_none() {
            let error = anyhow::Error::new(ShareSheetSendError::AccountUnavailable);
            for destination in destinations {
                self.handle_send_result(Err(&error), destination, None, context, cx);
            }
            return;
        }

        let task_context = Rc::clone(context);
        let destinations: Vec<ShareSheetDestination> = sorted_destinations(destinations);
        let task = cx.spawn(async move |this, cx| {
            Self::run_text_send(&this, &payload, &batches, &destinations, photo, &task_context, cx).await;
            task_context.borrow_mut().photo_lease = None;
            let _ = this.update(cx, |store, _| {
                if store.is_active(&task_context) {
                    let generation = task_context.borrow().generation;
                    store.active_sends.remove(&generation);
                }
            });
        });
        context.borrow_mut().task = Some(task);
    }

    async fn run_text_send(
        this: &WeakEntity<Self>,
        payload: &SharePayload,
        batches: &[TextShareBatch],
        destinations: &[ShareSheetDestination],
        photo: Option<ShareSheetPhotoDraft>,
        context: &SendContextRef,
        cx: &mut AsyncApp,
    ) {
        let is_active = |cx: &mut AsyncApp| this.read_with(cx, |store, _| store.is_active(context)).unwrap_or(false);
        if !context.borrow().is_current_account() || !is_active(cx) {
            return;
        }
        let mut prepared_media: Vec<ShareSheetMediaItem> = Vec::new();
        if let Some(draft) = photo {
            let Some(anchor_chat_id) = batches
                .first()
                .and_then(|batch| batch.destinations.first())
                .map(|destination| destination.chat_id.clone())
            else {
                return;
            };
            let upload = cx.update(|cx| Self::upload_photo(draft, anchor_chat_id, cx));
            match upload.await {
                Ok(media) => prepared_media = media,
                Err(error) => {
                    for destination in destinations {
                        if !is_active(cx) {
                            return;
                        }
                        let _ = this.update(cx, |store, cx| {
                            store.handle_send_result(Err(&error), destination, None, context, cx);
                        });
                    }
                    return;
                }
            }
        }

        for batch in batches {
            if !is_active(cx) {
                return;
            }
            let media_override = (!prepared_media.is_empty()).then_some(prepared_media.as_slice());
            let request = Self::make_text_share_body(payload, batch, media_override);
            let reply = cx.update(|cx| {
                ShareSendHandoffActionsStore::shared(cx).update(cx, |handoff, cx| {
                    handoff.share_payload_action(request, false, Box::new(|_: &mut App| Ok(())), cx)
                })
            });
            let validated = reply.await.and_then(|response| Self::validated_results(&response, batch));
            match &validated {
                Ok(results) => tracing::warn!(
                    target: "riseonly::X1057",
                    batch = batch.destinations.len(),
                    failed = ?results.iter().filter(|(_, result)| !result.success).map(|(destination, _)| destination.id()).collect::<Vec<_>>(),
                    "share batch reply"
                ),
                Err(error) => tracing::warn!(
                    target: "riseonly::X1057",
                    batch = batch.destinations.len(),
                    error = %error,
                    "share batch failed"
                ),
            }
            match validated {
                Ok(results) => {
                    let invalid = anyhow::Error::new(ShareSheetSendError::InvalidResponse);
                    for destination in &batch.destinations {
                        if !is_active(cx) {
                            return;
                        }
                        let result = results.get(destination);
                        let succeeded = result.is_some_and(|result| result.success);
                        let refusal = result.and_then(|result| result.refusal());
                        let canon = refusal.as_ref().and_then(CanonErrors::text_for);
                        let slow_mode = !succeeded && canon.is_none() && Self::is_slow_mode(refusal.as_ref());
                        let prose = canon.or_else(|| result.and_then(|result| result.error.clone()));
                        let _ = this.update(cx, |store, cx| {
                            let reason = if slow_mode {
                                Self::slow_mode_reason(&destination.chat_id, cx).or(prose)
                            } else {
                                prose
                            };
                            if succeeded {
                                if let Some(result) = result {
                                    MessageInteractionsStore::shared(cx).update(cx, |messages, cx| {
                                        messages.apply_shared_payload_success(result, destination, cx);
                                    });
                                }
                                RiseShareAppBridge::donate_outgoing_interaction(&destination.chat_id, cx);
                            }
                            let outcome = if succeeded { Ok(()) } else { Err(&invalid) };
                            store.handle_send_result(outcome, destination, reason, context, cx);
                        });
                    }
                }
                Err(error) => {
                    for destination in &batch.destinations {
                        if !is_active(cx) {
                            return;
                        }
                        let _ = this.update(cx, |store, cx| {
                            store.handle_send_result(Err(&error), destination, None, context, cx);
                        });
                    }
                }
            }
        }
    }

    fn upload_photo(
        draft: ShareSheetPhotoDraft,
        chat_id: String,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<ShareSheetMediaItem>>> {
        let account_id = AuthServicesStore::shared(cx)
            .read(cx)
            .profile()
            .map(|profile| profile.id.clone())
            .or_else(SaiAccountCacheScope::active_user_id);
        let Some(account_id) = account_id.filter(|account_id| !account_id.is_empty()) else {
            return Task::ready(Err(anyhow::Error::new(ShareSheetSendError::AccountUnavailable)));
        };
        let Some(file_path) = draft.file_url.to_file_path() else {
            return Task::ready(Err(anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::NotFound))));
        };
        let upload = ShareSendHandoffActionsStore::shared(cx).update(cx, |handoff, cx| {
            handoff.upload_attachment(
                file_path.clone(),
                draft.filename.clone(),
                draft.mime_type.clone(),
                chat_id,
                account_id,
                SaiAccountCacheScope::active_epoch(),
                Box::new(|_: f64| {}),
                cx,
            )
        });
        cx.spawn(async move |_| {
            let response = upload.await?;
            let root = AnyDictionaryBridge::dictionary(response.get(key::DATA)).unwrap_or_else(|| response.clone());
            let raw_url = [events::FILE_URL, events::URL].into_iter().find_map(|wire_key| {
                root.get(wire_key)
                    .and_then(Value::as_str)
                    .or_else(|| response.get(wire_key).and_then(Value::as_str))
                    .filter(|value| !trimming_whitespaces_and_newlines(value).is_empty())
                    .map(str::to_owned)
            });
            let Some(remote_url) = raw_url
                .map(|value| trimming_whitespaces_and_newlines(&value).to_owned())
                .filter(|value| !value.is_empty())
            else {
                return Err(anyhow::Error::new(ShareSheetSendError::InvalidResponse));
            };
            let byte_count = FileManager.attributes_of_item(&file_path).ok().map(|attributes| attributes.size);
            Ok(vec![ShareSheetMediaItem {
                media_type: MessageContentType::Image.raw_value().to_owned(),
                file_url: remote_url,
                thumbnail_url: None,
                file_name: draft.filename,
                mime_type: draft.mime_type,
                duration: None,
                width: Some(draft.width),
                height: Some(draft.height),
                file_size: byte_count,
            }])
        })
    }

    fn make_text_share_batches(
        destinations: &HashSet<ShareSheetDestination>,
        context: &mut SendContext,
    ) -> Vec<TextShareBatch> {
        let (existing, fresh) = partition_by_retry_key(destinations, context);
        let mut batches: Vec<TextShareBatch> = existing
            .into_iter()
            .filter(|(_, values)| !values.is_empty())
            .map(|(idempotency_key, destinations)| TextShareBatch { idempotency_key, destinations })
            .collect();
        for values in ShareSheetTextBatchPlanner::chunks(&fresh) {
            let key = lowercase_uuid();
            for destination in &values {
                context.retry_key_by_destination.insert(destination.clone(), key.clone());
            }
            batches.push(TextShareBatch { idempotency_key: key, destinations: values });
        }
        batches
    }

    fn make_text_share_body(
        payload: &SharePayload,
        batch: &TextShareBatch,
        media_override: Option<&[ShareSheetMediaItem]>,
    ) -> SharePayloadBody {
        let destinations: Vec<SharePayloadDestination> = batch
            .destinations
            .iter()
            .map(|destination| SharePayloadDestination {
                chat_id: Some(destination.chat_id.clone()),
                topic_id: destination.topic_id.clone(),
            })
            .collect();
        let media: Vec<ShareSheetMediaItem> =
            media_override.map_or_else(|| payload.in_app_media_items(), <[ShareSheetMediaItem]>::to_vec);
        let sends_track_file = !media.is_empty() && matches!(payload, SharePayload::Track { .. });
        let mut body = SharePayloadBody::new(batch.idempotency_key.clone());
        body.content = Some(if sends_track_file { String::new() } else { payload.share_text() });
        body.content_type =
            Some(media.first().map_or_else(|| payload.in_app_content_type(), |item| item.media_type.clone()));
        body.destinations = Some(destinations);
        body.link_preview_type = if sends_track_file { None } else { payload.link_preview_type().map(str::to_owned) };
        body.media_items = Some(media.iter().map(ShareSheetMediaItem::wire_value).collect());
        body
    }

    fn validated_results(
        response: &RiseStoredSharePayloadReply,
        batch: &TextShareBatch,
    ) -> anyhow::Result<HashMap<ShareSheetDestination, RiseStoredSharePayloadResult>> {
        let invalid = || anyhow::Error::new(ShareSheetSendError::InvalidResponse);
        if response.idempotency_key.to_lowercase() != batch.idempotency_key.to_lowercase()
            || response.all_succeeded != response.results.iter().all(|result| result.success)
        {
            return Err(invalid());
        }

        let mut mapped: HashMap<ShareSheetDestination, RiseStoredSharePayloadResult> = HashMap::new();
        for result in &response.results {
            let destination = ShareSheetDestination::new(&result.chat_id, result.topic_id.as_deref());
            if mapped.insert(destination, result.clone()).is_some() {
                return Err(invalid());
            }
        }
        let expected: HashSet<&ShareSheetDestination> = batch.destinations.iter().collect();
        let answered: HashSet<&ShareSheetDestination> = mapped.keys().collect();
        if answered != expected {
            return Err(invalid());
        }
        Ok(mapped)
    }

    pub fn copy_link(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self.payload.as_ref().and_then(SharePayload::copied_link_text) else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(link));
        Notifier::copied(cx);
        if let Some(SharePayload::Post(post)) = self.payload.as_ref() {
            let post = post.clone();
            PostInteractionsStore::shared(cx)
                .update(cx, |posts, cx| posts.record_share(&post, ShareReceiptChannel::LinkCopy, cx));
        }
        self.dismiss(cx);
    }

    pub fn copy_text(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.payload.as_ref().and_then(SharePayload::copy_text) else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        Notifier::copied(cx);
        self.dismiss(cx);
    }

    pub fn system_share(&mut self, cx: &mut Context<Self>) {
        let Some(payload) = self.payload.clone() else { return };
        if self.external_share_generation.is_some() {
            return;
        }
        if let SharePayload::Messages(messages) = &payload {
            let messages = messages.clone();
            MessageExternalShareCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.present(messages, cx));
            return;
        }
        let items = payload.share_items();
        if items.is_empty() {
            return;
        }
        let generation = Uuid::new_v4();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        self.external_share_generation = Some(generation);

        let success_store = cx.weak_entity();
        let on_success: Box<dyn FnOnce(&mut App)> = Box::new(move |cx| {
            let is_current_share = success_store
                .read_with(cx, |store, _| store.external_share_generation == Some(generation))
                .unwrap_or(false);
            if !is_current_share
                || account_user_id.is_none()
                || account_epoch.is_none()
                || !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
            {
                return;
            }
            let SharePayload::Post(post) = payload else { return };
            PostInteractionsStore::shared(cx)
                .update(cx, |posts, cx| posts.record_share(&post, ShareReceiptChannel::External, cx));
        });
        let completion_store = cx.weak_entity();
        let completion: Box<dyn FnOnce(&mut App)> = Box::new(move |cx| {
            let _ = completion_store.update(cx, |store, _| {
                if store.external_share_generation == Some(generation) {
                    store.external_share_generation = None;
                }
            });
        });
        let presented = SystemShare::present_reporting(items, Some(on_success), Some(completion), cx);
        if !presented && self.external_share_generation == Some(generation) {
            self.external_share_generation = None;
        }
    }

    fn handle_send_result(
        &mut self,
        result: ShareSendResult<'_>,
        destination: &ShareSheetDestination,
        reason: Option<String>,
        context: &SendContextRef,
        cx: &mut Context<Self>,
    ) {
        if !self.is_active(context) || context.borrow().pending_count == 0 {
            return;
        }

        let (generation, total_count, failure_count, failure_reason) = {
            let mut send = context.borrow_mut();
            send.pending_count -= 1;
            match result {
                Err(error) => {
                    send.failed_destinations.insert(destination.clone());
                    if send.failure_reason.is_none() {
                        send.failure_reason = Self::presentable_reason(reason.as_deref())
                            .or_else(|| Self::presentable_reason_for(error));
                    }
                }
                Ok(()) => {
                    send.retry_key_by_destination.remove(destination);
                }
            }
            if send.pending_count != 0 {
                return;
            }
            (send.generation, send.total_count, send.failed_destinations.len(), send.failure_reason.clone())
        };

        self.active_sends.remove(&generation);
        tracing::warn!(
            target: "riseonly::X1057",
            total = total_count,
            failed = failure_count,
            has_reason = failure_reason.is_some(),
            "share send settled"
        );
        if failure_count == 0 {
            return;
        }

        let _ = notification(NotificationKind::Warning);
        let is_partial = total_count > failure_count;
        let delivered = i64::try_from(total_count - failure_count).unwrap_or(i64::MAX);
        let failed = i64::try_from(failure_count).unwrap_or(i64::MAX);
        let tally = tr_args("share_send_handoff_partial", &[FormatArg::Int(delivered), FormatArg::Int(failed)]);
        let content = match (is_partial, failure_reason) {
            (true, Some(reason)) => format!("{tally} {reason}"),
            (false, Some(reason)) => reason,
            (_, None) => tally,
        };
        let title = if is_partial {
            tr("share_send_handoff_partial_title")
        } else {
            tr("share_send_handoff_failed_title")
        };
        let options = NotifierSendOptions {
            content: Some(content),
            r#type: NotifierType::Warning,
            ..NotifierSendOptions::default()
        };
        Notifier::send(title, options, cx);
    }

    fn is_slow_mode(refusal: Option<&RiseFailure>) -> bool {
        let code = refusal.and_then(|failure| failure.code.as_deref());
        tracing::warn!(target: "riseonly::X1090", code = ?code, "share refusal without canon text");
        code == Some(ErrorCode::ChatSlowModeActive.wire())
    }

    fn slow_mode_reason(chat_id: &str, cx: &mut App) -> Option<String> {
        let remaining =
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.slow_mode_remaining_seconds(chat_id, cx));
        if remaining <= 0 {
            return Some(tr("chat_perm_slowmode"));
        }
        let details = HashMap::from([(ErrorParam::RetryAfterS.wire().to_owned(), remaining.to_string())]);
        CanonErrors::text(Some(ErrorCode::ChatSlowModeActive.wire()), &details)
            .or_else(|| Some(tr("chat_perm_slowmode")))
    }

    fn presentable_reason(raw: Option<&str>) -> Option<String> {
        let value = RiseMessageWireCodec::normalized(raw)?;
        let lowercased = value.to_lowercase();
        if value.chars().count() > MAX_PRESENTABLE_REASON_LENGTH
            || value.contains('\n')
            || lowercased.contains("status:")
            || lowercased.contains("transport error")
        {
            return None;
        }
        Some(value)
    }

    fn presentable_reason_for(error: &anyhow::Error) -> Option<String> {
        if let Some(failure) = CanonErrors::failure(error) {
            return CanonErrors::text_for(&failure);
        }
        if let Some(socket_error) = error.downcast_ref::<SaiWsSendError>() {
            return match socket_error {
                SaiWsSendError::Maintenance | SaiWsSendError::GuideSandbox => Some(socket_error.error_description()),
                SaiWsSendError::NotConnected | SaiWsSendError::EncodingFailed(_) | SaiWsSendError::SendFailed(_) => {
                    None
                }
            };
        }
        if error.downcast_ref::<std::io::Error>().is_some() || error.downcast_ref::<CocoaError>().is_some() {
            return None;
        }
        let described = error
            .downcast_ref::<ShareSendHandoffError>()
            .and_then(ShareSendHandoffError::error_description)
            .or_else(|| error.downcast_ref::<MessageForwardError>().and_then(MessageForwardError::error_description))?;
        Self::presentable_reason(Some(&described))
    }
}

const SYSTEM_SHARE_SOURCE_BOTTOM_INSET: f32 = 40.0;

pub enum SystemShare {}

impl SystemShare {
    pub fn present(items: Vec<ShareItem>, completion: Option<Box<dyn FnOnce(&mut App)>>, cx: &mut App) -> bool {
        Self::present_reporting(items, None, completion, cx)
    }

    /// presents items to share, no system sheet on linux so text goes to the clipboard
    /// notifier says so, false when no window is open to show it
    pub fn present_reporting(
        items: Vec<ShareItem>,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        completion: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut App,
    ) -> bool {
        let Some(window_handle) = cx
            .active_window()
            .or_else(|| cx.window_stack().and_then(|stack| stack.first().copied()))
            .or_else(|| cx.windows().first().copied())
        else {
            return false;
        };
        cx.defer(move |cx| {
            let (sender, receiver) = oneshot::channel::<ShareFinished>();
            let outcome = window_handle.update(cx, |_, window, _| {
                let viewport = window.viewport_size();
                let anchor = Bounds::new(
                    point(viewport.width / 2.0, viewport.height - px(SYSTEM_SHARE_SOURCE_BOTTOM_INSET)),
                    size(px(0.0), px(0.0)),
                );
                match ShareRequest::new(items, anchor) {
                    Ok(request) => share(window, &request, move |finished| {
                        let _ = sender.send(finished);
                    }),
                    Err(reason) => ShareOutcome::Unavailable { reason, copy: None },
                }
            });
            match outcome {
                Ok(ShareOutcome::Presented) => {
                    cx.spawn(async move |cx| {
                        let succeeded = receiver.await.is_ok_and(|finished| finished.succeeded);
                        cx.update(|cx| {
                            if succeeded && let Some(on_success) = on_success {
                                on_success(cx);
                            }
                            if let Some(completion) = completion {
                                completion(cx);
                            }
                        });
                    })
                    .detach();
                }
                Ok(ShareOutcome::Unavailable { copy: Some(text), .. }) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    Notifier::copied(cx);
                    if let Some(completion) = completion {
                        completion(cx);
                    }
                }
                Ok(ShareOutcome::Unavailable { copy: None, .. }) | Err(_) => {
                    if let Some(completion) = completion {
                        completion(cx);
                    }
                }
            }
        });
        true
    }
}

const SHEET_FRACTION_REST: f32 = 0.92;
const SHEET_FRACTION_EXPANDED: f32 = 0.97;
const RECENT_TARGETS_LIMIT: usize = 8;
const LOAD_MORE_TRAILING_ROWS: usize = 8;
const ACTION_TITLE_BASE_SIZE: f32 = 11.0;
const ACTION_TITLE_MINIMUM_SCALE: f32 = 0.7;
const ACTION_TITLE_HORIZONTAL_INSET: f32 = 8.0;
const DISABLED_CHROME_OPACITY: f32 = 0.55;
const SENDING_BAR_OPACITY: f32 = 0.72;
const DIVIDER_OPACITY: f32 = 0.48;
const DIVIDER_LEADING_INSET: f32 = 72.0;
const STATE_TOP_PADDING: f32 = 48.0;
const SHARE_TARGETS_SEARCH_DEBOUNCE: &str = "share_targets_search";
const KEY_ESCAPE: &str = "escape";
const KEY_UP: &str = "up";
const KEY_DOWN: &str = "down";
const KEY_ENTER: &str = "enter";

/// the share sheet, its state lives on the entity
/// root view makes it once and keeps it mounted, the store decides if its up
pub struct ShareSheet {
    store: Entity<ShareSheetStore>,
    target_services: Entity<ShareTargetsServicesStore>,
    socket: Entity<SaiWebSocketManager>,
    theme_store: Entity<ThemeStore>,
    sheet_fraction: f32,
    navigation_path: Vec<RiseShareTarget>,
    debouncing_search_query: Option<String>,
    is_selection_mode: bool,
    action_row_width: f32,
    highlighted_index: Option<usize>,
    saw_own_click: bool,
    focus_handle: FocusHandle,
    search_task: Option<(String, Task<()>)>,
    was_visible: bool,
    was_connected: bool,
    targets_failed: bool,
    content_view: Option<Entity<ShareSheetContentView>>,
    _subscriptions: Vec<Subscription>,
}

/// page body of the share sheet (browse, search topics) cached so sheet motion frames replay it
/// the sheet tells it about changes through ShareSheet changed
struct ShareSheetContentView {
    sheet: WeakEntity<ShareSheet>,
}

impl Render for ShareSheetContentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(sheet) = self.sheet.upgrade() else { return div().into_any_element() };
        gpui::hear_entity_through_events_only(sheet.entity_id());
        ShareSheet::navigation_content(&sheet, window, cx)
    }
}

enum ShareSheetKeyRow {
    Target(RiseShareTarget),
    Topic(RiseShareTopic),
}

impl ShareSheet {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = ShareSheetStore::shared(cx);
        let target_services = ShareTargetsServicesStore::shared(cx);
        let socket = SaiWebSocketManager::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        let subscriptions = vec![
            cx.observe(&store, |sheet, _, cx| sheet.store_changed(cx)),
            cx.observe(&target_services, |sheet, _, cx| sheet.targets_changed(cx)),
            cx.observe(&socket, |sheet, _, cx| sheet.socket_changed(cx)),
            cx.observe(&theme_store, |sheet, _, cx| sheet.changed(cx)),
        ];
        let was_visible = store.read(cx).is_visible();
        let was_connected = socket.read(cx).is_connected();
        let mut sheet = Self {
            store,
            target_services,
            socket,
            theme_store,
            sheet_fraction: SHEET_FRACTION_REST,
            navigation_path: Vec::new(),
            debouncing_search_query: None,
            is_selection_mode: false,
            action_row_width: 0.0,
            highlighted_index: None,
            saw_own_click: false,
            focus_handle: cx.focus_handle(),
            search_task: None,
            was_visible,
            was_connected,
            targets_failed: false,
            content_view: None,
            _subscriptions: subscriptions,
        };
        sheet.targets_failed = sheet.compute_targets_failed(cx);
        sheet
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.content_view {
            view.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    fn content_view(&mut self, cx: &mut Context<Self>) -> Entity<ShareSheetContentView> {
        if let Some(view) = &self.content_view {
            return view.clone();
        }
        let sheet = cx.weak_entity();
        let view = cx.new(|_| ShareSheetContentView { sheet });
        self.content_view = Some(view.clone());
        view
    }

    fn store_changed(&mut self, cx: &mut Context<Self>) {
        let is_visible = self.store.read(cx).is_visible();
        if is_visible != self.was_visible {
            self.was_visible = is_visible;
            if is_visible {
                self.sheet_fraction = SHEET_FRACTION_REST;
            } else {
                self.navigation_path.clear();
            }
            self.debouncing_search_query = None;
            self.is_selection_mode = false;
            self.highlighted_index = None;
            self.saw_own_click = false;
        }
        self.refresh_targets_failed(cx);
        self.changed(cx);
    }

    fn targets_changed(&mut self, cx: &mut Context<Self>) {
        self.refresh_targets_failed(cx);
        self.changed(cx);
    }

    fn socket_changed(&mut self, cx: &mut Context<Self>) {
        let connected = self.socket.read(cx).is_connected();
        let became_connected = connected && !self.was_connected;
        self.was_connected = connected;
        if became_connected && self.compute_targets_failed(cx) {
            self.store.update(cx, |store, cx| store.retry_failed_targets(true, cx));
        }
    }

    fn refresh_targets_failed(&mut self, cx: &mut Context<Self>) {
        let failed = self.compute_targets_failed(cx);
        let became_failed = failed && !self.targets_failed;
        self.targets_failed = failed;
        if became_failed {
            self.store.update(cx, |store, cx| store.retry_failed_targets(false, cx));
        }
    }

    fn compute_targets_failed(&self, cx: &mut App) -> bool {
        if !self.store.read(cx).is_visible() {
            return false;
        }
        let (has_media, has_link, has_documents) = self.permissions(cx);
        self.target_services
            .update(cx, |services, cx| {
                services.picker_targets_request_state(None, has_media, has_link, has_documents, cx)
            })
            .1
            .is_some()
    }

    fn permissions(&self, cx: &App) -> (bool, bool, bool) {
        let store = self.store.read(cx);
        (
            store.requires_media_permission(),
            store.requires_link_permission(),
            store.requires_documents_permission(),
        )
    }

    fn should_hide_footer(&self, cx: &App) -> bool {
        let store = self.store.read(cx);
        store.selected_destinations().is_empty() && (store.is_search_active() || !self.navigation_path.is_empty())
    }

    fn normalized_query(&self, cx: &App) -> String {
        trimming_whitespaces_and_newlines(self.store.read(cx).query()).to_owned()
    }

    fn base_targets(&self, cx: &mut App) -> Vec<RiseShareTarget> {
        self.picker_targets(None, cx)
    }

    fn picker_targets(&self, query: Option<&str>, cx: &mut App) -> Vec<RiseShareTarget> {
        let (has_media, has_link, has_documents) = self.permissions(cx);
        let query = query.filter(|query| !query.is_empty());
        let targets = self
            .target_services
            .update(cx, |services, cx| services.picker_targets(query, has_media, has_link, has_documents, cx));
        mark_deleted_peers(targets, cx)
    }

    fn filtered_topic_groups(&self, chat_id: &str, cx: &mut App) -> Vec<RiseShareTopicGroup<RiseShareTopic>> {
        let (has_media, has_link, has_documents) = self.permissions(cx);
        let structure = self.target_services.update(cx, |services, cx| {
            services.picker_topic_structure(chat_id, has_media, has_link, has_documents, cx)
        });
        let query = trimming_whitespaces_and_newlines(self.store.read(cx).topic_query()).to_lowercase();
        let topics: Vec<RiseShareTopic> = if query.is_empty() {
            structure.topics
        } else {
            structure.topics.into_iter().filter(|topic| topic_title(topic).to_lowercase().contains(&query)).collect()
        };
        RiseShareTopicGrouping::groups(
            &structure.sections,
            topics,
            |topic| topic.id.clone(),
            |topic| topic.section_id.clone(),
            |topic| topic.position,
            |topic| topic.is_general,
        )
    }

    fn sync_search_task(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let is_on_screen = store.is_visible() && store.is_search_active() && self.navigation_path.is_empty();
        if !is_on_screen {
            self.cancel_search_task();
            return;
        }
        let request_query = self.normalized_query(cx);
        let request_key = request_query.to_lowercase();
        if self.search_task.as_ref().is_some_and(|(key, _)| *key == request_key) {
            return;
        }
        self.cancel_search_task();
        if request_key.is_empty() {
            self.debouncing_search_query = None;
            self.search_task = Some((request_key, Task::ready(())));
            return;
        }
        if self.picker_targets(Some(&request_query), cx).is_empty() {
            self.debouncing_search_query = Some(request_key.clone());
        }
        let debounce = Duration::from_nanos(
            RiseTimingCanon::debounce(SHARE_TARGETS_SEARCH_DEBOUNCE).client_debounce_nanoseconds(),
        );
        let task_key = request_key.clone();
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(debounce).await;
            let _ = this.update(cx, |sheet, cx| sheet.finish_search_debounce(&task_key, &request_query, cx));
        });
        self.search_task = Some((request_key, task));
    }

    fn cancel_search_task(&mut self) {
        if let Some((key, _task)) = self.search_task.take()
            && self.debouncing_search_query.as_deref() == Some(key.as_str())
        {
            self.debouncing_search_query = None;
        }
    }

    fn finish_search_debounce(&mut self, request_key: &str, request_query: &str, cx: &mut Context<Self>) {
        if self.normalized_query(cx).to_lowercase() == request_key {
            let (has_media, has_link, has_documents) = self.permissions(cx);
            ShareTargetsInteractionsStore::shared(cx).update(cx, |targets, cx| {
                targets.load_picker_targets(Some(request_query), has_media, has_link, has_documents, cx);
            });
        }
        if self.debouncing_search_query.as_deref() == Some(request_key) {
            self.debouncing_search_query = None;
        }
        self.changed(cx);
    }

    fn set_active_query(&mut self, value: String, cx: &mut Context<Self>) {
        let previous_key = self.normalized_query(cx).to_lowercase();
        let key = trimming_whitespaces_and_newlines(&value).to_lowercase();
        self.store.update(cx, |store, cx| store.set_query(value, cx));
        if key == previous_key {
            return;
        }
        self.debouncing_search_query = if key.is_empty() { None } else { Some(key) };
        self.highlighted_index = None;
        self.changed(cx);
    }

    fn activate_search(&mut self, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.set_search_active(true, cx));
        self.sheet_fraction = SHEET_FRACTION_EXPANDED;
        self.highlighted_index = None;
        self.changed(cx);
    }

    fn deactivate_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        self.store.update(cx, |store, cx| {
            store.set_query(String::new(), cx);
            store.set_search_active(false, cx);
        });
        self.sheet_fraction = SHEET_FRACTION_REST;
        self.highlighted_index = None;
        self.changed(cx);
    }

    /// false for a follow up click of a multi click that started before the sheet showed
    fn accepts_click(&mut self, click_count: usize) -> bool {
        if click_count > 1 && !self.saw_own_click {
            return false;
        }
        self.saw_own_click = true;
        true
    }

    fn select_or_open(&mut self, target: RiseShareTarget, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.read(cx).is_sending() {
            return;
        }
        let head: Vec<String> = self.picker_targets(None, cx).into_iter().take(8).map(|target| target.id).collect();
        tracing::warn!(target: "riseonly::X640", "select {} selection_mode {} search {} head {:?}", target.id, self.is_selection_mode, self.store.read(cx).is_search_active(), head);
        if target.topics_enabled {
            window.focus(&self.focus_handle, cx);
            self.store.update(cx, |store, cx| store.set_topic_query(String::new(), cx));
            self.load_topics(&target.id, cx);
            self.sheet_fraction = SHEET_FRACTION_EXPANDED;
            self.navigation_path.push(target);
            self.highlighted_index = None;
        } else {
            self.is_selection_mode = true;
            self.store.update(cx, |store, cx| store.toggle(&target.id, None, cx));
        }
        self.changed(cx);
    }

    fn toggle_topic(&mut self, topic: &RiseShareTopic, cx: &mut Context<Self>) {
        self.is_selection_mode = true;
        self.store.update(cx, |store, cx| store.toggle(&topic.chat_id, Some(topic.id.as_str()), cx));
        self.changed(cx);
    }

    fn load_topics(&self, chat_id: &str, cx: &mut Context<Self>) {
        let (has_media, has_link, has_documents) = self.permissions(cx);
        ShareTargetsInteractionsStore::shared(cx).update(cx, |targets, cx| {
            targets.load_picker_topics(chat_id, has_media, has_link, has_documents, cx);
        });
    }

    fn toggle_selection_mode(&mut self, cx: &mut Context<Self>) {
        if self.store.read(cx).is_sending() {
            return;
        }
        if self.is_selection_mode {
            self.store.update(cx, |store, cx| store.clear_selection(cx));
            self.is_selection_mode = false;
        } else {
            self.is_selection_mode = true;
        }
        self.changed(cx);
    }

    fn pop_topic_page(&mut self, cx: &mut Context<Self>) {
        if self.navigation_path.pop().is_none() {
            return;
        }
        self.store.update(cx, |store, cx| store.set_topic_query(String::new(), cx));
        self.highlighted_index = None;
        self.changed(cx);
    }

    fn key_rows(&self, cx: &mut App) -> Vec<ShareSheetKeyRow> {
        if let Some(target) = self.navigation_path.last() {
            return self
                .filtered_topic_groups(&target.id, cx)
                .into_iter()
                .flat_map(|group| group.topics)
                .map(ShareSheetKeyRow::Topic)
                .collect();
        }
        let query = if self.store.read(cx).is_search_active() { Some(self.normalized_query(cx)) } else { None };
        self.picker_targets(query.as_deref(), cx).into_iter().map(ShareSheetKeyRow::Target).collect()
    }

    fn handle_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match key {
            KEY_ESCAPE => {
                if !self.navigation_path.is_empty() {
                    self.pop_topic_page(cx);
                    return true;
                }
                if self.store.read(cx).is_search_active() {
                    self.deactivate_search(window, cx);
                    return true;
                }
                false
            }
            KEY_UP | KEY_DOWN => {
                let count = self.key_rows(cx).len();
                if count == 0 {
                    return false;
                }
                let next = match (self.highlighted_index, key == KEY_DOWN) {
                    (None, true) => 0,
                    (None, false) => count - 1,
                    (Some(index), true) => (index + 1).min(count - 1),
                    (Some(index), false) => index.saturating_sub(1),
                };
                self.highlighted_index = Some(next);
                self.changed(cx);
                true
            }
            KEY_ENTER => {
                let Some(index) = self.highlighted_index else { return false };
                match self.key_rows(cx).into_iter().nth(index) {
                    Some(ShareSheetKeyRow::Target(target)) => self.select_or_open(target, window, cx),
                    Some(ShareSheetKeyRow::Topic(topic)) => self.toggle_topic(&topic, cx),
                    None => return false,
                }
                true
            }
            _ => false,
        }
    }

    fn navigation_content(this: &Entity<Self>, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        this.update(cx, |sheet, cx| {
            let page = match sheet.navigation_path.last() {
                Some(target) => sheet.topics_content(this, target, &theme, cx),
                None if sheet.store.read(cx).is_search_active() => sheet.search_content(this, &theme, window, cx),
                None => sheet.browse_content(this, &theme, cx),
            };
            let key_sheet = this.clone();
            div()
                .id("share-sheet-content")
                .size_full()
                .flex()
                .flex_col()
                .bg(theme.bg._100)
                .track_focus(&sheet.focus_handle)
                .on_key_down(move |event, window, cx| {
                    let key = event.keystroke.key.clone();
                    if key_sheet.update(cx, |sheet, cx| sheet.handle_key(&key, window, cx)) {
                        cx.stop_propagation();
                    }
                })
                .child(page)
                .into_any_element()
        })
    }

    fn browse_content(&self, this: &Entity<Self>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let activate_sheet = this.clone();
        let query = Binding::entity(
            self.store.clone(),
            |store: &ShareSheetStore| store.query().to_owned(),
            |store: &mut ShareSheetStore, value: String, cx: &mut Context<ShareSheetStore>| store.set_query(value, cx),
        );
        vstack(0.0)
            .size_full()
            .child(self.root_header(this, theme, cx))
            .child(
                div().w_full().px(px(14.0)).pb(px(10.0)).child(
                    SearchFieldUi::new("share-sheet-search-rest", query)
                        .placeholder(SharedString::from(tr("search_placeholder")))
                        .on_activate(move |_, cx| activate_sheet.update(cx, |sheet, cx| sheet.activate_search(cx))),
                ),
            )
            .child(self.target_list(this, None, theme, cx))
            .into_any_element()
    }

    fn search_content(&self, this: &Entity<Self>, theme: &AppTheme, _window: &mut Window, cx: &mut App) -> AnyElement {
        let reader = self.store.clone();
        let writer = this.clone();
        let active_query = Binding::new(
            move |cx| reader.read(cx).query().to_owned(),
            move |value: String, _, cx| writer.update(cx, |sheet, cx| sheet.set_active_query(value, cx)),
        );
        let cancel_sheet = this.clone();
        let escape_sheet = this.clone();
        let normalized_query = self.normalized_query(cx);
        let mut content = vstack(0.0).size_full().child(
            hstack(10.0)
                .w_full()
                .px(px(12.0))
                .pt(px(6.0))
                .pb(px(8.0))
                .child(
                    div().flex_1().min_w_0().child(
                        SearchFieldUi::new("share-sheet-search-active", active_query)
                            .placeholder(SharedString::from(tr("search_placeholder")))
                            .on_cancel(move |window, cx| {
                                escape_sheet.update(cx, |sheet, cx| sheet.deactivate_search(window, cx));
                            })
                            .uses_inline_text_field(true)
                            .auto_focus(true),
                    ),
                )
                .child(
                    LiquidGlassButton::new("share-sheet-search-cancel", LiquidGlassButtonShape::Circle)
                        .size(42.0)
                        .accessibility_label(Some(tr("cancel")))
                        .on_press(move |window, cx| {
                            cancel_sheet.update(cx, |sheet, cx| sheet.deactivate_search(window, cx));
                        })
                        .child(symbol("xmark", 15.0, theme.text.secondary)),
                ),
        );
        if normalized_query.is_empty() {
            content = content.child(self.recent_targets(this, theme, cx));
        }
        content.child(self.target_list(this, Some(&normalized_query), theme, cx)).into_any_element()
    }

    fn root_header(&self, this: &Entity<Self>, theme: &AppTheme, cx: &App) -> AnyElement {
        let is_sending = self.store.read(cx).is_sending();
        let chrome_opacity = if is_sending { DISABLED_CHROME_OPACITY } else { 1.0 };
        let dismiss_store = self.store.clone();
        let mode_sheet = this.clone();
        let mode_title = if self.is_selection_mode { tr("cancel") } else { tr("ctx_select") };
        let mode_color = if self.is_selection_mode { theme.text.secondary } else { theme.text.primary };
        hstack(10.0)
            .w_full()
            .px(px(14.0))
            .pt(px(5.0))
            .pb(px(12.0))
            .child(
                div().flex_none().opacity(chrome_opacity).child(
                    LiquidGlassButton::new("share-sheet-close", LiquidGlassButtonShape::Circle)
                        .size(42.0)
                        .disabled(is_sending)
                        .accessibility_label(Some(tr("cancel")))
                        .on_press(move |_, cx| dismiss_store.update(cx, |store, cx| store.dismiss(cx)))
                        .child(symbol("xmark", 16.0, theme.text.primary)),
                ),
            )
            .child(
                div().flex_1().min_w_0().child(
                    MainText::new(tr("share_sheet_title"))
                        .px(18.0)
                        .tac(TextAlignT::Center)
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(1)
                        .minimum_scale_factor(0.75),
                ),
            )
            .child(
                div().flex_none().opacity(chrome_opacity).child(
                    LiquidGlassButton::new("share-sheet-selection-mode", LiquidGlassButtonShape::Capsule)
                        .size(42.0)
                        .horizontal_padding(14.0)
                        .vertical_padding(10.0)
                        .disabled(is_sending)
                        .accessibility_label(Some(mode_title.clone()))
                        .on_press(move |_, cx| mode_sheet.update(cx, |sheet, cx| sheet.toggle_selection_mode(cx)))
                        .child(
                            MainText::new(mode_title)
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(mode_color)
                                .number_of_lines(1),
                        ),
                ),
            )
            .into_any_element()
    }

    fn recent_targets(&self, this: &Entity<Self>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let recents: Vec<RiseShareTarget> = self.base_targets(cx).into_iter().take(RECENT_TARGETS_LIMIT).collect();
        if recents.is_empty() {
            return div().into_any_element();
        }
        let store = self.store.read(cx);
        let mut strip = hstack(14.0).px(px(14.0)).py(px(6.0));
        for target in recents {
            let is_selected = store.selected_count_in(&target.id) > 0;
            let title = ShareTargetPresentation::title(&target);
            let mut avatar = zstack(Alignment::BottomTrailing).child(target_avatar(&target, 56.0));
            if is_selected {
                avatar = avatar.child(selection_indicator(true, theme, 20.0));
            }
            let press_sheet = this.clone();
            let press_target = target.clone();
            strip = strip.child(
                div()
                    .id(ElementId::Name(format!("share-recent-{}", target.id).into()))
                    .cursor_pointer()
                    .on_click(move |event, window, cx| {
                        let target = press_target.clone();
                        press_sheet.update(cx, |sheet, cx| {
                            if sheet.accepts_click(event.click_count()) {
                                sheet.select_or_open(target, window, cx);
                            }
                        });
                    })
                    .child(
                        vstack(5.0).items_center().child(avatar).child(
                            div()
                                .w(px(64.0))
                                .child(MainText::new(title).px(11.0).tac(TextAlignT::Center).number_of_lines(1)),
                        ),
                    ),
            );
        }
        vstack(2.0)
            .w_full()
            .items_start()
            .h(px(112.0))
            .flex_none()
            .child(
                div().px(px(14.0)).child(
                    MainText::new(tr("chat_search_overlay_recent"))
                        .px(12.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(theme.text.secondary),
                ),
            )
            .child(div().id("share-sheet-recents").w_full().overflow_x_scroll().child(strip))
            .into_any_element()
    }

    fn target_list(&self, this: &Entity<Self>, query: Option<&str>, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let (has_media, has_link, has_documents) = self.permissions(cx);
        let targets = self.picker_targets(query, cx);
        let (is_pending, error) = self.target_services.update(cx, |services, cx| {
            services.picker_targets_request_state(query, has_media, has_link, has_documents, cx)
        });
        let query_key = query.map(|query| trimming_whitespaces_and_newlines(query).to_lowercase()).unwrap_or_default();
        let is_debouncing =
            !query_key.is_empty() && self.debouncing_search_query.as_deref() == Some(query_key.as_str());
        let is_scanning_older_chats = !query_key.is_empty()
            && self.target_services.update(cx, |services, cx| {
                services.is_picker_search_scanning(query, has_media, has_link, has_documents, cx)
            });

        if is_debouncing || (targets.is_empty() && (is_pending || is_scanning_older_chats)) {
            return loading_state();
        }
        if targets.is_empty() && error.is_some() {
            let retry_query = query.map(str::to_owned);
            return error_state(theme, move |_, cx| {
                ShareTargetsInteractionsStore::shared(cx).update(cx, |targets, cx| {
                    targets.load_picker_targets(retry_query.as_deref(), has_media, has_link, has_documents, cx);
                });
            });
        }
        if targets.is_empty() {
            return empty_state(theme, tr("share_empty"));
        }

        let page_query = if query_key.is_empty() { None } else { query.map(str::to_owned) };
        let count = targets.len();
        let mut rows = vstack(0.0).w_full();
        for (index, target) in targets.into_iter().enumerate() {
            let row_id = ElementId::Name(format!("share-target-row-{}", target.id).into());
            let row = self.target_row(this, &target, self.highlighted_index == Some(index), theme, cx);
            let more_query = page_query.clone();
            rows = rows.child(row.on_appear(row_id, move |_, cx| {
                if index + LOAD_MORE_TRAILING_ROWS < count {
                    return;
                }
                ShareTargetsInteractionsStore::shared(cx).update(cx, |targets, cx| {
                    targets.load_more_picker_targets(more_query.as_deref(), has_media, has_link, has_documents, 0, cx);
                });
            }));
            if index + 1 < count {
                rows = rows.child(row_divider(theme));
            }
        }
        rows = rows.child(spacer_min_h(18.0));
        let list_id = if query_key.is_empty() { "share-sheet-targets" } else { "share-sheet-search-targets" };
        div()
            .w_full()
            .flex_1()
            .min_h_0()
            .child(scroll_view(list_id).shows_indicators(false).child(rows))
            .into_any_element()
    }

    fn target_row(
        &self,
        this: &Entity<Self>,
        target: &RiseShareTarget,
        is_highlighted: bool,
        theme: &AppTheme,
        cx: &App,
    ) -> AnyElement {
        let selected_count = self.store.read(cx).selected_count_in(&target.id);
        let mut row = hstack(12.0)
            .w_full()
            .px(px(14.0))
            .py(px(9.0))
            .child(target_avatar(target, 50.0))
            .child(
                div().flex_1().min_w_0().child(
                    MainText::new(ShareTargetPresentation::title(target))
                        .px(16.0)
                        .font_weight(FontWeight::MEDIUM)
                        .number_of_lines(1),
                ),
            );
        if target.topics_enabled {
            if selected_count > 0 {
                row = row.child(
                    div()
                        .size(px(24.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme.primary._100)
                        .child(
                            MainText::new(selected_count.to_string())
                                .px(12.0)
                                .font_weight(FontWeight::BOLD)
                                .color(theme.content_on(theme.primary._100)),
                        ),
                );
            }
            row = row.child(symbol(rise_ui::IconUi::trailing_chevron(), 13.0, theme.text.secondary));
        } else if self.is_selection_mode || selected_count > 0 {
            row = row.child(selection_indicator(selected_count > 0, theme, 24.0));
        }
        let press_sheet = this.clone();
        let press_target = target.clone();
        div()
            .id(ElementId::Name(format!("share-target-{}", target.id).into()))
            .w_full()
            .cursor_pointer()
            .when(is_highlighted, |row| row.bg(theme.bg._200))
            .on_click(move |event, window, cx| {
                let target = press_target.clone();
                press_sheet.update(cx, |sheet, cx| {
                    if sheet.accepts_click(event.click_count()) {
                        sheet.select_or_open(target, window, cx);
                    }
                });
            })
            .child(row)
            .into_any_element()
    }

    fn topics_content(&self, this: &Entity<Self>, target: &RiseShareTarget, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let topic_query = Binding::entity(
            self.store.clone(),
            |store: &ShareSheetStore| store.topic_query().to_owned(),
            |store: &mut ShareSheetStore, value: String, cx: &mut Context<ShareSheetStore>| {
                store.set_topic_query(value, cx);
            },
        );
        let load_sheet = this.clone();
        let chat_id = target.id.clone();
        vstack(0.0)
            .size_full()
            .bg(theme.bg._100)
            .child(self.topic_header(this, target, theme))
            .child(
                div().w_full().px(px(14.0)).pb(px(10.0)).child(
                    SearchFieldUi::new("share-sheet-topic-search", topic_query)
                        .placeholder(SharedString::from(tr("search_placeholder")))
                        .uses_inline_text_field(true),
                ),
            )
            .child(self.topic_list(this, target, theme, cx))
            .on_appear(ElementId::Name(format!("share-topics-{}", target.id).into()), move |_, cx| {
                load_sheet.update(cx, |sheet, cx| sheet.load_topics(&chat_id, cx));
            })
            .into_any_element()
    }

    fn topic_header(&self, this: &Entity<Self>, target: &RiseShareTarget, theme: &AppTheme) -> AnyElement {
        let back_sheet = this.clone();
        div()
            .relative()
            .w_full()
            .px(px(14.0))
            .pt(px(5.0))
            .pb(px(12.0))
            .child(
                div().h(px(42.0)).px(px(64.0)).flex().items_center().justify_center().child(
                    MainText::new(ShareTargetPresentation::title(target))
                        .px(18.0)
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(1),
                ),
            )
            .child(
                div().absolute().left(px(14.0)).top(px(5.0)).child(
                    LiquidGlassButton::new("share-sheet-topics-back", LiquidGlassButtonShape::Circle)
                        .size(42.0)
                        .accessibility_label(Some(tr("common_back")))
                        .on_press(move |_, cx| back_sheet.update(cx, |sheet, cx| sheet.pop_topic_page(cx)))
                        .child(symbol("chevron.backward", 18.0, theme.text.primary)),
                ),
            )
            .into_any_element()
    }

    fn topic_list(&self, this: &Entity<Self>, target: &RiseShareTarget, theme: &AppTheme, cx: &mut App) -> AnyElement {
        let (has_media, has_link, has_documents) = self.permissions(cx);
        let groups = self.filtered_topic_groups(&target.id, cx);
        let (is_pending, error) = self.target_services.update(cx, |services, cx| {
            services.picker_topics_request_state(&target.id, has_media, has_link, has_documents, cx)
        });

        if groups.is_empty() && is_pending {
            return loading_state();
        }
        if groups.is_empty() && error.is_some() {
            let chat_id = target.id.clone();
            return error_state(theme, move |_, cx| {
                ShareTargetsInteractionsStore::shared(cx).update(cx, |targets, cx| {
                    targets.load_picker_topics(&chat_id, has_media, has_link, has_documents, cx);
                });
            });
        }
        if groups.is_empty() {
            return empty_state(theme, tr("chat_search_no_results"));
        }

        let mut rows = vstack(0.0).w_full();
        let mut key_index = 0usize;
        for group in groups {
            if let Some(section) = &group.section {
                rows = rows.child(
                    div().w_full().px(px(18.0)).pt(px(14.0)).pb(px(6.0)).child(
                        MainText::new(section.name.to_uppercase())
                            .px(12.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.text.secondary)
                            .number_of_lines(1),
                    ),
                );
            }
            let topic_count = group.topics.len();
            for (index, topic) in group.topics.iter().enumerate() {
                rows = rows.child(self.topic_row(this, topic, self.highlighted_index == Some(key_index), theme, cx));
                key_index += 1;
                if index + 1 < topic_count {
                    rows = rows.child(row_divider(theme));
                }
            }
        }
        rows = rows.child(spacer_min_h(18.0));
        div()
            .w_full()
            .flex_1()
            .min_h_0()
            .child(scroll_view("share-sheet-topics").shows_indicators(false).child(rows))
            .into_any_element()
    }

    fn topic_row(
        &self,
        this: &Entity<Self>,
        topic: &RiseShareTopic,
        is_highlighted: bool,
        theme: &AppTheme,
        cx: &App,
    ) -> AnyElement {
        let selected = self.store.read(cx).is_selected(&topic.chat_id, Some(topic.id.as_str()));
        let icon = trimming_whitespaces_and_newlines(&topic.icon).to_owned();
        let glyph: AnyElement = if icon.is_empty() {
            MainText::new("#")
                .px(19.0)
                .font_weight(FontWeight::SEMIBOLD)
                .color(theme.text.secondary)
                .into_any_element()
        } else {
            TopicIconGlyph::new(icon).size(24.0).into_any_element()
        };
        let press_sheet = this.clone();
        let press_topic = topic.clone();
        div()
            .id(ElementId::Name(format!("share-topic-{}-{}", topic.chat_id, topic.id).into()))
            .w_full()
            .cursor_pointer()
            .when(is_highlighted, |row| row.bg(theme.bg._200))
            .on_click(move |event, _, cx| {
                press_sheet.update(cx, |sheet, cx| {
                    if sheet.accepts_click(event.click_count()) {
                        sheet.toggle_topic(&press_topic, cx);
                    }
                })
            })
            .child(
                hstack(12.0)
                    .w_full()
                    .px(px(14.0))
                    .py(px(7.0))
                    .child(div().size(px(46.0)).flex().items_center().justify_center().child(glyph))
                    .child(
                        div().flex_1().min_w_0().child(
                            MainText::new(topic_title(topic))
                                .px(16.0)
                                .font_weight(FontWeight::MEDIUM)
                                .number_of_lines(1),
                        ),
                    )
                    .child(selection_indicator(selected, theme, 24.0)),
            )
            .into_any_element()
    }

    fn footer(this: &Entity<Self>, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = rise_ui::theme(cx).clone();
        let sheet = this.read(cx);
        if sheet.store.read(cx).selected_destinations().is_empty() {
            sheet.action_row(this, &theme, window, cx)
        } else {
            sheet.send_bar(&theme, cx)
        }
    }

    fn actions(&self, cx: &App) -> Vec<(&'static str, String, ShareSheetAction)> {
        let payload = self.store.read(cx).payload();
        let mut actions = Vec::new();
        if let Some(SharePayload::Post(post)) = payload {
            let title = if post.is_reposted { tr("repost_delete_action") } else { tr("repost_action") };
            actions.push(("arrow.2.squarepath", title, ShareSheetAction::Repost(post.clone())));
        }
        if payload.is_some_and(|payload| payload.share_url().is_some()) {
            actions.push(("link", tr("share_action_copy_link"), ShareSheetAction::CopyLink));
        } else if payload.is_some_and(|payload| payload.copy_text().is_some()) {
            actions.push(("doc.on.doc", tr("ctx_copy"), ShareSheetAction::CopyText));
        }
        actions.push(("square.and.arrow.up", tr("share_action_more"), ShareSheetAction::SystemShare));
        actions
    }

    fn action_row(&self, this: &Entity<Self>, theme: &AppTheme, window: &Window, cx: &App) -> AnyElement {
        let actions = self.actions(cx);
        let titles: Vec<String> = actions.iter().map(|(_, title, _)| title.clone()).collect();
        let text_delta = self.theme_store.read(cx).text_delta();
        let title_size = action_title_size(&titles, self.action_row_width, text_delta, window);
        let mut row = hstack(0.0).items_start().w_full();
        for (index, (icon, title, action)) in actions.into_iter().enumerate() {
            let store = self.store.clone();
            row = row.child(
                div()
                    .id(("share-sheet-action", index))
                    .flex_1()
                    .min_w_0()
                    .min_h(px(52.0))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| action.perform(&store, cx))
                    .child(
                        vstack(4.0)
                            .items_center()
                            .child(div().h(px(28.0)).flex().items_center().child(symbol(icon, 20.0, theme.text.primary)))
                            .child(
                                env_scope::<IgnoresTextSizeDeltaKey>(true).child(
                                    div().px(px(4.0)).child(
                                        MainText::new(title)
                                            .px(title_size)
                                            .tac(TextAlignT::Center)
                                            .font_weight(FontWeight::MEDIUM)
                                            .number_of_lines(2),
                                    ),
                                ),
                            ),
                    ),
            );
        }
        let width_sheet = this.clone();
        div()
            .w_full()
            .px(px(10.0))
            .py(px(8.0))
            .child(
                div().relative().w_full().child(row).child(measure("share-sheet-action-row", move |bounds, _, cx| {
                    let width = f32::from(bounds.size.width);
                    width_sheet.update(cx, |sheet, cx| {
                        if (sheet.action_row_width - width).abs() > 0.5 {
                            sheet.action_row_width = width;
                            sheet.changed(cx);
                        }
                    });
                })),
            )
            .into_any_element()
    }

    fn send_bar(&self, theme: &AppTheme, cx: &App) -> AnyElement {
        let store = self.store.read(cx);
        let is_sending = store.is_sending();
        let label = format!("{} ({})", tr("share_send"), store.selected_count());
        let leading: AnyElement = if is_sending {
            progress_view().tint(theme.content_on(theme.primary._100)).into_any_element()
        } else {
            symbol("paperplane.fill", 16.0, theme.content_on(theme.primary._100))
        };
        let send_store = self.store.clone();
        div()
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .child(
                div()
                    .id("share-sheet-send")
                    .w_full()
                    .py(px(14.0))
                    .rounded_full()
                    .bg(theme.primary._100)
                    .opacity(if is_sending { SENDING_BAR_OPACITY } else { 1.0 })
                    .when(!is_sending, |bar| {
                        bar.cursor_pointer()
                            .on_click(move |_, _, cx| send_store.update(cx, |store, cx| store.send_to_selected(cx)))
                    })
                    .child(
                        hstack(8.0)
                            .w_full()
                            .justify_center()
                            .child(leading)
                            .child(MainText::new(label).px(15.0).font_weight(FontWeight::SEMIBOLD).color(theme.content_on(theme.primary._100))),
                    ),
            )
            .into_any_element()
    }
}

impl Render for ShareSheet {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_search_task(cx);
        let theme = rise_ui::theme(cx).clone();
        let this = cx.entity();
        let is_sending = self.store.read(cx).is_sending();
        let hides_footer = self.should_hide_footer(cx);
        let reader = self.store.clone();
        let writer = self.store.clone();
        let visible = Binding::new(
            move |cx| reader.read(cx).is_visible(),
            move |is_visible: bool, _, cx| {
                writer.update(cx, |store, cx| {
                    if is_visible {
                        store.set_visible(true, cx);
                    } else {
                        store.dismiss(cx);
                    }
                });
            },
        );
        let fraction = Binding::entity(
            this.clone(),
            |sheet: &ShareSheet| sheet.sheet_fraction,
            |sheet: &mut ShareSheet, value: f32, _: &mut Context<ShareSheet>| sheet.sheet_fraction = value,
        );
        let content_view = self.content_view(cx);
        let footer_sheet = this;
        let dismiss_store = self.store.clone();
        BottomSheetUi::new(
            "share-sheet",
            visible,
            move |_, _| content_view.clone().cached(StyleRefinement::default().size_full()),
            move |window, cx| Self::footer(&footer_sheet, window, cx),
        )
        .snap_fractions(vec![SHEET_FRACTION_REST, SHEET_FRACTION_EXPANDED])
        .selected_snap_fraction(Some(fraction))
        .experimental_bottom_sheet(true)
        .experimental_uses_liquid_glass_surface(false)
        .full_page_allows_swipe_dismiss(!is_sending)
        .full_page_hides_footer(hides_footer)
        .full_page_avoids_keyboard_when_footer_hidden(true)
        .footer_shows_top_border(false)
        .footer_overlays_content(Some(false))
        .footer_horizontal_padding(Some(0.0))
        .footer_top_padding(Some(0.0))
        .content_scrollable(true)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
        .on_dismiss(move |_, cx| dismiss_store.update(cx, |store, cx| store.finish_dismissal(cx)))
    }
}

enum ShareSheetAction {
    Repost(GetPostFeedResponse),
    CopyLink,
    CopyText,
    SystemShare,
}

impl ShareSheetAction {
    fn perform(&self, store: &Entity<ShareSheetStore>, cx: &mut App) {
        match self {
            Self::Repost(post) => {
                store.update(cx, |store, cx| store.dismiss(cx));
                PostInteractionsStore::shared(cx).update(cx, |posts, cx| posts.toggle_repost(post, None, cx));
            }
            Self::CopyLink => store.update(cx, |store, cx| store.copy_link(cx)),
            Self::CopyText => store.update(cx, |store, cx| store.copy_text(cx)),
            Self::SystemShare => store.update(cx, |store, cx| store.system_share(cx)),
        }
    }
}

fn topic_title(topic: &RiseShareTopic) -> String {
    if topic.is_general { tr("chat_topics_general") } else { topic.title.clone() }
}

fn action_title_size(titles: &[String], row_width: f32, text_delta: f32, window: &Window) -> f32 {
    let base = (ACTION_TITLE_BASE_SIZE + text_delta).max(1.0);
    if row_width <= 0.0 || titles.is_empty() {
        return base;
    }
    let column = row_width / titles.len() as f32 - ACTION_TITLE_HORIZONTAL_INSET;
    let widest = widest_word_width(titles, base, FontWeight::MEDIUM, window);
    if widest <= column || widest <= 0.0 {
        return base;
    }
    let scale = ACTION_TITLE_MINIMUM_SCALE.max(column / widest * 0.98);
    (base * scale * 2.0).floor() / 2.0
}

fn widest_word_width(texts: &[String], size: f32, weight: FontWeight, window: &Window) -> f32 {
    let face = gpui::Font { weight, ..gpui::font(FONT_FAMILY) };
    let mut widest = 0.0_f32;
    for text in texts {
        for word in text.split(|c: char| !(c.is_alphanumeric() || c == '\'')).filter(|word| !word.is_empty()) {
            let runs = [gpui::TextRun {
                len: word.len(),
                font: face.clone(),
                color: black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let width =
                window.text_system().shape_line(SharedString::from(word.to_owned()), px(size), &runs, None).width();
            widest = widest.max(f32::from(width));
        }
    }
    widest
}

fn mark_deleted_peers(targets: Vec<RiseShareTarget>, cx: &mut App) -> Vec<RiseShareTarget> {
    let chats = ChatServicesStore::shared(cx);
    targets
        .into_iter()
        .map(|mut target| {
            if target.kind == RiseShareTargetKind::PrivateChat
                && !target.is_deleted
                && chats
                    .read(cx)
                    .chat_info(&target.id, cx)
                    .is_some_and(|chat| chat.r#type == ChatType::Direct && chat.participant.is_deleted)
            {
                target.is_deleted = true;
                target.title = tr("chat_deleted_account");
                target.avatar_url = None;
            }
            target
        })
        .collect()
}

fn target_avatar(target: &RiseShareTarget, side: f32) -> AnyElement {
    if ShareTargetPresentation::is_favourites(target) {
        return ChatFavouritesLogo::new(side).into_any_element();
    }
    let color_key = target.avatar_color_key.clone().unwrap_or_else(|| target.id.clone());
    UserLogoUi::new(ElementId::Name(format!("share-target-avatar-{}-{side}", target.id).into()))
        .source(target.avatar_url.clone().map(SharedString::from))
        .name(Some(SharedString::from(target.title.clone())))
        .user_id(Some(SharedString::from(color_key)))
        .color_scope_id(Some(SharedString::from(target.id.clone())))
        .is_deleted(target.is_deleted)
        .size(side)
        .show_story_ring(false)
        .into_any_element()
}

fn selection_indicator(selected: bool, theme: &AppTheme, side: f32) -> AnyElement {
    let ring = div().size(px(side)).flex().items_center().justify_center().rounded_full();
    if selected {
        ring.bg(theme.primary._100).child(symbol("checkmark", side * 0.48, theme.content_on(theme.primary._100))).into_any_element()
    } else {
        ring.relative().child(centred_stroke(theme.border._200, 1.5, Shape::Circle)).into_any_element()
    }
}

fn row_divider(theme: &AppTheme) -> AnyElement {
    div()
        .pl(px(DIVIDER_LEADING_INSET))
        .child(div().h(px(0.5)).w_full().bg(theme.border._200.opacity(DIVIDER_OPACITY)))
        .into_any_element()
}

fn loading_state() -> AnyElement {
    div().size_full().flex().justify_center().pt(px(STATE_TOP_PADDING)).child(progress_view()).into_any_element()
}

fn empty_state(theme: &AppTheme, text: String) -> AnyElement {
    vstack(10.0)
        .size_full()
        .items_center()
        .pt(px(STATE_TOP_PADDING))
        .child(symbol("bubble.left.and.bubble.right", 34.0, theme.text.secondary))
        .child(MainText::new(text).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary))
        .into_any_element()
}

fn error_state(theme: &AppTheme, retry: impl Fn(&mut Window, &mut App) + 'static) -> AnyElement {
    vstack(12.0)
        .size_full()
        .items_center()
        .pt(px(STATE_TOP_PADDING))
        .child(symbol("exclamationmark.triangle", 30.0, theme.text.secondary))
        .child(
            div().id("share-sheet-retry").cursor_pointer().on_click(move |_, window, cx| retry(window, cx)).child(
                MainText::new(tr("common_retry"))
                    .px(14.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(theme.primary._100),
            ),
        )
        .into_any_element()
}

fn symbol(name: &str, side: f32, color: Hsla) -> AnyElement {
    match IconUi::sized(name, px(side), color, name.ends_with(".fill")) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(side)).into_any_element(),
    }
}

#[cfg(test)]
#[path = "share_sheet_tests.rs"]
mod tests;
