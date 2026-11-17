use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::RangeInclusive;
use std::rc::Rc;

use gpui::{AnyView, App, Hsla, Pixels, Size, Window, px, size};
use rise_core::foundation::foundation_compare::{CompareOptions, ns_range_of};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::{NSRange, length_utf16};
use rise_theme::ThemePalette;
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;

use crate::modules::chat::components::chat::bubble::media_letterbox_photo::MediaAspectFit;
use crate::modules::comment::stores::comment_actions::comment_types::{
    CommentMedia, CommentMediaType, GetCommentsResponse,
};

/// one comment list row as a value
/// the engine never sees a view until the row is mounted, identity and height and what
/// changed are here
#[derive(Clone, PartialEq)]
pub struct CommentListRow {
    pub id: String,
    pub kind: CommentListRowKind,
    pub content_version: i64,
    pub layout_version: i64,
    pub is_text_expanded: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommentListRowPlacement {
    RootList,
    ThreadRoot,
    ThreadReply,
}

impl CommentListRowPlacement {
    pub fn is_inside_thread(&self) -> bool {
        *self != Self::RootList
    }

    pub fn leading_indent(&self) -> f32 {
        if *self == Self::ThreadReply {
            CommentRowMetrics::REPLY_INDENT
        } else {
            0.0
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum CommentListRowKind {
    Comment(GetCommentsResponse, CommentListRowPlacement),
    Accessory(String),
}

impl CommentListRow {
    pub fn comment(
        comment: GetCommentsResponse,
        placement: CommentListRowPlacement,
        is_text_expanded: bool,
        extra_content: Option<impl Hash>,
    ) -> CommentListRow {
        let mut content = DefaultHasher::new();
        comment.hash(&mut content);
        placement.is_inside_thread().hash(&mut content);
        placement.leading_indent().to_bits().hash(&mut content);
        extra_content.hash(&mut content);
        is_text_expanded.hash(&mut content);

        let mut layout = DefaultHasher::new();
        comment.content.hash(&mut layout);
        comment.addressed_to_tag.hash(&mut layout);
        comment.entities.hash(&mut layout);
        comment.media.hash(&mut layout);
        comment.moderation_status.hash(&mut layout);
        comment.is_first.hash(&mut layout);
        comment.author.more.is_official.hash(&mut layout);
        comment.author.more.is_premium.hash(&mut layout);
        placement.leading_indent().to_bits().hash(&mut layout);
        is_text_expanded.hash(&mut layout);

        CommentListRow {
            id: comment.id.clone(),
            kind: CommentListRowKind::Comment(comment, placement),
            content_version: content.finish() as i64,
            layout_version: layout.finish() as i64,
            is_text_expanded,
        }
    }

    pub fn accessory(name: &str, version: impl Hash) -> CommentListRow {
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        version.hash(&mut hasher);
        let value = hasher.finish() as i64;
        CommentListRow {
            id: format!("accessory.{name}"),
            kind: CommentListRowKind::Accessory(name.to_string()),
            content_version: value,
            layout_version: value,
            is_text_expanded: false,
        }
    }

    pub fn is_accessory(&self) -> bool {
        matches!(self.kind, CommentListRowKind::Accessory(_))
    }
}

/// everything the controller needs for one update, handed over by the owning view
#[derive(Clone)]
pub struct CommentListInput {
    pub identity: String,
    pub rows: Vec<CommentListRow>,
    pub has_more: bool,
    pub is_loading_more: bool,
    pub top_inset: f32,
    pub bottom_inset: f32,
    pub is_interaction_locked: bool,
    pub background_color: Hsla,
    pub palette: ThemePalette,
    pub focus_id: Option<String>,
    pub reveal_id: Option<String>,
    pub load_more: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_focus_resolved: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    pub on_focus_missing: Rc<dyn Fn(&mut Window, &mut App)>,
    pub on_revealed: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    pub content: Rc<dyn Fn(&CommentListRow, &mut Window, &mut App) -> AnyView>,
}

/// row geometry in one place for the row view and for the estimator
/// a number in only one of the two makes the row change height after it shows
pub enum CommentRowMetrics {}

impl CommentRowMetrics {
    pub const HORIZONTAL_PADDING: f32 = HoldContextMenuMetrics::EDGE_INSET + 12.0;
    pub const VERTICAL_PADDING: f32 = 10.0;
    pub const AVATAR_SIZE: f32 = 36.0;
    pub const AVATAR_SPACING: f32 = 9.0;
    pub const BODY_SPACING: f32 = 5.0;
    pub const REPLY_INDENT: f32 = 22.0;
    pub const AUTHOR_FONT_SIZE: f32 = 13.0;
    pub const TEXT_FONT_SIZE: f32 = 14.0;
    pub const TEXT_LINE_SPACING: f32 = 3.0;
    pub const PLACEHOLDER_FONT_SIZE: f32 = 13.0;
    pub const TEXT_LINE_LIMIT: usize = 5;
    pub const TEXT_TOGGLE_GAP: f32 = 2.0;
    pub const TEXT_TOGGLE_FONT_SIZE: f32 = 14.0;
    pub const BADGE_SIZE: f32 = 15.0;
    pub const ACTIONS_MIN_HEIGHT: f32 = 32.0;
    pub const ACTIONS_BOTTOM_PADDING: f32 = 8.0;
    pub const ACTIONS_LEADING_PADDING: f32 = Self::HORIZONTAL_PADDING + Self::AVATAR_SIZE + Self::AVATAR_SPACING;
    pub const MEDIA_TOP_PADDING_WITH_TEXT: f32 = 3.0;
    pub const MEDIA_TOP_PADDING_ALONE: f32 = 1.0;
    pub const STICKER_SIDE: f32 = 144.0;
    pub const PHOTO_WIDTH: f32 = 232.0;
    pub const GIF_ASPECT_RANGE: RangeInclusive<f32> = 0.5..=2.5;

    pub fn gif_bounding_box() -> Size<Pixels> {
        size(px(232.0), px(268.0))
    }

    /// size comes from the wire sizes never from decoded pixels so row height doesnt move once
    /// the list stopped
    pub fn photo_size(media: &CommentMedia) -> Size<Pixels> {
        let source_width = media.width.max(1) as f32;
        let source_height = media.height.max(1) as f32;
        let ratio = 1.9_f32.min(0.62_f32.max(source_width / source_height));
        size(
            px(Self::PHOTO_WIDTH),
            px(320.0_f32.min(120.0_f32.max(Self::PHOTO_WIDTH / ratio))),
        )
    }

    /// box follows the gif own proportions so aspect fill has nothing to crop
    pub fn gif_size(media: &CommentMedia) -> Size<Pixels> {
        let source_width = media.width.max(1) as f32;
        let source_height = media.height.max(1) as f32;
        let raw = source_width / source_height;
        let ratio = if raw.is_finite() && raw > 0.0 {
            Self::GIF_ASPECT_RANGE.end().min(Self::GIF_ASPECT_RANGE.start().max(raw))
        } else {
            1.0
        };
        let fitted = MediaAspectFit::aspect_fit_size(size(px(ratio), px(1.0)), Self::gif_bounding_box());
        if fitted.width < px(1.0) || fitted.height < px(1.0) {
            return Self::gif_bounding_box();
        }
        fitted
    }

    pub fn media_height(media: &CommentMedia) -> f32 {
        match media.r#type {
            CommentMediaType::Photo => f32::from(Self::photo_size(media).height),
            CommentMediaType::Gif => f32::from(Self::gif_size(media).height),
            CommentMediaType::Sticker => Self::STICKER_SIDE,
        }
    }

    /// the text the row draws, with the addressee prefixed
    /// entities address the original string by offset so they shift with the prefix
    pub fn resolved_body(comment: &GetCommentsResponse) -> (String, i64) {
        let tag = comment
            .addressed_to_tag
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or("");
        let raw = comment.content.as_str();
        let body = trimming_whitespaces_and_newlines(raw);
        let leading_trim = ns_range_of(raw, body, CompareOptions::NONE, NSRange::full(raw))
            .map(|range| -(range.location as i64))
            .unwrap_or(0);
        if tag.is_empty() {
            return (body.to_string(), leading_trim);
        }
        let mention = format!("@{tag}");
        if body.starts_with(&mention) {
            return (body.to_string(), leading_trim);
        }
        if body.is_empty() {
            return (mention, leading_trim);
        }
        let prefix = format!("{mention} ");
        let shift = leading_trim + length_utf16(&prefix) as i64;
        (format!("{prefix}{body}"), shift)
    }
}
