use std::collections::{HashMap, VecDeque};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, FontWeight, Hsla, SharedString, Task, TextRun, Window, px};
use rise_canon::RiseLimits;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use rise_theme::ThemeStore;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::comment::components::native_list::comment_list_models::{
    CommentListRow, CommentListRowPlacement, CommentRowMetrics,
};
use crate::modules::comment::stores::comment_actions::comment_types::GetCommentsResponse;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CommentRowHeightKey {
    pub row_id: String,
    pub layout_version: i64,
    pub width_pixels: i64,
    pub text_delta: i64,
}

impl CommentRowHeightKey {
    pub fn new(row: &CommentListRow, width: f32, display_scale: f32) -> Self {
        Self {
            row_id: row.id.clone(),
            layout_version: row.layout_version,
            width_pixels: (width * display_scale.max(1.0)).round() as i64,
            text_delta: (ThemeStore::text_delta_snapshot() * 10.0).round() as i64,
        }
    }
}

/// heights measured from a realised row, bounded and main thread only
/// dropped on memory warning and account change since a height is cheap to measure again
pub struct CommentRowHeightCache {
    heights: HashMap<CommentRowHeightKey, f32>,
    order: VecDeque<CommentRowHeightKey>,
    account_token: String,
    _memory_warning_observer: RiseSubscription,
    _memory_warning_task: Task<()>,
}

impl CommentRowHeightCache {
    pub fn shared(cx: &mut App) -> Entity<CommentRowHeightCache> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn capacity() -> usize {
        RiseLimits::client_value("comment_row_heights_memory").max(0) as usize
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let observer = RiseAppEvents::default_center()
            .observe(UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION, move |_| {
                let _ = sender.unbounded_send(());
            });
        let task = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                if this.update(cx, |cache, _| cache.remove_all()).is_err() {
                    break;
                }
            }
        });
        Self {
            heights: HashMap::new(),
            order: VecDeque::new(),
            account_token: String::new(),
            _memory_warning_observer: observer,
            _memory_warning_task: task,
        }
    }

    pub fn height(&mut self, key: &CommentRowHeightKey) -> Option<f32> {
        self.validate_account();
        self.heights.get(key).copied()
    }

    pub fn insert(&mut self, height: f32, key: CommentRowHeightKey) {
        self.validate_account();
        if !height.is_finite() || height <= 0.0 {
            return;
        }
        if self.heights.insert(key.clone(), height).is_none() {
            self.order.push_back(key);
        }
        let capacity = Self::capacity();
        while self.order.len() > capacity {
            let Some(oldest) = self.order.pop_front() else { break };
            self.heights.remove(&oldest);
        }
    }

    pub fn remove_all(&mut self) {
        self.heights.clear();
        self.order.clear();
    }

    fn validate_account(&mut self) {
        let token = format!(
            "{}|{}",
            SaiAccountCacheScope::active_user_id().unwrap_or_default(),
            SaiAccountCacheScope::active_epoch().unwrap_or_default()
        );
        if token == self.account_token {
            return;
        }
        self.account_token = token;
        self.remove_all();
    }
}

/// sizes a comment row before it exists from the same metrics the row is built with
/// exact for plain text, a formatted comment is corrected by the live audit once mounted
pub enum CommentRowHeightEstimator {}

impl CommentRowHeightEstimator {
    pub fn height(
        comment: &GetCommentsResponse,
        placement: CommentListRowPlacement,
        is_text_expanded: bool,
        width: f32,
        display_scale: f32,
        window: &Window,
    ) -> f32 {
        let delta = ThemeStore::text_delta_snapshot();
        let text_width = (width
            - placement.leading_indent()
            - CommentRowMetrics::HORIZONTAL_PADDING * 2.0
            - CommentRowMetrics::AVATAR_SIZE
            - CommentRowMetrics::AVATAR_SPACING)
            .max(1.0);

        let author_font = Font::system_weight((CommentRowMetrics::AUTHOR_FONT_SIZE + delta).max(1.0), FontWeight::SEMIBOLD);
        let mut author_height = Self::line_height(&author_font).ceil();
        if comment.author.more.is_official || comment.author.more.is_premium {
            author_height = author_height.max(CommentRowMetrics::BADGE_SIZE);
        }

        let mut body = author_height;
        if comment.is_moderation_disabled() {
            let font = Font::system_size((CommentRowMetrics::PLACEHOLDER_FONT_SIZE + delta).max(1.0));
            body += CommentRowMetrics::BODY_SPACING + Self::line_height(&font).ceil();
        } else {
            let (text, _) = CommentRowMetrics::resolved_body(comment);
            if !text.is_empty() {
                body += CommentRowMetrics::BODY_SPACING + Self::text_height(&text, text_width, delta, is_text_expanded, window);
            }
            if let Some(media) = &comment.media {
                let media_top = if text.is_empty() {
                    CommentRowMetrics::MEDIA_TOP_PADDING_ALONE
                } else {
                    CommentRowMetrics::MEDIA_TOP_PADDING_WITH_TEXT
                };
                body += CommentRowMetrics::BODY_SPACING + media_top + CommentRowMetrics::media_height(media);
            }
        }

        let total = CommentRowMetrics::VERTICAL_PADDING * 2.0
            + CommentRowMetrics::AVATAR_SIZE.max(body)
            + CommentRowMetrics::ACTIONS_MIN_HEIGHT
            + CommentRowMetrics::ACTIONS_BOTTOM_PADDING;
        let scale = display_scale.max(1.0);
        (total * scale).ceil() / scale
    }

    /// line height of the font, ascender plus descender
    fn line_height(font: &Font) -> f32 {
        font.size * NATURAL_LINE_HEIGHT
    }

    fn text_height(text: &str, width: f32, delta: f32, is_expanded: bool, window: &Window) -> f32 {
        let font = Font::system_size((CommentRowMetrics::TEXT_FONT_SIZE + delta).max(1.0));
        let line = Self::line_height(&font);
        let text = SharedString::from(text.to_string());
        let run = TextRun {
            len: text.len(),
            font: gpui::Font {
                family: font.family(),
                features: font.features(),
                fallbacks: None,
                weight: font.weight,
                style: font.style(),
            },
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let lines = window
            .text_system()
            .shape_text(text, px(font.size), &[run], Some(px(width)), None)
            .map(|wrapped| wrapped.iter().map(|line| line.wrap_boundaries().len() + 1).sum::<usize>())
            .unwrap_or(1)
            .max(1);
        let limit = CommentRowMetrics::TEXT_LINE_LIMIT;
        if lines <= limit {
            return (lines as f32 * line + (lines - 1) as f32 * CommentRowMetrics::TEXT_LINE_SPACING).ceil();
        }
        let shown = if is_expanded { lines } else { limit };
        let toggle_font =
            Font::system_weight((CommentRowMetrics::TEXT_TOGGLE_FONT_SIZE + delta).max(1.0), FontWeight::MEDIUM);
        (shown as f32 * line
            + (shown - 1) as f32 * CommentRowMetrics::TEXT_LINE_SPACING
            + CommentRowMetrics::TEXT_TOGGLE_GAP
            + Self::line_height(&toggle_font))
            .ceil()
    }
}
