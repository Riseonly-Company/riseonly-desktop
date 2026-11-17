use gpui::{App, Context, Entity};

use crate::modules::user::engine::core::rise_user_profile_photo_wire_models::RiseProfilePhoto;
use crate::modules::user::engine::rise_user_profile_photos_presentation::{
    RiseProfilePhotoAlbum, RiseUserProfilePhotosPresentation, RiseUserProfilePhotosSnapshot,
};

pub struct UserProfilePhotosServicesStore {}

impl UserProfilePhotosServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn presentation(cx: &mut App) -> Entity<RiseUserProfilePhotosPresentation> {
        RiseUserProfilePhotosPresentation::shared(cx)
    }

    pub fn snapshot(&self, cx: &mut App) -> RiseUserProfilePhotosSnapshot {
        Self::presentation(cx).read(cx).snapshot().clone()
    }

    fn reading<R>(user_id: &str, cx: &mut App, read: impl FnOnce(Option<&RiseProfilePhotoAlbum>) -> R) -> R {
        let entity = Self::presentation(cx);
        read(entity.read(cx).snapshot().album(user_id))
    }

    pub fn album(&self, user_id: &str, cx: &mut App) -> Option<RiseProfilePhotoAlbum> {
        Self::reading(user_id, cx, |album| album.cloned())
    }

    pub fn photos(&self, user_id: &str, cx: &mut App) -> Vec<RiseProfilePhoto> {
        Self::reading(user_id, cx, |album| album.map(|album| album.photos.clone()).unwrap_or_default())
    }

    pub fn total(&self, user_id: &str, cx: &mut App) -> i64 {
        Self::reading(user_id, cx, |album| album.map(|album| album.total).unwrap_or(0))
    }

    pub fn main_index(&self, user_id: &str, cx: &mut App) -> Option<usize> {
        Self::reading(user_id, cx, |album| album.and_then(RiseProfilePhotoAlbum::main_index))
    }

    pub fn is_visible(&self, user_id: &str, cx: &mut App) -> bool {
        Self::reading(user_id, cx, |album| album.map(|album| album.is_visible).unwrap_or(true))
    }

    pub fn has_more(&self, user_id: &str, cx: &mut App) -> bool {
        Self::reading(user_id, cx, |album| album.is_some_and(|album| album.has_more))
    }

    pub fn is_refreshing(&self, user_id: &str, cx: &mut App) -> bool {
        Self::reading(user_id, cx, |album| album.is_some_and(|album| album.is_refreshing))
    }

    pub fn is_mutating(&self, user_id: &str, photo_id: &str, cx: &mut App) -> bool {
        Self::reading(user_id, cx, |album| album.is_some_and(|album| album.mutating_ids.contains(photo_id)))
    }

    pub fn load_error(&self, user_id: &str, cx: &mut App) -> Option<String> {
        Self::reading(user_id, cx, |album| album.and_then(|album| album.error.clone()))
    }

    pub fn mutation_error(&self, user_id: &str, cx: &mut App) -> Option<String> {
        Self::reading(user_id, cx, |album| album.and_then(|album| album.mutation_error.clone()))
    }
}
