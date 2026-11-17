use gpui::App;

use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::kernel::shared_element_types::{
    ProfilePostListWindowPolicy, SharedElementSessionPolicyPhase,
};
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::RiseManualListScrollMetrics;
use crate::modules::post::pages::posts::post_detail::native_list::post_list_input::PostListRows;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostListRevealControllerDecision {
    Idle,
    Blocked { reason: String },
    Reveal { count: usize },
}

pub struct PostListRevealController {
    revealed_from_id: String,
    departed_from_id: Option<String>,
    restores_departed_top: bool,
    returning_departed_id: Option<String>,
    hint_index: usize,
    open_settled: bool,
    user_scrolled_down: bool,
    is_pinned_to_top: bool,
    is_feed: bool,
}

impl PostListRevealController {
    pub const WARM_ROWS_ABOVE: usize = 8;

    pub fn new(anchor_id: &str, anchor_index: usize) -> Self {
        Self {
            revealed_from_id: anchor_id.to_owned(),
            departed_from_id: None,
            restores_departed_top: false,
            returning_departed_id: None,
            hint_index: anchor_index,
            open_settled: false,
            user_scrolled_down: false,
            is_pinned_to_top: false,
            is_feed: false,
        }
    }

    pub fn new_feed_restore_id(restore_id: Option<String>) -> Self {
        let is_pinned_to_top = restore_id.is_none();
        Self {
            revealed_from_id: restore_id.unwrap_or_default(),
            departed_from_id: None,
            restores_departed_top: false,
            returning_departed_id: None,
            hint_index: 0,
            open_settled: true,
            user_scrolled_down: true,
            is_pinned_to_top,
            is_feed: true,
        }
    }

    pub fn revealed_from_id(&self) -> &str {
        &self.revealed_from_id
    }

    pub fn hint_index(&self) -> usize {
        self.hint_index
    }

    pub fn open_settled(&self) -> bool {
        self.open_settled
    }

    pub fn user_scrolled_down(&self) -> bool {
        self.user_scrolled_down
    }

    pub fn is_pinned_to_top(&self) -> bool {
        self.is_pinned_to_top
    }

    pub fn restores_departed_top(&self) -> bool {
        self.restores_departed_top
    }

    pub fn restored_departed_top_did_present(&mut self) {
        self.restores_departed_top = false;
    }

    pub fn returning_departed_id(&self) -> Option<&str> {
        self.returning_departed_id.as_deref()
    }

    pub fn returning_departed_did_present(&mut self) {
        self.returning_departed_id = None;
    }

    /// latches user_scrolled_down
    /// true when it latched on this call
    pub fn observe(&mut self, metrics: &RiseManualListScrollMetrics) -> bool {
        let pulled_above_top = metrics.is_tracking
            && f32::from(metrics.minimum_content_offset_y - metrics.content_offset_y)
                > ProfilePostListWindowPolicy::SCROLLED_DOWN_THRESHOLD;
        if self.user_scrolled_down
            || !(pulled_above_top
                || ProfilePostListWindowPolicy::latches_scrolled_down(
                    f32::from(metrics.distance_from_top),
                    metrics.is_tracking,
                    metrics.is_decelerating,
                ))
        {
            return false;
        }
        self.user_scrolled_down = true;
        true
    }

    pub fn reader_pushed_above_window_top(&mut self) -> bool {
        if self.user_scrolled_down {
            return false;
        }
        self.user_scrolled_down = true;
        true
    }

    pub fn mark_open_settled(&mut self) {
        self.open_settled = true;
    }

    /// nothing to wait for, a plain push or a flight that already ended
    pub fn settle_if_no_transition_session(&mut self, cx: &mut App) {
        if self.open_settled {
            return;
        }
        let coordinator = SharedElementTransitionCoordinator::shared(cx);
        if coordinator.read(cx).phase() == SharedElementSessionPolicyPhase::Idle {
            self.open_settled = true;
        }
    }

    pub fn lower(&self, rows: &PostListRows) -> usize {
        if self.is_pinned_to_top {
            return 0;
        }
        if let Some(index) = rows.first_index_by_id.get(&self.revealed_from_id) {
            return *index;
        }
        if self.is_feed {
            return 0;
        }
        ProfilePostListWindowPolicy::lower_index(&rows.ids, &self.revealed_from_id, self.hint_index as i64)
    }

    pub fn rows_did_change(&mut self, rows: &PostListRows) -> bool {
        if self.is_pinned_to_top || rows.ids.is_empty() {
            return false;
        }
        if let Some(departed) = self.departed_from_id.as_ref()
            && let Some(departed_index) = rows.first_index_by_id.get(departed).copied()
        {
            let current = rows.first_index_by_id.get(&self.revealed_from_id).copied();
            self.returning_departed_id = Some(departed.clone());
            if current.is_none_or(|current| departed_index <= current) {
                self.revealed_from_id = departed.clone();
                self.restores_departed_top = true;
            }
            self.departed_from_id = None;
        }
        if let Some(index) = rows.first_index_by_id.get(&self.revealed_from_id).copied() {
            self.hint_index = index;
            if self.is_feed && index == 0 {
                self.is_pinned_to_top = true;
            }
            return false;
        }
        if self.is_feed {
            self.is_pinned_to_top = true;
            return true;
        }
        let index = self.hint_index.min(rows.ids.len() - 1);
        if self.departed_from_id.is_none() {
            self.departed_from_id = Some(self.revealed_from_id.clone());
        }
        self.revealed_from_id = rows.ids[index].clone();
        true
    }

    /// heights_above are prepared heights
    /// nearest row first, up to the first unprepared row
    #[allow(clippy::too_many_arguments)]
    pub fn decide(
        &mut self,
        lower: usize,
        metrics: &RiseManualListScrollMetrics,
        is_dismissing: bool,
        is_in_navigation_transition: bool,
        is_commit_in_flight: bool,
        heights_above: &[f32],
        inter_item_spacing: f32,
        cx: &mut App,
    ) -> PostListRevealControllerDecision {
        if lower == 0 {
            return PostListRevealControllerDecision::Idle;
        }
        self.settle_if_no_transition_session(cx);
        let distance_from_top = f32::from(metrics.distance_from_top);
        let viewport_height = f32::from(metrics.viewport_size.height);
        if !ProfilePostListWindowPolicy::should_reveal(
            self.open_settled,
            self.user_scrolled_down,
            distance_from_top,
            viewport_height,
            lower,
            SharedElementMotion::REVEAL_ABOVE_VIEWPORTS,
        ) {
            return PostListRevealControllerDecision::Idle;
        }
        if is_dismissing {
            return PostListRevealControllerDecision::Blocked { reason: "dismiss".to_owned() };
        }
        if is_in_navigation_transition {
            return PostListRevealControllerDecision::Blocked { reason: "transition".to_owned() };
        }
        if is_commit_in_flight {
            return PostListRevealControllerDecision::Idle;
        }
        let count = ProfilePostListWindowPolicy::reveal_count(
            heights_above,
            inter_item_spacing,
            distance_from_top,
            viewport_height,
            SharedElementMotion::REVEAL_ABOVE_VIEWPORTS,
            ProfilePostListWindowPolicy::MAXIMUM_REVEAL_ROWS_PER_COMMIT.min(lower),
        );
        if count > 0 {
            PostListRevealControllerDecision::Reveal { count }
        } else {
            PostListRevealControllerDecision::Blocked { reason: "not_prepared".to_owned() }
        }
    }

    pub fn commit_reveal(&mut self, rows: &PostListRows, lower: usize, count: usize) {
        if rows.ids.is_empty() || count == 0 {
            return;
        }
        let index = lower.saturating_sub(count).min(rows.ids.len() - 1);
        self.revealed_from_id = rows.ids[index].clone();
        self.hint_index = index;
        if self.is_feed && index == 0 {
            self.is_pinned_to_top = true;
        }
    }

    /// extra rule on top of the old pull-down conditions
    pub fn allows_pull_down_dismiss(&self, lower: usize) -> bool {
        ProfilePostListWindowPolicy::allows_pull_down_dismiss(lower, self.user_scrolled_down)
    }
}
