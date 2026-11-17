use std::collections::{HashMap, HashSet};
use std::ops::Range;

use gpui::App;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::avatar_palette::AvatarPalette;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::rise_media_prefetch_limits::RiseMediaPrefetchWindow;
use crate::core::media::video_warm_start_prefetch::VideoWarmStartPrefetch;
use crate::core::ui::clever_image_disk_cache::{CleverImageDiskPrefetch, CleverImagePrefetchItem};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::shorts_media_prefetch::ShortsMediaPrefetchPolicy;

pub enum PostFeedMediaPrefetchPolicy {}

impl PostFeedMediaPrefetchPolicy {
    pub const TRAILING: i64 = 2;

    pub const AVATAR_PIXEL_DIMENSION: i64 = 120;

    pub const POSTER_PIXEL_DIMENSION: Option<i64> = None;

    pub const PHOTO_PIXEL_DIMENSION: Option<i64> = Some(2048);

    /// memory bucket of one gallery slide, the one its warm-up decoded
    /// posters full size, photos capped
    pub fn slide_pixel_dimension(is_video: bool) -> Option<i64> {
        if is_video {
            Self::POSTER_PIXEL_DIMENSION
        } else {
            Self::PHOTO_PIXEL_DIMENSION
        }
    }

    pub fn lookahead() -> i64 {
        RiseLimits::client_value("post_feed_media_prefetch_lookahead")
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

    pub fn items(post: &GetPostFeedResponse, _cx: &mut App) -> Vec<CleverImagePrefetchItem> {
        let mut items = Vec::new();
        let persists_media = PostMediaCachePolicy::FEED_IMAGES_PERSIST_TO_DISK;
        let media_policy = PostMediaCachePolicy::FEED_IMAGE_POLICY;

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

        if let Some(video) = post.resolved_video_url().filter(|video| !video.is_empty()) {
            let poster = RemoteMediaURL::display_source(&video, post.video_thumbnail.as_deref());
            if !poster.is_empty() && poster.to_lowercase().starts_with("http") {
                items.push(CleverImagePrefetchItem {
                    url_string: poster,
                    maximum_pixel_dimension: Self::POSTER_PIXEL_DIMENSION,
                    persist_to_disk: persists_media,
                    cache_policy: media_policy,
                });
            }
        }

        if let Some(first) = post.images.iter().find(|image| !trimming_whitespaces_and_newlines(image).is_empty())
            && let Some(normalized) = RemoteMediaURL::optional_normalized(Some(first.as_str()))
        {
            items.push(CleverImagePrefetchItem {
                url_string: normalized,
                maximum_pixel_dimension: Self::PHOTO_PIXEL_DIMENSION,
                persist_to_disk: persists_media,
                cache_policy: media_policy,
            });
        }

        items
    }

    /// inline feed video plays through the same shorts player as the reels tab so it warms
    /// through the same primitive and rule
    pub fn video_url(post: &GetPostFeedResponse) -> Option<String> {
        ShortsMediaPrefetchPolicy::video_url(post)
    }
}

#[derive(Clone, Default)]
struct PageState {
    center: Option<i64>,
    generation: u64,
    item_count: Option<i64>,
    first_item_id: Option<String>,
}

/// direction inputs of one window replacement
struct ReplaceWindow {
    visible_index: i64,
    previous_center: Option<i64>,
    video_indices: Option<Range<i64>>,
}

/// directional feed media warm-up
/// one window and generation per feed page
/// every centre change replaces the previous set so fast scrolling leaves no pile of pending downloads
pub struct PostFeedMediaPrefetchController {
    scope_prefix: String,
    pages: HashMap<String, PageState>,
}

impl PostFeedMediaPrefetchController {
    const CENTER_DEADBAND: i64 = 2;

    pub fn new(owner: &str) -> Self {
        Self { scope_prefix: format!("post-feed-media:{owner}"), pages: HashMap::new() }
    }

    pub fn update(&mut self, page_id: &str, visible_index: i64, posts: &[GetPostFeedResponse], cx: &mut App) {
        let state = self.pages.get(page_id).cloned().unwrap_or_default();
        if let Some(center) = state.center
            && (center - visible_index).abs() < Self::CENTER_DEADBAND
        {
            return;
        }
        let previous_center = state.center;
        self.replace(
            page_id,
            state,
            ReplaceWindow { visible_index, previous_center, video_indices: None },
            posts,
            cx,
        );
    }

    /// window for a list that grows at both ends, one scope per edge
    /// backward extends toward lower indices whatever the previous center was
    /// a new count or first id re-arms it past the deadband
    pub fn update_edge_window(
        &mut self,
        page_id: &str,
        visible_index: i64,
        posts: &[GetPostFeedResponse],
        backward: bool,
        video_indices: Range<i64>,
        cx: &mut App,
    ) {
        let mut state = self.pages.get(page_id).cloned().unwrap_or_default();
        let first_item_id = posts.first().map(|post| post.id.clone());
        let item_count = posts.len() as i64;
        if state.item_count != Some(item_count) || state.first_item_id != first_item_id {
            state.item_count = Some(item_count);
            state.first_item_id = first_item_id;
            state.center = None;
        }
        if let Some(center) = state.center
            && (center - visible_index).abs() < Self::CENTER_DEADBAND
        {
            return;
        }
        let previous_center = if backward { Some(visible_index + 1) } else { state.center };
        self.replace(
            page_id,
            state,
            ReplaceWindow { visible_index, previous_center, video_indices: Some(video_indices) },
            posts,
            cx,
        );
    }

    fn replace(
        &mut self,
        page_id: &str,
        mut state: PageState,
        replace_window: ReplaceWindow,
        posts: &[GetPostFeedResponse],
        cx: &mut App,
    ) {
        let ReplaceWindow { visible_index, previous_center, video_indices } = replace_window;
        let indices =
            PostFeedMediaPrefetchPolicy::candidate_indices(posts.len() as i64, visible_index, previous_center);
        state.center = Some(visible_index);
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        self.pages.insert(page_id.to_owned(), state);
        if indices.is_empty() {
            return;
        }

        let mut seen = HashSet::new();
        let mut items = Vec::with_capacity(indices.len() * 3);
        let mut seen_videos = HashSet::new();
        let mut videos = Vec::new();
        for index in indices {
            let Some(post) = usize::try_from(index).ok().and_then(|position| posts.get(position)) else {
                continue;
            };
            for item in PostFeedMediaPrefetchPolicy::items(post, cx) {
                if seen.insert(item.clone()) {
                    items.push(item);
                }
            }
            if index == visible_index || !video_indices.as_ref().is_none_or(|range| range.contains(&index)) {
                continue;
            }
            let Some(video) = PostFeedMediaPrefetchPolicy::video_url(post) else {
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
        for (page_id, state) in &self.pages {
            let generation = state.generation.wrapping_add(1);
            let scope = self.scope(page_id);
            CleverImageDiskPrefetch::cancel_directional(&scope, generation, cx);
            VideoWarmStartPrefetch::cancel_directional(&scope, generation, cx);
        }
        self.pages.clear();
    }

    fn scope(&self, page_id: &str) -> String {
        format!("{}:{page_id}", self.scope_prefix)
    }
}
