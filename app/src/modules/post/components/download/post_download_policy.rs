use std::collections::HashMap;

use gpui::{App, Global};
use rise_catalog::{PostMediaStatus, PostModerationStatus};

use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::engine::rise_post_engine_models::RisePostContentKind;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// what a viewer may do with a posts media
/// the author always may and picks the watermark
/// a stranger follows the two switches the author set
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostDownloadPolicy {
    pub allowed: bool,
    pub chooses_watermark: bool,
}

impl GetPostFeedResponse {
    /// none when theres nothing to download
    /// a thread, an achievement share
    /// a post still uploading or processing, or one moderation switched off
    pub fn download_policy(&self, cx: &mut App) -> Option<PostDownloadPolicy> {
        if self.is_temp
            || self.content_kind == RisePostContentKind::Thread.wire()
            || self.achievement_id.unwrap_or(0) > 0
            || self.media_status != PostMediaStatus::Ready.wire()
            || self.moderation_status == PostModerationStatus::Disabled.wire()
            || self.media_slide_count() <= 0
        {
            return None;
        }
        if PostMediaCachePolicy::is_own_post(self, cx) {
            return Some(PostDownloadPolicy { allowed: true, chooses_watermark: true });
        }
        let allowed = self.count_visibility.allows_download;
        Some(PostDownloadPolicy {
            allowed,
            chooses_watermark: allowed && self.count_visibility.allows_download_without_watermark,
        })
    }

    pub fn allows_download_entry(&self, cx: &mut App) -> bool {
        self.download_policy(cx).is_some_and(|policy| policy.allowed)
    }
}

/// slide each visible gallery shows, so a menu opened on the card knows which media the reader was on
/// written by pagers, read when a download starts
#[derive(Default)]
pub struct PostMediaFocus {
    pages: HashMap<String, usize>,
}

impl Global for PostMediaFocus {}

impl PostMediaFocus {
    const CAPACITY: usize = 256;

    pub fn set(post_id: &str, page: usize, cx: &mut App) {
        let focus = cx.default_global::<Self>();
        if focus.pages.len() > Self::CAPACITY {
            focus.pages.clear();
        }
        focus.pages.insert(post_id.to_owned(), page);
    }

    pub fn page(post_id: &str, cx: &App) -> usize {
        cx.try_global::<Self>().and_then(|focus| focus.pages.get(post_id).copied()).unwrap_or(0)
    }
}
