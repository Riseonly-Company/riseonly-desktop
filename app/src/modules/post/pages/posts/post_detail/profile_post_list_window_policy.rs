use crate::core::shared_element_transition::kernel::shared_element_types::ProfilePostListWindowPolicy;

impl ProfilePostListWindowPolicy {
    pub const SCROLLED_DOWN_THRESHOLD: f32 = 1.0;
    pub const PULL_DOWN_TOP_TOLERANCE: f32 = 0.5;
    pub const MAXIMUM_REVEAL_ROWS_PER_COMMIT: usize = 8;

    /// index of the anchor row or none to keep the legacy list
    /// once latched the window stays on even if the anchor left the page
    /// so the body type never flips back mid session
    pub fn anchor_index(is_eligible: bool, ids: &[String], anchor_id: &str, latched: bool) -> Option<usize> {
        if let Some(index) = ids.iter().position(|id| id == anchor_id)
            && (is_eligible || latched)
        {
            return Some(index);
        }
        if latched { Some(0) } else { None }
    }

    /// first rendered index, the revealed id if present or else the hint clamped into the page
    /// always a valid slice start
    pub fn lower_index(ids: &[String], revealed_from_id: &str, hint_index: i64) -> usize {
        if let Some(index) = ids.iter().position(|id| id == revealed_from_id) {
            return index;
        }
        if ids.is_empty() {
            return 0;
        }
        usize::try_from(hint_index.max(0)).unwrap_or(0).min(ids.len() - 1)
    }

    /// id that starts the window after revealing one more chunk above
    /// current_lower, none if the window already starts at the first row
    pub fn next_revealed_from_id(ids: &[String], current_lower: usize, chunk: usize) -> Option<String> {
        if ids.is_empty() {
            return None;
        }
        let lower = current_lower.min(ids.len() - 1);
        if lower == 0 {
            return None;
        }
        Some(ids[lower.saturating_sub(chunk.max(1))].clone())
    }

    /// nothing above is revealed until the reader scrolled down off
    /// the tapped post, so the list opens with it at the very top
    pub fn should_reveal(
        open_settled: bool,
        user_scrolled_down: bool,
        content_above_viewport: f32,
        viewport_height: f32,
        lower: usize,
        above_viewports: f32,
    ) -> bool {
        if !open_settled || !user_scrolled_down || lower == 0 || viewport_height <= 0.0 {
            return false;
        }
        content_above_viewport < above_viewports * viewport_height
    }

    /// pull to refresh only works once the window reaches the first row,
    /// so a pull at the window top never refreshes instead of revealing
    pub fn refresh_allowed(lower: usize) -> bool {
        lower == 0
    }

    /// true when the reader scrolled down off the tapped post, past the top by a drag or a fling
    pub fn latches_scrolled_down(distance_from_top: f32, is_tracking: bool, is_decelerating: bool) -> bool {
        distance_from_top > Self::SCROLLED_DOWN_THRESHOLD && (is_tracking || is_decelerating)
    }

    /// how many prepared rows to insert above the window in one commit
    /// stops at above_viewports viewports of content above, at most max_rows
    pub fn reveal_count(
        prepared_heights_above: &[f32],
        inter_item_spacing: f32,
        content_above: f32,
        viewport_height: f32,
        above_viewports: f32,
        max_rows: usize,
    ) -> usize {
        if viewport_height <= 0.0 || max_rows == 0 || prepared_heights_above.is_empty() {
            return 0;
        }
        let target = above_viewports * viewport_height;
        let spacing = inter_item_spacing.max(0.0);
        let mut covered = content_above.max(0.0);
        let mut count = 0;
        for height in prepared_heights_above.iter().take(max_rows) {
            count += 1;
            if *height > 0.0 {
                covered += *height + spacing;
            }
            if covered >= target {
                break;
            }
        }
        count
    }

    /// pull down dismiss at the window top only once nothing is
    /// left to reveal or before the reader scrolled down at all
    pub fn allows_pull_down_dismiss(lower: usize, user_scrolled_down: bool) -> bool {
        lower == 0 || !user_scrolled_down
    }
}
