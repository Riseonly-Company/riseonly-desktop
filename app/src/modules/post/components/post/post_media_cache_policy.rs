use gpui::App;

use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

pub enum PostMediaCachePolicy {}

impl PostMediaCachePolicy {
    /// flat author_id is asked first, nested author.id second
    /// flat is the rows own foreign key and always there, nested is a join the gateway may omit
    pub fn is_own(author_id: Option<&str>, nested_author_id: Option<&str>, cx: &mut App) -> bool {
        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        RiseIdentity::is_own(
            &RiseIdentity::author(author_id, None, nested_author_id),
            viewer.as_deref(),
            None,
        )
    }

    pub fn is_own_post(post: &GetPostFeedResponse, cx: &mut App) -> bool {
        Self::is_own(Some(post.author_id.as_str()), post.author.id.as_deref(), cx)
    }

    pub fn image_policy(is_own: bool) -> CleverImageCachePolicy {
        if is_own {
            CleverImageCachePolicy::Standard
        } else {
            CleverImageCachePolicy::MemoryOnly
        }
    }

    pub const FEED_IMAGE_POLICY: CleverImageCachePolicy = CleverImageCachePolicy::Standard;
    pub const FEED_IMAGES_PERSIST_TO_DISK: bool = true;
}
