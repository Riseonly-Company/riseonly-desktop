use std::sync::LazyLock;

use rise_canon::RiseLimits;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

static QUEUE_CONCURRENCY: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::client_value("image_prefetch_queue_concurrency"));

static QUEUE_PENDING: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("image_prefetch_queue_pending"));

/// shared image prefetch queue limits
/// resolved once since the client value lookup is a linear scan and this is a hot path
pub enum RiseImagePrefetchLimits {}

impl RiseImagePrefetchLimits {
    pub fn queue_concurrency() -> i64 {
        *QUEUE_CONCURRENCY
    }

    pub fn queue_pending() -> i64 {
        *QUEUE_PENDING
    }
}

/// prefetch candidates around the visible row
/// warms along the travel direction and keeps a short tail behind, posts feed and reels share it
pub enum RiseMediaPrefetchWindow {}

impl RiseMediaPrefetchWindow {
    pub fn candidate_indices(
        item_count: i64,
        center: i64,
        previous_center: Option<i64>,
        lookahead: i64,
        trailing: i64,
    ) -> Vec<i64> {
        if item_count <= 0 || center < 0 || center >= item_count || lookahead <= 0 {
            return Vec::new();
        }
        let trailing = trailing.max(0);
        let moves_forward = previous_center.is_none_or(|previous| center >= previous);
        let (lower, upper) = if moves_forward {
            ((center - trailing).max(0), item_count.min(center + lookahead + 1))
        } else {
            ((center - lookahead).max(0), item_count.min(center + 1 + trailing))
        };
        (lower..upper).collect()
    }
}
