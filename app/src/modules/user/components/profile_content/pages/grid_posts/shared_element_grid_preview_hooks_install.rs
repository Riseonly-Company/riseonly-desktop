use crate::core::shared_element_transition::renderer::shared_element_cover_hooks::SharedElementGridPreviewHooks;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::{
    ProfileGridPreviewSourcePolicy, ProfileThumbnailDecodePolicy,
};

pub fn install_shared_element_grid_preview_hooks() {
    SharedElementGridPreviewHooks::set(SharedElementGridPreviewHooks {
        profile_grid_preview_source: ProfileGridPreviewSourcePolicy::source,
        profile_thumbnail_maximum_pixel_dimension: ProfileThumbnailDecodePolicy::bucketed_pixel_dimension,
        profile_thumbnail_maximum_display_scale: ProfileThumbnailDecodePolicy::MAXIMUM_DISPLAY_SCALE,
    });
}
