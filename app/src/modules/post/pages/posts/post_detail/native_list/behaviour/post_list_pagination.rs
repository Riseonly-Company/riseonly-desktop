use std::rc::Rc;

use gpui::{
    App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, Window, div, px,
};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_i18n::tr;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::progress_view;

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListScrollMetrics;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageHistoryEdgePolicy;
use crate::modules::post::engine::rise_post_engine_models::RisePostPagePresentation;

const RETRY_SYMBOL: &str = "arrow.clockwise";
const RETRY_FONT_SIZE: f32 = 15.0;
const RETRY_HORIZONTAL_PADDING: f32 = 12.0;
const SPINNER_SIDE: f32 = 20.0;

pub enum PostListPaginationPolicy {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostListPaginationPolicyThresholds {
    pub load: f32,
    pub rearm: f32,
}

impl PostListPaginationPolicy {
    pub const ROW_THRESHOLD: i64 = 10;
    pub const MINIMUM_REQUEST_INTERVAL: TimeInterval = RiseMessageHistoryEdgePolicy::MINIMUM_REQUEST_INTERVAL;

    /// none while at least ROW_THRESHOLD rows of the page arent presented yet
    /// tail_row_heights are the presented rows heights, last row first
    pub fn thresholds(
        viewport_height: f32,
        velocity_y: f32,
        tail_row_heights: &[f32],
        unpresented_tail_count: i64,
        inter_item_spacing: f32,
    ) -> Option<PostListPaginationPolicyThresholds> {
        let unpresented = unpresented_tail_count.max(0);
        if unpresented >= Self::ROW_THRESHOLD {
            return None;
        }
        let edge = RiseMessageHistoryEdgePolicy::thresholds(viewport_height, velocity_y.max(0.0));
        let mut tail = 0.0_f32;
        let taken = usize::try_from(Self::ROW_THRESHOLD - unpresented).unwrap_or(0);
        for (index, height) in tail_row_heights.iter().take(taken).enumerate() {
            if index > 0 {
                tail += inter_item_spacing.max(0.0);
            }
            tail += height.max(0.0);
        }
        let load = edge.load.max(tail);
        let rearm = edge.rearm.max(load + 600.0_f32.max(viewport_height.max(0.0) * 0.75));
        Some(PostListPaginationPolicyThresholds { load, rearm })
    }

    pub fn should_request(
        distance_from_bottom: f32,
        thresholds: Option<PostListPaginationPolicyThresholds>,
        is_armed: bool,
        has_more: bool,
        is_loading_more: bool,
        is_refreshing: bool,
        elapsed_since_last_request: TimeInterval,
    ) -> bool {
        let Some(thresholds) = thresholds else { return false };
        if !is_armed
            || !has_more
            || is_loading_more
            || is_refreshing
            || elapsed_since_last_request < Self::MINIMUM_REQUEST_INTERVAL
        {
            return false;
        }
        distance_from_bottom <= thresholds.load
    }

    /// scrolling back up past the rearm distance allows a new trigger
    pub fn should_rearm(distance_from_bottom: f32, thresholds: Option<PostListPaginationPolicyThresholds>) -> bool {
        let Some(thresholds) = thresholds else { return true };
        distance_from_bottom > thresholds.rearm
    }

    /// distance below the viewport the list keeps filled with prepared rows
    pub fn suffix_lookahead(viewport_height: f32, velocity_y: f32) -> f32 {
        let edge = RiseMessageHistoryEdgePolicy::thresholds(viewport_height, velocity_y.max(0.0));
        edge.load + viewport_height.max(0.0)
    }

    pub fn needs_suffix_rows(distance_from_bottom: f32, viewport_height: f32, velocity_y: f32) -> bool {
        distance_from_bottom <= Self::suffix_lookahead(viewport_height, velocity_y)
    }

    /// backoff after loads that ended with no new rows
    /// a failure or the page cap with has_more still set
    /// 2 then 4 then 8 up to 30 s
    pub fn retry_delay(after_empty_loads: i64) -> TimeInterval {
        let exponent = i32::try_from((after_empty_loads - 1).max(0)).unwrap_or(i32::MAX);
        TimeInterval::min(30.0, 2.0 * 2.0_f64.powi(exponent))
    }
}

/// edge trigger of the next page, evaluated on both engine metric callbacks
pub struct PostListPagination {
    is_armed: bool,
    last_request_time: Option<TimeInterval>,
    last_item_count: Option<usize>,
    last_tail_id: Option<String>,
    was_loading_more: bool,
    empty_load_streak: i64,
    retry_not_before: Option<TimeInterval>,
    is_stopped: bool,
}

impl Default for PostListPagination {
    fn default() -> Self {
        Self::new()
    }
}

impl PostListPagination {
    pub fn new() -> Self {
        Self {
            is_armed: true,
            last_request_time: None,
            last_item_count: None,
            last_tail_id: None,
            was_loading_more: false,
            empty_load_streak: 0,
            retry_not_before: None,
            is_stopped: false,
        }
    }

    fn now() -> TimeInterval {
        RiseClock::monotonic().as_secs_f64()
    }

    pub fn page_did_change(&mut self, page: &RisePostPagePresentation) {
        let count = page.items.len();
        let tail_id = page.items.last().map(|item| item.id.clone());
        let previous_count = self.last_item_count;
        let tail_changed = tail_id != self.last_tail_id;
        let load_ended = self.was_loading_more && !page.is_loading_more;
        self.last_item_count = Some(count);
        self.last_tail_id = tail_id;
        self.was_loading_more = page.is_loading_more;
        let Some(previous_count) = previous_count else { return };
        if count != previous_count || tail_changed {
            self.is_armed = true;
            self.empty_load_streak = 0;
            self.retry_not_before = None;
        } else if load_ended {
            self.empty_load_streak += 1;
            self.is_armed = true;
            self.retry_not_before =
                Some(Self::now() + PostListPaginationPolicy::retry_delay(self.empty_load_streak));
        }
    }

    /// true when the caller should request the next page now
    pub fn evaluate(
        &mut self,
        metrics: &RiseManualListScrollMetrics,
        page: &RisePostPagePresentation,
        tail_row_heights: &[f32],
        unpresented_tail_count: i64,
        inter_item_spacing: f32,
    ) -> bool {
        if self.is_stopped {
            return false;
        }
        let distance_from_bottom = metrics.distance_from_bottom.as_f32();
        let thresholds = PostListPaginationPolicy::thresholds(
            metrics.viewport_size.height.as_f32(),
            metrics.velocity_y.as_f32(),
            tail_row_heights,
            unpresented_tail_count,
            inter_item_spacing,
        );
        if !self.is_armed && PostListPaginationPolicy::should_rearm(distance_from_bottom, thresholds) {
            self.is_armed = true;
        }
        let now = Self::now();
        if self.retry_not_before.is_some_and(|retry_not_before| now < retry_not_before) {
            return false;
        }
        let elapsed = self.last_request_time.map_or(TimeInterval::MAX, |last_request_time| now - last_request_time);
        if !PostListPaginationPolicy::should_request(
            distance_from_bottom,
            thresholds,
            self.is_armed,
            page.has_more,
            page.is_loading_more,
            page.is_refreshing,
            elapsed,
        ) {
            return false;
        }
        self.is_armed = false;
        self.last_request_time = Some(now);
        true
    }

    /// seconds left before a backed off retry may request again, none when nothing waits
    pub fn retry_delay_remaining(&self) -> Option<TimeInterval> {
        if self.is_stopped {
            return None;
        }
        let remaining = self.retry_not_before? - Self::now();
        (remaining > 0.0).then_some(remaining)
    }

    pub fn connection_did_return(&mut self) {
        self.empty_load_streak = 0;
        self.retry_not_before = None;
        self.is_armed = true;
    }

    pub fn stop(&mut self) {
        self.is_stopped = true;
    }

    pub fn diagnostic_summary(&self) -> String {
        format!(
            "armed {} stopped {} empty_streak {} retry_in {:?} last_count {:?}",
            self.is_armed,
            self.is_stopped,
            self.empty_load_streak,
            self.retry_delay_remaining(),
            self.last_item_count,
        )
    }
}

/// spinner below the last row while the page has more
/// list reserves its height in the bottom inset
/// failed or backing off shows a retry button
pub struct PostListLoadMoreFooter {
    is_reserved: bool,
    is_waiting_retry: bool,
    is_failed: bool,
    is_spinning: bool,
    shows_retry: bool,
    content_end: f32,
    width: f32,
    tint: Option<gpui::Hsla>,
    pub on_retry: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl Default for PostListLoadMoreFooter {
    fn default() -> Self {
        Self::new()
    }
}

impl PostListLoadMoreFooter {
    pub const HEIGHT: f32 = 44.0;

    pub fn new() -> Self {
        Self {
            is_reserved: false,
            is_waiting_retry: false,
            is_failed: false,
            is_spinning: false,
            shows_retry: false,
            content_end: 0.0,
            width: 0.0,
            tint: None,
            on_retry: None,
        }
    }

    pub fn is_reserved(&self) -> bool {
        self.is_reserved
    }

    pub fn is_waiting_retry(&self) -> bool {
        self.is_waiting_retry
    }

    pub fn reserved_inset(&self) -> f32 {
        if self.is_reserved { Self::HEIGHT } else { 0.0 }
    }

    pub fn page_did_change(&mut self, page: &RisePostPagePresentation, cx: &mut Context<Self>) -> bool {
        self.is_failed = page.error_message.is_some() && !page.is_loading_more;
        let reserve = page.has_more && !page.items.is_empty();
        if reserve == self.is_reserved {
            return false;
        }
        self.is_reserved = reserve;
        if !reserve {
            self.is_spinning = false;
            self.shows_retry = false;
        }
        cx.notify();
        true
    }

    /// bottom_inset is the lists whole bottom inset, this footer included
    pub fn layout(
        &mut self,
        metrics: &RiseManualListScrollMetrics,
        bottom_inset: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        let viewport = metrics.viewport_size.height.as_f32();
        if !self.is_reserved || viewport <= 0.0 || metrics.maximum_content_offset_y <= metrics.minimum_content_offset_y {
            self.set_presented(false, false, self.content_end, width, cx);
            return;
        }
        let content_end = viewport - bottom_inset + metrics.distance_from_bottom.as_f32();
        let center_y = content_end + Self::HEIGHT / 2.0;
        let is_visible = center_y - Self::HEIGHT / 2.0 < viewport && center_y + Self::HEIGHT / 2.0 > 0.0;
        let shows_retry = is_visible && (self.is_failed || self.is_waiting_retry);
        let is_on_screen = is_visible && !self.is_failed && !self.is_waiting_retry;
        self.set_presented(is_on_screen, shows_retry, content_end, width, cx);
    }

    fn set_presented(
        &mut self,
        is_spinning: bool,
        shows_retry: bool,
        content_end: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        let is_shown = is_spinning || shows_retry;
        let moved = is_shown && (self.content_end != content_end || self.width != width);
        if self.is_spinning == is_spinning && self.shows_retry == shows_retry && !moved {
            return;
        }
        self.is_spinning = is_spinning;
        self.shows_retry = shows_retry;
        self.content_end = content_end;
        self.width = width;
        cx.notify();
    }

    pub fn set_waiting_retry(&mut self, waiting: bool, cx: &mut Context<Self>) -> bool {
        if waiting == self.is_waiting_retry {
            return false;
        }
        self.is_waiting_retry = waiting;
        cx.notify();
        true
    }

    pub fn set_tint(&mut self, color: gpui::Hsla, cx: &mut Context<Self>) {
        self.tint = Some(color);
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.is_spinning = false;
        self.shows_retry = false;
        cx.notify();
    }
}

impl Render for PostListLoadMoreFooter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tint = self.tint.unwrap_or_else(|| rise_ui::theme(cx).text.secondary);
        let top = px(self.content_end);
        let height = px(Self::HEIGHT);
        let width = px(self.width);
        let mut root = div().absolute().top_0().left_0().size_full();
        if self.is_spinning {
            root = root.child(
                div()
                    .absolute()
                    .top(top)
                    .left_0()
                    .w(width)
                    .h(height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(SPINNER_SIDE)).child(progress_view().tint(tint))),
            );
        }
        if self.shows_retry {
            let on_retry = self.on_retry.clone();
            root = root.child(
                div().absolute().top(top).left_0().w(width).h(height).flex().items_center().justify_center().child(
                    div()
                        .id("post-list-load-more-retry")
                        .h(height)
                        .px(px(RETRY_HORIZONTAL_PADDING))
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            if let Some(on_retry) = on_retry.as_ref() {
                                on_retry(window, cx);
                            }
                        })
                        .children(IconUi::sized(RETRY_SYMBOL, px(RETRY_FONT_SIZE), tint, false))
                        .child(
                            MainText::new(tr("common_retry"))
                                .px(RETRY_FONT_SIZE)
                                .font_weight(FontWeight::MEDIUM)
                                .color(tint),
                        ),
                ),
            );
        }
        root
    }
}
