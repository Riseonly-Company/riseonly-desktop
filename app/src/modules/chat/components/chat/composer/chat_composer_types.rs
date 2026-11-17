use std::sync::LazyLock;

use gpui::{AnyElement, App, IntoElement, RenderOnce, Window};
use image::RgbaImage;
use rise_core::foundation::ns_range::{self, NSRange};
use rise_core::foundation::foundation_compare::{CompareOptions, ns_range_of};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::input_ui::text_boundaries::{next_grapheme_boundary, previous_grapheme_boundary};
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use serde::{Deserialize, Serialize};

use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapDecoder};
use crate::core::widgets::wrappers::list_shell_ui::{list_shell_floating_top_inset, list_shell_safe_area_insets};
use crate::modules::chat::stores::message::message_actions::message_types::{MessageCustomEmojiEntity, RemoteMediaURL};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    StickerAssetType, StickerAssetTypeInfer, StickerItem,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ChatComposerReplyTarget {
    pub message_id: String,
    pub author_name: String,
    pub snippet: String,
    pub media_thumbnail_source: Option<String>,
}

impl ChatComposerReplyTarget {
    pub fn new(message_id: String, author_name: String, snippet: String) -> Self {
        Self { message_id, author_name, snippet, media_thumbnail_source: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatComposerEditTarget {
    pub message_id: String,
    pub original_text: String,
    pub preview_text: String,
    pub original_custom_emoji_runs: Vec<ChatComposerCustomEmojiRun>,
    pub media_thumbnail_source: Option<String>,
    pub media_full_source: Option<String>,
    pub can_reedit_media: bool,
    pub is_media_caption: bool,
}

impl ChatComposerEditTarget {
    pub fn new(message_id: String, original_text: String) -> Self {
        Self {
            message_id,
            original_text,
            preview_text: String::new(),
            original_custom_emoji_runs: Vec::new(),
            media_thumbnail_source: None,
            media_full_source: None,
            can_reedit_media: false,
            is_media_caption: false,
        }
    }
}

pub enum ChatBottomBarMetrics {}

impl ChatBottomBarMetrics {
    pub const CIRCLE: f32 = 42.0;
    pub const H_PADDING: f32 = 10.0;
    pub const TOP_PADDING: f32 = 5.0;
    pub const ROW_GAP: f32 = 6.0;
    pub const INPUT_MAX_HEIGHT: f32 = 120.0;
    pub const INPUT_CORNER: f32 = 25.0;
    pub const REPLY_EDIT_BAR_HEIGHT: f32 = 46.0;
    pub const LOCKED_PILL_HEIGHT: f32 = 38.0;
    pub const COMPOSER_ANIM_DURATION: f64 = 0.25;
}

/// takes the keyboard focus away from whoever holds it
/// the field gives focus back to its sheet or screen
pub fn resign_chat_keyboard(window: &mut Window, cx: &mut App) {
    ScreenFocus::release_text_field(window, cx);
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatDraftCustomEmoji {
    pub placeholder: String,
    pub item: StickerItem,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatComposerCustomEmojiRun {
    pub range: NSRange,
    pub item: StickerItem,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatComposerCaretEditResult {
    pub text: String,
    pub caret: NSRange,
}

/// emoji panel edits at the caret, the way the keyboard types
/// custom emoji drafts pair with placeholder chars in text order so edits cut by them
/// none or out of range selection edits at the end
pub enum ChatComposerCaretEdit {}

const OBJECT_REPLACEMENT: char = '\u{FFFC}';
const OBJECT_REPLACEMENT_TEXT: &str = "\u{FFFC}";

impl ChatComposerCaretEdit {
    pub fn custom_emoji_runs(text: &str, drafts: &[ChatDraftCustomEmoji]) -> Vec<ChatComposerCustomEmojiRun> {
        let length = ns_range::length_utf16(text);
        let mut runs = Vec::new();
        let mut cursor = 0;
        for draft in drafts {
            if cursor > length {
                break;
            }
            let Some(range) = ns_range_of(text, &draft.placeholder, CompareOptions::NONE, NSRange::new(cursor, length - cursor))
            else {
                continue;
            };
            runs.push(ChatComposerCustomEmojiRun { range, item: draft.item.clone() });
            cursor = range.location + range.length;
        }
        runs
    }

    pub fn replace(
        text: &str,
        selection: Option<NSRange>,
        with: &str,
        drafts: &mut Vec<ChatDraftCustomEmoji>,
        inserting: Option<ChatDraftCustomEmoji>,
    ) -> ChatComposerCaretEditResult {
        let range = Self::resolved(text, selection);
        let Some(bytes) = ns_range::utf16_to_byte_range(text, range) else {
            return ChatComposerCaretEditResult { text: text.to_owned(), caret: NSRange::new(range.location, 0) };
        };
        let before = drafts.len().min(Self::placeholder_count(&text[..bytes.start]));
        let removed = (drafts.len() - before).min(Self::placeholder_count(&text[bytes.clone()]));
        if removed > 0 {
            drafts.drain(before..before + removed);
        }
        if let Some(draft) = inserting {
            drafts.insert(before, draft);
        }
        let mut edited = String::with_capacity(text.len() - bytes.len() + with.len());
        edited.push_str(&text[..bytes.start]);
        edited.push_str(with);
        edited.push_str(&text[bytes.end..]);
        ChatComposerCaretEditResult {
            text: edited,
            caret: NSRange::new(range.location + ns_range::length_utf16(with), 0),
        }
    }

    /// deletes the selection, or the one composed character before the caret. none at the start
    pub fn delete_backward(
        text: &str,
        selection: Option<NSRange>,
        drafts: &mut Vec<ChatDraftCustomEmoji>,
    ) -> Option<ChatComposerCaretEditResult> {
        let mut range = Self::resolved(text, selection);
        if range.length == 0 {
            if range.location == 0 {
                return None;
            }
            let caret = ns_range::utf16_to_byte_offset(text, range.location)?;
            let start = previous_grapheme_boundary(text, caret);
            let end = next_grapheme_boundary(text, start);
            range = ns_range::byte_to_utf16_range(text, start..end);
        }
        Some(Self::replace(text, Some(range), "", drafts, None))
    }

    fn resolved(text: &str, selection: Option<NSRange>) -> NSRange {
        let length = ns_range::length_utf16(text);
        match selection {
            Some(selection) if selection.location + selection.length <= length => selection,
            _ => NSRange::new(length, 0),
        }
    }

    fn placeholder_count(text: &str) -> usize {
        text.chars().filter(|scalar| *scalar == OBJECT_REPLACEMENT).count()
    }
}

pub enum ChatComposerCustomEmojiPlaceholder {}

static TRANSPARENT_IMAGE: LazyLock<CleverImage> =
    LazyLock::new(|| CleverImageBitmapDecoder::render_image(RgbaImage::new(1, 1)));

impl ChatComposerCustomEmojiPlaceholder {
    /// one transparent pixel at scale 1, the attachment image of a custom emoji not drawn yet
    pub fn transparent_image() -> CleverImage {
        TRANSPARENT_IMAGE.clone()
    }
}

/// local payload of a composer custom emoji, keys are the original property names
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatComposerCustomEmojiAttachmentPayload {
    pub id: String,
    pub pack_id: String,
    pub file_url: String,
    pub thumbnail_url: Option<String>,
    pub width: i64,
    pub height: i64,
    pub duration_ms: Option<i64>,
    pub position: i64,
    pub created_at_ms: i64,
    pub associated_emojis: Vec<String>,
    pub asset_type: String,
    pub mime_type: Option<String>,
    pub codec: Option<String>,
}

impl ChatComposerCustomEmojiAttachmentPayload {
    pub fn new(item: &StickerItem) -> Self {
        Self {
            id: item.id.clone(),
            pack_id: item.pack_id.clone(),
            file_url: item.file_url.clone(),
            thumbnail_url: item.thumbnail_url.clone(),
            width: item.width,
            height: item.height,
            duration_ms: item.duration_ms,
            position: item.position,
            created_at_ms: item.created_at_ms,
            associated_emojis: item.associated_emojis.clone(),
            asset_type: item.asset_type.wire().to_owned(),
            mime_type: item.mime_type.clone(),
            codec: item.codec.clone(),
        }
    }

    pub fn item(&self) -> StickerItem {
        let normalized_file_url =
            RemoteMediaURL::optional_normalized(Some(self.file_url.as_str())).unwrap_or_else(|| self.file_url.clone());
        let asset_type = StickerAssetType::infer(Some(self.asset_type.as_str()), &normalized_file_url);
        StickerItem::new(
            self.id.clone(),
            self.pack_id.clone(),
            normalized_file_url,
            self.thumbnail_url
                .as_deref()
                .map(|thumbnail| RemoteMediaURL::optional_normalized(Some(thumbnail)).unwrap_or_else(|| thumbnail.to_owned())),
            self.width,
            self.height,
            self.duration_ms,
            self.position,
            self.created_at_ms,
            self.associated_emojis.clone(),
            asset_type,
            self.mime_type.clone(),
            self.codec.clone(),
        )
    }

    pub fn encoded_string(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }

    /// decodes the attribute value, an encoded string or nothing
    pub fn decode(value: Option<&str>) -> Option<Self> {
        serde_json::from_str(value?).ok()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatComposerCustomEmojiEditHydration {
    pub text: String,
    pub entities: Vec<MessageCustomEmojiEntity>,
}

pub enum ChatComposerCustomEmojiEditHydrator {}

impl ChatComposerCustomEmojiEditHydrator {
    pub fn hydrate(text: &str, entities: &[MessageCustomEmojiEntity]) -> ChatComposerCustomEmojiEditHydration {
        let source_length = ns_range::length_utf16(text) as i64;
        let mut sorted: Vec<(usize, &MessageCustomEmojiEntity)> = entities.iter().enumerate().collect();
        sorted.sort_by(|(left_index, left), (right_index, right)| {
            left.offset.cmp(&right.offset).then(left_index.cmp(right_index))
        });

        let mut accepted: Vec<MessageCustomEmojiEntity> = Vec::with_capacity(sorted.len());
        let mut previous_end: i64 = 0;
        for (_, candidate) in sorted {
            let fits = candidate.offset >= previous_end
                && candidate.offset >= 0
                && candidate.length > 0
                && candidate.offset <= source_length
                && candidate.length <= source_length - candidate.offset;
            if !fits {
                continue;
            }
            accepted.push(candidate.clone());
            previous_end = candidate.offset + candidate.length;
        }

        let mut hydrated_text = text.to_owned();
        for entity in accepted.iter().rev() {
            let range = NSRange::new(entity.offset as usize, entity.length as usize);
            if let Some(bytes) = ns_range::utf16_to_byte_range(&hydrated_text, range) {
                hydrated_text.replace_range(bytes, OBJECT_REPLACEMENT_TEXT);
            }
        }

        let mut cumulative_delta: i64 = 0;
        let normalized_entities = accepted
            .into_iter()
            .map(|entity| {
                let mut normalized = entity.clone();
                normalized.offset += cumulative_delta;
                normalized.length = 1;
                cumulative_delta += 1 - entity.length;
                normalized
            })
            .collect();
        ChatComposerCustomEmojiEditHydration { text: hydrated_text, entities: normalized_entities }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatComposerUserMention {
    pub user_id: String,
    pub tag: String,
    pub range: NSRange,
}

/// hands the content the shell bottom safe area inset and floating top inset
#[derive(IntoElement)]
pub struct ChatShellInsetsReader {
    content: Box<dyn FnOnce(f32, f32) -> AnyElement>,
}

impl ChatShellInsetsReader {
    pub fn new(content: impl FnOnce(f32, f32) -> AnyElement + 'static) -> Self {
        Self { content: Box::new(content) }
    }
}

impl RenderOnce for ChatShellInsetsReader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let shell_insets = list_shell_safe_area_insets(cx);
        let floating_top_inset = list_shell_floating_top_inset(cx);
        (self.content)(shell_insets.bottom, floating_top_inset)
    }
}

#[derive(IntoElement)]
struct ChatComposerInputChromeModifier {
    content: AnyElement,
    is_attached_to_inline_panel: bool,
}

impl RenderOnce for ChatComposerInputChromeModifier {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if self.is_attached_to_inline_panel {
            return self.content;
        }
        self.content.liquid_chrome_rounded_rectangle_with(ChatBottomBarMetrics::INPUT_CORNER, 0.12, None, true)
    }
}

pub trait ChatComposerInputChromeModifiers: IntoElement + Sized {
    fn chat_composer_input_chrome(self, is_attached_to_inline_panel: bool) -> AnyElement {
        ChatComposerInputChromeModifier { content: self.into_any_element(), is_attached_to_inline_panel }
            .into_any_element()
    }
}

impl<E: IntoElement> ChatComposerInputChromeModifiers for E {}
