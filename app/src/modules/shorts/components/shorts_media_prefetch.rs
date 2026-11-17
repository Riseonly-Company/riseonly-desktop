use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gpui::App;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_url::Url;
use rise_theme::avatar_palette::AvatarPalette;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::rise_media_prefetch_limits::RiseMediaPrefetchWindow;
use crate::core::media::video_warm_start_prefetch::VideoWarmStartPrefetch;
use crate::core::ui::clever_image_disk_cache::{CleverImageDiskPrefetch, CleverImagePrefetchItem};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

static LOOKAHEAD: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("shorts_media_prefetch_lookahead"));

pub enum ShortsMediaPrefetchPolicy {}

impl ShortsMediaPrefetchPolicy {
    pub const TRAILING: i64 = 1;

    pub const POSTER_PIXEL_DIMENSION: i64 = 3_072;
    pub const AVATAR_PIXEL_DIMENSION: i64 = 120;

    pub fn lookahead() -> i64 {
        *LOOKAHEAD
    }

    pub fn candidate_indices(item_count: i64, center: i64, previous_center: Option<i64>) -> Vec<i64> {
        RiseMediaPrefetchWindow::candidate_indices(
            item_count,
            center,
            previous_center,
            Self::lookahead(),
            Self::TRAILING,
        )
    }

    pub fn image_items(post: &GetPostFeedResponse, cx: &mut App) -> Vec<CleverImagePrefetchItem> {
        let mut items = Vec::new();
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        let media_policy = PostMediaCachePolicy::image_policy(is_own);

        if let Some(avatar) = AvatarPalette::resolved_logo_url(Some(post.author.more.logo.as_str()))
            && !post.author.is_deleted
        {
            items.push(CleverImagePrefetchItem {
                url_string: avatar,
                maximum_pixel_dimension: Some(Self::AVATAR_PIXEL_DIMENSION),
                persist_to_disk: true,
                cache_policy: CleverImageCachePolicy::Standard,
            });
        }

        match post.cover_slide() {
            Some(PostMediaSlide::Image { url, .. }) => {
                if let Some(normalized) = RemoteMediaURL::optional_normalized(Some(url.as_str())) {
                    items.push(CleverImagePrefetchItem {
                        url_string: normalized,
                        maximum_pixel_dimension: Some(Self::POSTER_PIXEL_DIMENSION),
                        persist_to_disk: is_own,
                        cache_policy: media_policy,
                    });
                }
            }
            Some(PostMediaSlide::Video { url, thumbnail, .. }) => {
                let poster = RemoteMediaURL::display_source(&url, thumbnail.as_deref());
                if !poster.is_empty() && poster.to_lowercase().starts_with("http") {
                    items.push(CleverImagePrefetchItem {
                        url_string: poster,
                        maximum_pixel_dimension: Some(Self::POSTER_PIXEL_DIMENSION),
                        persist_to_disk: is_own,
                        cache_policy: media_policy,
                    });
                }
            }
            None => {}
        }
        items
    }

    /// only a first slide video is worth warming, local files are not
    pub fn video_url(post: &GetPostFeedResponse) -> Option<String> {
        if post.video_slide_index() != Some(0) || post.shows_upload_overlay() {
            return None;
        }
        let resolved = RemoteMediaURL::optional_normalized(post.resolved_video_url().as_deref())?;
        let url = Url::from_string(&resolved)?;
        let scheme = url.scheme()?.to_lowercase();
        if scheme != "http" && scheme != "https" {
            return None;
        }
        Some(resolved)
    }
}

#[derive(Clone, Default)]
struct PageState {
    center: Option<i64>,
    items_fingerprint: Option<String>,
    generation: u64,
}

/// warms upcoming reels in two directional sets under one generation
/// videos pause in background, low power mode and on a hot device
pub struct ShortsMediaPrefetchController {
    scope_prefix: String,
    pages: HashMap<String, PageState>,
}

impl ShortsMediaPrefetchController {
    pub fn new(owner: &str) -> Self {
        Self {
            scope_prefix: format!("shorts-media:{owner}"),
            pages: HashMap::new(),
        }
    }

    pub fn update(&mut self, page_id: &str, active_post_id: &str, posts: &[GetPostFeedResponse], cx: &mut App) {
        let Some(center) = posts.iter().position(|post| post.id == active_post_id) else {
            return;
        };
        let center = center as i64;
        let mut state = self.pages.get(page_id).cloned().unwrap_or_default();
        let fingerprint = format!("{}|{}", posts.len(), posts.first().map_or("", |post| post.id.as_str()));
        if state.items_fingerprint.as_deref() != Some(fingerprint.as_str()) {
            state.items_fingerprint = Some(fingerprint);
            state.center = None;
        }
        if state.center == Some(center) {
            return;
        }
        let indices = ShortsMediaPrefetchPolicy::candidate_indices(posts.len() as i64, center, state.center);
        state.center = Some(center);
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        self.pages.insert(page_id.to_owned(), state);

        let mut seen_items = HashSet::new();
        let mut items = Vec::new();
        let mut seen_videos = HashSet::new();
        let mut videos = Vec::new();
        for index in indices {
            let Some(post) = usize::try_from(index).ok().and_then(|index| posts.get(index)) else {
                continue;
            };
            for item in ShortsMediaPrefetchPolicy::image_items(post, cx) {
                if seen_items.insert(item.clone()) {
                    items.push(item);
                }
            }
            if index == center {
                continue;
            }
            let Some(video) = ShortsMediaPrefetchPolicy::video_url(post) else {
                continue;
            };
            if seen_videos.insert(video.clone()) {
                videos.push(video);
            }
        }

        let scope = self.scope(page_id);
        CleverImageDiskPrefetch::replace_directional(
            items,
            &scope,
            generation,
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
            cx,
        );
        VideoWarmStartPrefetch::replace_directional(
            &videos,
            &scope,
            generation,
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
            cx,
        );
    }

    pub fn cancel_all(&mut self, cx: &mut App) {
        let pages = std::mem::take(&mut self.pages);
        for (page_id, state) in pages {
            let generation = state.generation.wrapping_add(1);
            let scope = self.scope(&page_id);
            CleverImageDiskPrefetch::cancel_directional(&scope, generation, cx);
            VideoWarmStartPrefetch::cancel_directional(&scope, generation, cx);
        }
    }

    fn scope(&self, page_id: &str) -> String {
        format!("{}:{page_id}", self.scope_prefix)
    }
}
