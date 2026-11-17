use gpui::{App, AsyncApp, Context, Entity, Task};

use crate::modules::user::engine::rise_user_domain::RiseUserActionExecutor;
use crate::modules::user::engine::rise_user_profile_photo_upload::RiseProfilePhotoDraft;
use crate::modules::user::engine::rise_user_profile_photos_presentation::RiseProfilePhotoAlbum;

pub struct UserProfilePhotosActionsStore {}

impl UserProfilePhotosActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn seed_action(&mut self, user_id: String, cx: &mut Context<Self>) -> Task<Option<RiseProfilePhotoAlbum>> {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return None;
            };

            domain.seed_profile_photos(&user_id).await
        })
    }

    pub fn load_action(&mut self, user_id: String, force: bool, cx: &mut Context<Self>) {
        self.await_load_action(user_id, force, cx).detach();
    }

    pub fn await_load_action(&mut self, user_id: String, force: bool, cx: &mut Context<Self>) -> Task<()> {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };

            domain.load_profile_photos(&user_id, force).await;
        })
    }

    pub fn load_more_action(&mut self, user_id: String, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };

            domain.load_more_profile_photos(&user_id).await;
        })
        .detach();
    }

    pub fn add_photos_action(&mut self, drafts: Vec<RiseProfilePhotoDraft>, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };

            domain.add_profile_photos(drafts).await;
        })
        .detach();
    }

    pub fn set_main_action(&mut self, photo_id: String, cx: &mut Context<Self>) -> Task<anyhow::Result<String>> {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = domain.await?;

            domain.set_main_profile_photo(&photo_id).await
        })
    }

    pub fn delete_action(&mut self, photo_id: String, cx: &mut Context<Self>) -> Task<anyhow::Result<String>> {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = domain.await?;

            domain.delete_profile_photo(&photo_id).await
        })
    }

    pub fn invalidate_action(&mut self, user_id: String, cx: &mut Context<Self>) {
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };

            domain.invalidate_profile_photos(&user_id).await;
        })
        .detach();
    }
}
