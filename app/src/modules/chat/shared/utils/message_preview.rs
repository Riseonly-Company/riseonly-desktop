use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use gpui::{App, AsyncApp};
use parking_lot::Mutex;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::{
    character_prefix, is_whitespace_or_newline, trimming_whitespaces_and_newlines,
};
use rise_core::foundation::ns_cache::NSCache;
use rise_core::rise_app_events::{RiseAppEvents, UIApplication};
use rise_i18n::i18n::I18n;
use rise_i18n::{FormatArg, format_printf, tr};
use rise_ui::rich_text::document::rise_document_projection::RiseDocumentProjection;
use rise_ui::rich_text::rich_text_markup::RichTextMarkup;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::message_call_view::MessageCallPresentationFactory;
use crate::modules::chat::components::chat::bubble::message_content::{MessageContent, MessageContentKind};
use crate::modules::chat::components::chat::message_system_pill::{MessageSystemFormatContext, MessageSystemFormatter};
use crate::modules::chat::shared::utils::message_preview_hooks::MessagePreviewHooks;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageCallInfo, MessageContentType, MessageMediaKind, MessageSystemInfo, MessageSystemKind,
};

static LIST_CACHE: LazyLock<Mutex<MessageListPreviewCache>> =
    LazyLock::new(|| Mutex::new(MessageListPreviewCache::new()));
static LIST_PREVIEW_CHARACTERS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::client_value("message_list_preview_characters")).unwrap_or(0));

pub enum MessagePreview {}

impl MessagePreview {
    pub const CUSTOM_EMOJI_PREVIEW_GLYPH: &'static str = "🖼️";

    pub fn plain_custom_emoji(text: &str) -> String {
        text.replace(RiseDocumentProjection::CUSTOM_EMOJI_PLACEHOLDER, Self::CUSTOM_EMOJI_PREVIEW_GLYPH)
    }

    pub fn compact_list_text(text: &str) -> String {
        let visible = text.trim_start_matches(is_whitespace_or_newline);
        let prefix = character_prefix(visible, *LIST_PREVIEW_CHARACTERS);
        let result = Self::plain_custom_emoji(trimming_whitespaces_and_newlines(prefix));
        if prefix.len() < visible.len() { result + "…" } else { result }
    }

    pub fn list_snippet(message: &GetMessageMessage, cx: &mut App) -> String {
        let request = Self::list_request(message, cx);
        MessageListPreviewCache::synchronize_context(cx);
        if let Some(value) = LIST_CACHE.lock().value(&request) {
            return value;
        }
        let value = MessageListPreviewCache::preparation(&request, cx).render();
        LIST_CACHE.lock().insert(&value, &request);
        value
    }

    pub fn list_draft(text: &str, scope: &str, cx: &mut App) -> String {
        let compact = Self::compact_list_text(text);
        let marked = RichTextMarkup::contains_delimiter(&compact);
        let request = MessageListPreviewRequest {
            key: MessageListPreviewRequestKey { scope: scope.to_owned(), id: String::new(), is_draft: true },
            source: if marked {
                MessageListPreviewSource::Markup(compact.clone(), text.len())
            } else {
                MessageListPreviewSource::Text(compact.clone())
            },
            markup_text: None,
        };
        MessageListPreviewCache::synchronize_context(cx);
        if let Some(value) = LIST_CACHE.lock().value(&request) {
            return value;
        }
        let value = if marked { Self::compact_list_text(&RichTextMarkup::plain_text(text)) } else { compact };
        LIST_CACHE.lock().insert(&value, &request);
        value
    }

    pub fn invalidate_list_cache() {
        LIST_CACHE.lock().invalidate();
    }

    pub async fn prepare_list_snippets(messages: &[GetMessageMessage], cx: &mut AsyncApp) {
        let (generation, requests, revisions, inputs) = cx.update(|cx| {
            MessageListPreviewCache::synchronize_context(cx);
            let generation = LIST_CACHE.lock().generation;
            let requests: Vec<MessageListPreviewRequest> = messages
                .iter()
                .map(|message| Self::list_request(message, cx))
                .filter(|request| LIST_CACHE.lock().value(request).is_none())
                .collect();
            let revisions: Vec<Option<u64>> =
                requests.iter().map(|request| LIST_CACHE.lock().revision(request)).collect();
            let inputs: Vec<MessageListPreviewPreparation> =
                requests.iter().map(|request| MessageListPreviewCache::preparation(request, cx)).collect();
            (generation, requests, revisions, inputs)
        });
        let values = MessageListPreviewPreparer::prepare(inputs, cx).await;
        cx.update(|cx| {
            MessageListPreviewCache::synchronize_context(cx);
            let mut cache = LIST_CACHE.lock();
            if generation != cache.generation {
                return;
            }
            for (index, (request, value)) in requests.iter().zip(values.iter()).enumerate() {
                if cache.revision(request) != revisions[index] {
                    continue;
                }
                cache.insert(value, request);
            }
        });
    }

    fn list_request(message: &GetMessageMessage, cx: &mut App) -> MessageListPreviewRequest {
        let mut markup_text = None;
        let source = if message.content_type == MessageContentType::System {
            MessageListPreviewSource::System(MessageListPreviewSystemSeed::new(
                message,
                MessagePreviewHooks::contact_name,
                cx,
            ))
        } else if message.content_type == MessageContentType::Call {
            MessageListPreviewSource::Call(message.call_info.clone(), message.sender_id.clone(), message.created_at)
        } else {
            let text = Self::list_text(message, cx);
            let compact = Self::compact_list_text(&text);
            if Self::draws_markup(message) && RichTextMarkup::contains_delimiter(&compact) {
                let length = text.len();
                markup_text = Some(text);
                MessageListPreviewSource::Markup(compact, length)
            } else {
                MessageListPreviewSource::Text(compact)
            }
        };
        MessageListPreviewRequest {
            key: MessageListPreviewRequestKey {
                scope: message.chat_id.clone(),
                id: message.id.clone(),
                is_draft: false,
            },
            source,
            markup_text,
        }
    }

    /// row text with inline formatting kept for a formatted draw, a code block reads as plain code
    /// none when theres no markup so the plain row stays the cheap path
    pub fn list_markup(message: &GetMessageMessage, cx: &mut App) -> Option<String> {
        if !Self::draws_markup(message) {
            return None;
        }
        let text = Self::list_text(message, cx);
        if !RichTextMarkup::contains_delimiter(&text) || !RichTextMarkup::has_any_markup(&text) {
            return None;
        }
        Some(Self::compact_list_text(&RichTextMarkup::flattening_code_blocks(&text)))
    }

    fn list_text(message: &GetMessageMessage, cx: &mut App) -> String {
        if !message.text.is_empty()
            || !matches!(MessageContent::bubble_kind(message, None), MessageContentKind::MediaGrid)
            || MessagePreviewHooks::media_thumbnail_source(Some(message)).is_none()
        {
            return Self::raw_snippet(message, cx);
        }
        Self::snippet_beside_thumbnail(message, cx)
    }

    /// true when the bubble draws this text as markup so a plain preview drops the delimiters
    fn draws_markup(m: &GetMessageMessage) -> bool {
        m.content_type != MessageContentType::System
            && m.content_type != MessageContentType::Call
            && !m.text.is_empty()
            && m.document_raw.is_none()
    }

    /// label for a row that already draws the first frame of the media so it drops the media emoji
    pub fn snippet_beside_thumbnail(m: &GetMessageMessage, cx: &mut App) -> String {
        if !m.text.is_empty() || !matches!(MessageContent::bubble_kind(m, None), MessageContentKind::MediaGrid) {
            return Self::snippet(m, false, cx);
        }
        let grid = m.grid_media_items();
        if grid.len() > 1 {
            return format_printf(&tr("msg_preview_media_count_plain"), &[FormatArg::Int(grid.len() as i64)]);
        }
        match grid.first().map(|item| &item.kind) {
            Some(MessageMediaKind::Image) => tr("msg_preview_photo_plain"),
            Some(MessageMediaKind::Video) => tr("msg_preview_video_plain"),
            Some(MessageMediaKind::Animation | MessageMediaKind::Gif) => tr("chat_inline_type_gif"),
            _ => Self::snippet(m, false, cx),
        }
    }

    /// spoilers are veiled with dots, reveals_spoilers keeps the letters for text search
    pub fn snippet(m: &GetMessageMessage, reveals_spoilers: bool, cx: &mut App) -> String {
        if !Self::draws_markup(m) {
            return Self::raw_snippet(m, cx);
        }
        if reveals_spoilers { RichTextMarkup::plain_text(&m.text) } else { RichTextMarkup::preview_text(&m.text) }
    }

    fn raw_snippet(m: &GetMessageMessage, cx: &mut App) -> String {
        if m.content_type == MessageContentType::System {
            return MessageSystemFormatter::render_current(m, cx);
        }
        if m.content_type == MessageContentType::Call {
            return MessageCallPresentationFactory::preview_title(m, cx);
        }
        if !m.text.is_empty() {
            return m.text.clone();
        }
        match MessageContent::bubble_kind(m, None) {
            MessageContentKind::Voice => tr("msg_preview_voice"),
            MessageContentKind::Audio => {
                let name = m.first_audio_item().and_then(|item| item.file_name.clone());
                format!("🎵 {}", name.unwrap_or_else(|| tr("msg_preview_audio_fallback")))
            }
            MessageContentKind::Document => {
                let name = m.first_document_item().and_then(|item| item.file_name.clone());
                format!("📄 {}", name.unwrap_or_else(|| tr("msg_preview_document_fallback")))
            }
            MessageContentKind::Sticker => {
                let emoji = m.first_sticker_item().and_then(|item| item.sticker_emojis.first().cloned());
                format!("{} {}", emoji.unwrap_or_else(|| "🙂".to_owned()), tr("msg_preview_sticker"))
            }
            MessageContentKind::Contact => {
                let name = m.contact.as_ref().map_or("", |contact| contact.name.as_str());
                format!("👤 {}: {name}", tr("msg_preview_contact_prefix"))
            }
            MessageContentKind::Location => {
                let title = m.location.as_ref().and_then(|location| location.title.clone());
                format!("📍 {}", title.unwrap_or_else(|| tr("msg_preview_location_fallback")))
            }
            MessageContentKind::Poll => {
                let question = m.poll.as_ref().map(|poll| poll.question.clone());
                format!("📊 {}", question.unwrap_or_else(|| tr("msg_preview_poll_fallback")))
            }
            MessageContentKind::VideoNote => tr("msg_preview_video_note"),
            MessageContentKind::Call => MessageCallPresentationFactory::preview_title(m, cx),
            MessageContentKind::PostShort => {
                let is_short = m.shared_post.as_ref().is_some_and(|post| post.is_riff);
                let icon = if is_short { "🎬" } else { "📎" };
                let title = m
                    .shared_post
                    .as_ref()
                    .map_or("", |post| trimming_whitespaces_and_newlines(&post.title));
                if !title.is_empty() {
                    return format!("{icon} {title}");
                }
                if is_short {
                    format_printf(&tr("msg_preview_reels"), &[FormatArg::Text(tr("riffs").into())])
                } else {
                    tr("msg_preview_post")
                }
            }
            MessageContentKind::MediaGrid => {
                let grid = m.grid_media_items();
                let n = grid.len();
                if n > 1 {
                    return format_printf(&tr("msg_preview_media_count"), &[FormatArg::Int(n as i64)]);
                }
                match grid.first().map(|item| &item.kind) {
                    Some(MessageMediaKind::Video) => tr("msg_preview_video"),
                    Some(MessageMediaKind::Animation | MessageMediaKind::Gif) => tr("chat_inline_type_gif"),
                    Some(_) => tr("msg_preview_photo"),
                    None => Self::content_type_fallback(m, cx),
                }
            }
            MessageContentKind::Text => Self::content_type_fallback(m, cx),
        }
    }

    fn content_type_fallback(message: &GetMessageMessage, cx: &mut App) -> String {
        match &message.content_type {
            MessageContentType::Text => String::new(),
            MessageContentType::Image => tr("msg_preview_photo"),
            MessageContentType::Video => tr("msg_preview_video"),
            MessageContentType::Animation => tr("chat_inline_type_gif"),
            MessageContentType::Voice => tr("msg_preview_voice"),
            MessageContentType::Audio => format!("🎵 {}", tr("msg_preview_audio_fallback")),
            MessageContentType::Document => format!("📄 {}", tr("msg_preview_document_fallback")),
            MessageContentType::Sticker => format!("🙂 {}", tr("msg_preview_sticker")),
            MessageContentType::VideoNote => tr("msg_preview_video_note"),
            MessageContentType::Contact => format!("👤 {}", tr("msg_preview_contact_prefix")),
            MessageContentType::Location => format!("📍 {}", tr("msg_preview_location_fallback")),
            MessageContentType::Poll => format!("📊 {}", tr("msg_preview_poll_fallback")),
            MessageContentType::Call => MessageCallPresentationFactory::preview_title(message, cx),
            MessageContentType::System => MessageSystemFormatter::render_current(message, cx),
            MessageContentType::PostShort => tr("msg_preview_post"),
            MessageContentType::Unknown(_) => tr("msg_reply_generic"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct MessageListPreviewSystemSeed {
    text: String,
    sender_name: String,
    kind: Option<String>,
    actor_id: Option<String>,
    actor_name: Option<String>,
    target_id: Option<String>,
    target_name: Option<String>,
    custom_text: Option<String>,
    gift_months: Option<i64>,
    gift_recipient_id: Option<String>,
}

impl MessageListPreviewSystemSeed {
    /// present names are swapped for the viewer contact names, an absent one stays absent
    /// so the formatter still picks the same sentence
    fn new(
        message: &GetMessageMessage,
        display_name: fn(Option<&str>, &str, &mut App) -> String,
        cx: &mut App,
    ) -> Self {
        let mut named = |id: Option<&str>, name: Option<&str>| -> Option<String> {
            let name = name?;
            if name.is_empty() {
                return Some(String::new());
            }
            Some(display_name(id, name, cx))
        };
        let system = message.system.as_ref();
        let sender_name = named(Some(message.sender_id.as_str()), Some(message.sender_name.as_str()))
            .unwrap_or_else(|| message.sender_name.clone());
        let actor_id = system.and_then(|system| system.actor_id.clone());
        let actor_name = named(actor_id.as_deref(), system.and_then(|system| system.actor_name.as_deref()));
        let target_id = system.and_then(|system| system.target_id.clone());
        let target_name = named(target_id.as_deref(), system.and_then(|system| system.target_name.as_deref()));
        Self {
            text: message.text.clone(),
            sender_name,
            kind: system.map(|system| system.kind.raw_value().to_owned()),
            actor_id,
            actor_name,
            target_id,
            target_name,
            custom_text: system.and_then(|system| system.text.clone()),
            gift_months: system.and_then(|system| system.gift_months),
            gift_recipient_id: system.and_then(|system| system.gift_recipient_id.clone()),
        }
    }

    fn message(&self) -> GetMessageMessage {
        let mut message = GetMessageMessage::new(String::new());
        message.content_type = MessageContentType::System;
        message.text = self.text.clone();
        message.sender_name = self.sender_name.clone();
        if let Some(kind) = self.kind.as_deref().and_then(MessageSystemKind::from_raw) {
            message.system = Some(MessageSystemInfo {
                kind,
                actor_id: self.actor_id.clone(),
                actor_name: self.actor_name.clone(),
                target_id: self.target_id.clone(),
                target_name: self.target_name.clone(),
                text: self.custom_text.clone(),
                wallpaper_preview: None,
                gift_months: self.gift_months,
                gift_recipient_id: self.gift_recipient_id.clone(),
                gift_item: None,
            });
        }
        message
    }

    fn retained_strings(&self) -> Vec<&str> {
        let optional = [
            &self.kind,
            &self.actor_id,
            &self.actor_name,
            &self.target_id,
            &self.target_name,
            &self.custom_text,
            &self.gift_recipient_id,
        ];
        let mut strings = vec![self.text.as_str(), self.sender_name.as_str()];
        strings.extend(optional.into_iter().filter_map(|value| value.as_deref()));
        strings
    }
}

#[derive(Clone, Debug, PartialEq)]
enum MessageListPreviewSource {
    Text(String),
    Markup(String, usize),
    System(MessageListPreviewSystemSeed),
    Call(Option<MessageCallInfo>, String, f64),
}

impl MessageListPreviewSource {
    fn retained_strings(&self) -> Vec<&str> {
        match self {
            Self::Text(text) | Self::Markup(text, _) => vec![text.as_str()],
            Self::System(seed) => seed.retained_strings(),
            Self::Call(call, sender_id, _) => {
                let mut strings = vec![sender_id.as_str()];
                if let Some(call) = call {
                    strings.push(call.call_id.as_str());
                    strings.push(call.caller_user_id.as_str());
                    strings.push(call.callee_user_id.as_str());
                    strings.extend(call.ended_by_user_id.as_deref());
                    strings.extend(call.end_reason.as_deref());
                }
                strings
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MessageListPreviewRequestKey {
    scope: String,
    id: String,
    is_draft: bool,
}

struct MessageListPreviewRequest {
    key: MessageListPreviewRequestKey,
    source: MessageListPreviewSource,
    markup_text: Option<String>,
}

enum MessageListPreviewPreparation {
    Text(String),
    Markup(String),
    System(MessageListPreviewSystemSeed, Arc<MessageSystemFormatContext>),
}

impl MessageListPreviewPreparation {
    fn render(self) -> String {
        let text = match self {
            Self::Text(value) => value,
            Self::Markup(value) => RichTextMarkup::preview_text(&value),
            Self::System(seed, context) => MessageSystemFormatter::render(&seed.message(), &context),
        };
        MessagePreview::compact_list_text(&text)
    }
}

struct MessageListPreviewCancellation(Arc<AtomicBool>);

impl Drop for MessageListPreviewCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

enum MessageListPreviewPreparer {}

static MESSAGE_LIST_PREVIEW_PREPARER: Mutex<()> = Mutex::new(());

impl MessageListPreviewPreparer {
    async fn prepare(inputs: Vec<MessageListPreviewPreparation>, cx: &AsyncApp) -> Vec<String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancellation = MessageListPreviewCancellation(cancelled.clone());
        cx.background_executor()
            .spawn(async move {
                let _isolation = MESSAGE_LIST_PREVIEW_PREPARER.lock();
                let mut values = Vec::with_capacity(inputs.len());
                for input in inputs {
                    if cancelled.load(Ordering::Relaxed) {
                        return Vec::new();
                    }
                    values.push(input.render());
                }
                values
            })
            .await
    }
}

struct MessageListPreviewCacheEntry {
    source: MessageListPreviewSource,
    text: String,
    revision: u64,
}

struct MessageListPreviewCache {
    entries: NSCache<MessageListPreviewRequestKey, Arc<MessageListPreviewCacheEntry>>,
    next_entry_revision: u64,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    current_user_id: Option<String>,
    profile_user_id: Option<String>,
    catalogue: Option<Arc<I18n>>,
    system_context: Option<Arc<MessageSystemFormatContext>>,
    generation: u64,
}

impl MessageListPreviewCache {
    fn new() -> Self {
        let entries = NSCache::new();
        entries.set_count_limit(usize::try_from(RiseLimits::client_value("message_list_preview_cache_entries")).unwrap_or(0));
        entries.set_total_cost_limit(usize::try_from(RiseLimits::client_value("message_list_preview_cache_bytes")).unwrap_or(0));
        RiseAppEvents::default_center()
            .observe(UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION, |_| {
                LIST_CACHE.lock().invalidate();
            })
            .detach();
        Self {
            entries,
            next_entry_revision: 0,
            account_user_id: None,
            account_epoch: None,
            current_user_id: None,
            profile_user_id: None,
            catalogue: None,
            system_context: None,
            generation: 0,
        }
    }

    fn synchronize_context(cx: &mut App) {
        let auth = AuthServicesStore::shared(cx);
        let auth = auth.read(cx);
        let current_user_id = auth.effective_user_id_for_wire();
        let profile_user_id = auth.profile().map(|profile| profile.id.clone());
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let catalogue = I18n::shared();
        let mut cache = LIST_CACHE.lock();
        if cache.account_user_id == account_user_id
            && cache.account_epoch == account_epoch
            && cache.current_user_id == current_user_id
            && cache.profile_user_id == profile_user_id
            && matches!(&cache.catalogue, Some(known) if Arc::ptr_eq(known, &catalogue))
        {
            return;
        }
        cache.invalidate();
        cache.account_user_id = account_user_id;
        cache.account_epoch = account_epoch;
        cache.current_user_id = current_user_id;
        cache.profile_user_id = profile_user_id;
        cache.catalogue = Some(catalogue);
    }

    fn revision(&self, request: &MessageListPreviewRequest) -> Option<u64> {
        self.entries.object(&request.key).map(|entry| entry.revision)
    }

    fn value(&self, request: &MessageListPreviewRequest) -> Option<String> {
        let entry = self.entries.object(&request.key)?;
        (entry.source == request.source).then(|| entry.text.clone())
    }

    fn preparation(request: &MessageListPreviewRequest, cx: &mut App) -> MessageListPreviewPreparation {
        match &request.source {
            MessageListPreviewSource::Text(text) => MessageListPreviewPreparation::Text(text.clone()),
            MessageListPreviewSource::Markup(compact, _) => {
                MessageListPreviewPreparation::Markup(request.markup_text.clone().unwrap_or_else(|| compact.clone()))
            }
            MessageListPreviewSource::System(seed) => {
                let cached = LIST_CACHE.lock().system_context.clone();
                let context = cached.unwrap_or_else(|| Arc::new(MessageSystemFormatContext::current(cx)));
                LIST_CACHE.lock().system_context = Some(context.clone());
                MessageListPreviewPreparation::System(seed.clone(), context)
            }
            MessageListPreviewSource::Call(call, sender_id, timestamp) => {
                let mut message = GetMessageMessage::new(request.key.id.clone());
                message.call_info = call.clone();
                message.sender_id = sender_id.clone();
                message.created_at = *timestamp;
                MessageListPreviewPreparation::Text(MessageCallPresentationFactory::preview_title(&message, cx))
            }
        }
    }

    fn insert(&mut self, text: &str, request: &MessageListPreviewRequest) {
        let cost = request
            .source
            .retained_strings()
            .into_iter()
            .chain([text, request.key.scope.as_str(), request.key.id.as_str()])
            .fold(size_of::<MessageListPreviewSource>(), |cost, string| cost + string.len() * size_of::<u16>());
        if cost > self.entries.total_cost_limit() {
            return;
        }
        self.next_entry_revision = self.next_entry_revision.wrapping_add(1);
        self.entries.set_object_cost(
            Arc::new(MessageListPreviewCacheEntry {
                source: request.source.clone(),
                text: text.to_owned(),
                revision: self.next_entry_revision,
            }),
            request.key.clone(),
            cost,
        );
    }

    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.remove_all_objects();
        self.system_context = None;
    }
}
