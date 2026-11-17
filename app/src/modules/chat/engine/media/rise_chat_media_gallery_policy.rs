use rise_canon::{RiseLimits, RiseTimingCanon, RiseTimingPage};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;

/// window policy of the chat media viewer
/// every number comes from canon, the page pairs with the server clamp
pub enum RiseChatMediaGalleryPolicy {}

impl RiseChatMediaGalleryPolicy {
    fn page() -> RiseTimingPage {
        RiseTimingCanon::page("chat_media_viewer_page")
    }

    /// canon entry has both halves of the pair so the server ceiling is the fallback
    pub fn page_limit() -> i64 {
        let page = Self::page();
        page.client_page_size_items.unwrap_or(page.server_maximum_items)
    }

    pub fn window_limit() -> i64 {
        RiseLimits::client_value("chat_media_viewer_window_limit")
    }

    pub fn window_bytes() -> i64 {
        RiseLimits::client_value("chat_media_viewer_window_bytes")
    }

    pub fn index_limit() -> i64 {
        RiseLimits::client_value("chat_media_viewer_index_limit")
    }

    /// items before an edge at which the next page is requested
    pub fn prefetch_lookahead() -> i64 {
        RiseLimits::client_value("chat_media_viewer_prefetch_lookahead")
    }

    /// empty pages in a row followed before an edge counts as exhausted
    /// a sparse feed can say nothing here but there is more
    pub fn empty_page_chain() -> i64 {
        RiseLimits::client_value("chat_media_viewer_empty_page_chain")
    }

    pub fn index_freshness() -> f64 {
        RiseTimingCanon::freshness_seconds("chat_media_index")
    }
}
