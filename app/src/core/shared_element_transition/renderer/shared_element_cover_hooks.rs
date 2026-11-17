use gpui::{AnyElement, Pixels, Size};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// hooks installed by the post module
pub struct SharedElementCoverPostHooks {
    pub post_grid_text_ui: fn(&GetPostFeedResponse) -> AnyElement,
}

pub(crate) static SHARED_ELEMENT_COVER_POST_HOOKS: FeatureHookSlot<SharedElementCoverPostHooks> = FeatureHookSlot::new("SHARED_ELEMENT_COVER_POST_HOOKS");

impl SharedElementCoverPostHooks {
    /// second install is ignored, first owner keeps the slot
    pub fn set(hooks: Self) {
        let _ = SHARED_ELEMENT_COVER_POST_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        SHARED_ELEMENT_COVER_POST_HOOKS.get()
    }

    /// none until post installs the slot
    pub fn post_grid_text_ui(post: &GetPostFeedResponse) -> Option<AnyElement> {
        Self::installed().map(|hooks| (hooks.post_grid_text_ui)(post))
    }
}

/// hooks installed by the profile reposts tab
pub struct SharedElementCoverRepostHooks {
    pub profile_grid_repost_text_ui: fn(&GetPostFeedResponse) -> AnyElement,
}

pub(crate) static SHARED_ELEMENT_COVER_REPOST_HOOKS: FeatureHookSlot<SharedElementCoverRepostHooks> = FeatureHookSlot::new("SHARED_ELEMENT_COVER_REPOST_HOOKS");

impl SharedElementCoverRepostHooks {
    /// second install is ignored, first owner keeps the slot
    pub fn set(hooks: Self) {
        let _ = SHARED_ELEMENT_COVER_REPOST_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        SHARED_ELEMENT_COVER_REPOST_HOOKS.get()
    }

    /// none until the reposts tab installs the slot
    pub fn profile_grid_repost_text_ui(post: &GetPostFeedResponse) -> Option<AnyElement> {
        Self::installed().map(|hooks| (hooks.profile_grid_repost_text_ui)(post))
    }
}

/// hooks installed by the profile grid
pub struct SharedElementGridPreviewHooks {
    pub profile_grid_preview_source: fn(Option<&str>, Option<&str>, Option<&str>) -> Option<String>,
    pub profile_thumbnail_maximum_pixel_dimension: fn(Size<Pixels>, f32) -> i64,
    pub profile_thumbnail_maximum_display_scale: f32,
}

pub(crate) static SHARED_ELEMENT_GRID_PREVIEW_HOOKS: FeatureHookSlot<SharedElementGridPreviewHooks> = FeatureHookSlot::new("SHARED_ELEMENT_GRID_PREVIEW_HOOKS");

impl SharedElementGridPreviewHooks {
    /// second install is ignored, first owner keeps the slot
    pub fn set(hooks: Self) {
        let _ = SHARED_ELEMENT_GRID_PREVIEW_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        SHARED_ELEMENT_GRID_PREVIEW_HOOKS.get()
    }

    /// none until the profile grid installs the slot, so no grid cover before that
    pub fn profile_grid_preview_source(
        local_preview_url: Option<&str>,
        first_image: Option<&str>,
        video_thumbnail: Option<&str>,
    ) -> Option<String> {
        Self::installed()
            .and_then(|hooks| (hooks.profile_grid_preview_source)(local_preview_url, first_image, video_thumbnail))
    }

    /// none until the profile grid installs the slot
    pub fn profile_thumbnail_maximum_pixel_dimension(rendered_size: Size<Pixels>, display_scale: f32) -> Option<i64> {
        Self::installed().map(|hooks| (hooks.profile_thumbnail_maximum_pixel_dimension)(rendered_size, display_scale))
    }

    /// none until the profile grid installs the slot
    pub fn profile_thumbnail_maximum_display_scale() -> Option<f32> {
        Self::installed().map(|hooks| hooks.profile_thumbnail_maximum_display_scale)
    }
}
