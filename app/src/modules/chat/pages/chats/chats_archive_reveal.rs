use rise_core::foundation::foundation_date::TimeInterval;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatsArchiveRevealPhase {
    Hidden,
    Pulling,
    Revealed,
}

/// where the list is allowed to rest once the pull is over
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatsArchiveSettleTarget {
    Hidden,
    Revealed,
}

pub enum ChatsArchiveRevealPolicy {}

impl ChatsArchiveRevealPolicy {
    pub const ROW_HEIGHT: f32 = 72.0;
    pub const REVEAL_THRESHOLD: f32 = 46.0;
    pub const TOP_TOLERANCE: f32 = 8.0;
    pub const GESTURE_MINIMUM_DISTANCE: f32 = 6.0;

    pub fn can_begin_pull(phase: ChatsArchiveRevealPhase, marker_y: f32, interactions_enabled: bool) -> bool {
        phase == ChatsArchiveRevealPhase::Hidden && interactions_enabled && marker_y.abs() <= Self::TOP_TOLERANCE
    }

    pub fn should_reveal(marker_y: f32) -> bool {
        marker_y >= Self::REVEAL_THRESHOLD
    }

    pub fn should_hide(phase: ChatsArchiveRevealPhase, marker_y: f32) -> bool {
        phase == ChatsArchiveRevealPhase::Revealed && marker_y <= 0.0
    }

    /// how far the list top sits below the resting position
    /// derived from the list and not from a row it may unmount
    /// the archive row is first so the content top is one row height above it
    pub fn marker_y(content_top_y: f32) -> f32 {
        Self::clamped_marker_y(content_top_y + Self::ROW_HEIGHT)
    }

    pub fn clamped_marker_y(marker_y: f32) -> f32 {
        marker_y.max(-Self::ROW_HEIGHT)
    }

    pub const DECELERATION_TRAVEL_FACTOR: f32 = 0.5;

    pub fn projected_marker_y(marker_y: f32, velocity_y: f32) -> f32 {
        marker_y + velocity_y * Self::DECELERATION_TRAVEL_FACTOR
    }

    /// the pull has two endpoints, row fully hidden or fully shown
    /// the list needs help only when it would stop over the row, a flick past it keeps its momentum
    pub fn needs_settle(marker_y: f32, velocity_y: f32) -> bool {
        if marker_y < -Self::ROW_HEIGHT {
            return false;
        }
        let projected = Self::projected_marker_y(marker_y, velocity_y);
        projected > 0.0 && projected < Self::ROW_HEIGHT
    }

    pub fn settle_target(marker_y: f32, velocity_y: f32) -> ChatsArchiveSettleTarget {
        if Self::should_reveal(Self::projected_marker_y(marker_y, velocity_y)) {
            ChatsArchiveSettleTarget::Revealed
        } else {
            ChatsArchiveSettleTarget::Hidden
        }
    }

    pub fn settle_marker_y(target: ChatsArchiveSettleTarget) -> f32 {
        if target == ChatsArchiveSettleTarget::Revealed { Self::ROW_HEIGHT } else { 0.0 }
    }

    /// one point of content offset moves the marker one point so a delta is enough
    /// the settle never needs insets, content size or mount position
    pub fn settle_content_offset_delta(marker_y: f32, target: ChatsArchiveSettleTarget) -> f32 {
        marker_y - Self::settle_marker_y(target)
    }

    pub const REST_CHECK_DELAY: TimeInterval = 0.12;

    /// strictly between the two resting positions of the row
    pub fn rests_over_row(marker_y: f32) -> bool {
        marker_y > Self::MARKER_EPSILON && marker_y < Self::ROW_HEIGHT - Self::MARKER_EPSILON
    }

    pub fn rest_settle_target(phase: ChatsArchiveRevealPhase, marker_y: f32) -> ChatsArchiveSettleTarget {
        if phase != ChatsArchiveRevealPhase::Hidden && Self::should_reveal(marker_y) {
            ChatsArchiveSettleTarget::Revealed
        } else {
            ChatsArchiveSettleTarget::Hidden
        }
    }

    pub const SETTLE_DURATION: TimeInterval = 0.4;

    pub fn visibility_progress(phase: ChatsArchiveRevealPhase, marker_y: f32) -> f32 {
        match phase {
            ChatsArchiveRevealPhase::Revealed => 1.0,
            ChatsArchiveRevealPhase::Pulling => (marker_y / Self::ROW_HEIGHT).clamp(0.0, 1.0),
            ChatsArchiveRevealPhase::Hidden => 0.0,
        }
    }

    pub const MARKER_EPSILON: f32 = 0.5;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatsListRowRef {
    pub id: String,
    pub index: usize,
}

pub enum ChatsArchiveMarkerPreferenceKey {}

impl ChatsArchiveMarkerPreferenceKey {
    pub const DEFAULT_VALUE: f32 = -f32::MAX;

    pub fn reduce(value: &mut f32, next_value: impl FnOnce() -> f32) {
        *value = value.max(next_value());
    }
}
