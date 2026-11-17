use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::rise_key::{bot, user as key};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde_json::Value;

use crate::core::lib::any_dictionary_bridge::AnyDictionaryBridge;
use crate::core::lib::sai::core::utilities::sai_cache::SaiCache;
use crate::core::lib::sai::modules::sai_updater::sai_updater::{SaiUpdater, SaiUpdaterWriteScope};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{SaiWsCachePreferredLayer, SaiWsDiskCachePolicy};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::user::engine::rise_user_profile_photo_upload::RiseProfilePhotoDraft;
use crate::modules::user::engine::rise_user_profile_photos_presentation::{
    RiseProfilePhotoAlbum, RiseUserProfilePhotosLoadState,
};
use crate::modules::user::engine::rise_user_profile_photos_repository::RiseUserProfilePhotosRepository;
use crate::modules::user::stores::user_profile_photos::user_profile_photos_actions::user_profile_photos_actions::UserProfilePhotosActionsStore;
use crate::modules::user::stores::user_profile_photos::user_profile_photos_services::user_profile_photos_services::UserProfilePhotosServicesStore;

pub struct UserProfilePhotosInteractionsStore {}

impl UserProfilePhotosInteractionsStore {
    pub const MAXIMUM_PHOTOS: i64 = RiseUserProfilePhotosRepository::MAXIMUM_PHOTOS;

    const PAGE_PREFETCH_THRESHOLD: usize = 8;

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn actions(cx: &mut App) -> Entity<UserProfilePhotosActionsStore> {
        UserProfilePhotosActionsStore::shared(cx)
    }

    fn services(cx: &mut App) -> Entity<UserProfilePhotosServicesStore> {
        UserProfilePhotosServicesStore::shared(cx)
    }

    pub fn seed_album(&mut self, user_id: &str, cx: &mut Context<Self>) -> Task<Option<RiseProfilePhotoAlbum>> {
        let Some(user_id) = Self::normalized(Some(user_id)) else {
            return Task::ready(None);
        };

        Self::actions(cx).update(cx, |actions, cx| actions.seed_action(user_id, cx))
    }

    pub fn open_album(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let Some(user_id) = Self::normalized(Some(user_id)) else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| actions.load_action(user_id, false, cx));
    }

    pub fn refresh_album(&mut self, user_id: &str, cx: &mut Context<Self>) -> Task<()> {
        let Some(user_id) = Self::normalized(Some(user_id)) else {
            return Task::ready(());
        };

        Self::actions(cx).update(cx, |actions, cx| actions.await_load_action(user_id, true, cx))
    }

    pub fn load_more_if_needed(&mut self, user_id: &str, visible_index: usize, cx: &mut Context<Self>) {
        let Some(user_id) = Self::normalized(Some(user_id)) else {
            return;
        };
        let album = Self::services(cx).update(cx, |services, cx| services.album(&user_id, cx));
        let Some(album) = album else {
            return;
        };
        if !album.has_more || album.is_refreshing {
            return;
        }
        if album.photos.len().saturating_sub(visible_index) > Self::PAGE_PREFETCH_THRESHOLD {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.load_more_action(user_id, cx));
    }

    pub fn add_photos(&mut self, drafts: Vec<RiseProfilePhotoDraft>, cx: &mut Context<Self>) {
        let drafts: Vec<RiseProfilePhotoDraft> = drafts.into_iter().filter(|draft| !draft.filename.is_empty()).collect();
        if drafts.is_empty() {
            return;
        }

        Self::actions(cx).update(cx, |actions, cx| actions.add_photos_action(drafts, cx));
    }

    pub fn remaining_slots(&mut self, user_id: &str, cx: &mut Context<Self>) -> i64 {
        let Some(user_id) = Self::normalized(Some(user_id)) else {
            return 0;
        };
        let album = Self::services(cx).update(cx, |services, cx| services.album(&user_id, cx));
        let held = match album {
            Some(album) if album.load_state == RiseUserProfilePhotosLoadState::Loaded => {
                album.total.max(i64::try_from(album.photos.len()).unwrap_or(i64::MAX))
            }
            _ => AuthServicesStore::shared(cx)
                .read(cx)
                .profile()
                .map(|profile| if profile.id == user_id { profile.profile_photos_count } else { 0 })
                .unwrap_or(0),
        };
        (Self::MAXIMUM_PHOTOS - held).max(0)
    }

    pub fn settle_account_avatar(&mut self, logo: String, cx: &mut Context<Self>) {
        Self::apply_avatar_to_own_profile(&logo, cx);
    }

    pub fn set_as_main(&mut self, photo_id: &str, cx: &mut Context<Self>) {
        let Some(photo_id) = Self::normalized(Some(photo_id)) else {
            return;
        };

        let request = Self::actions(cx).update(cx, |actions, cx| actions.set_main_action(photo_id, cx));
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(logo) = request.await else {
                return;
            };

            cx.update(|cx| Self::apply_avatar_to_own_profile(&logo, cx));
        })
        .detach();
    }

    pub fn delete_photo(&mut self, photo_id: &str, cx: &mut Context<Self>) {
        let Some(photo_id) = Self::normalized(Some(photo_id)) else {
            return;
        };

        let request = Self::actions(cx).update(cx, |actions, cx| actions.delete_action(photo_id, cx));
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(logo) = request.await else {
                return;
            };

            cx.update(|cx| Self::apply_avatar_to_own_profile(&logo, cx));
        })
        .detach();
    }

    pub fn invalidate_album(&mut self, user_id: &str, cx: &mut Context<Self>) {
        let Some(user_id) = Self::normalized(Some(user_id)) else {
            return;
        };

        Self::actions(cx).update(cx, |actions, cx| actions.invalidate_action(user_id, cx));
    }

    fn apply_avatar_to_own_profile(avatar_url: &str, cx: &mut App) {
        let avatar_url = trimming_whitespaces_and_newlines(avatar_url).to_owned();
        let auth = AuthServicesStore::shared(cx);
        let Some(mut profile) = auth.read(cx).profile().cloned() else {
            return;
        };
        if profile.avatar_url.as_deref() == Some(avatar_url.as_str()) && profile.more.logo == avatar_url {
            return;
        }

        let account_id = profile.id.clone();
        profile.avatar_url = if avatar_url.is_empty() { None } else { Some(avatar_url.clone()) };
        profile.more.logo = avatar_url.clone();
        auth.update(cx, |auth, cx| auth.apply_updated_session_profile(profile, cx));

        let cache_key = SaiCache::new(["getMyProfile", account_id.as_str()]).key;
        let cached = SaiWsResponseCache::current_payload(
            &cache_key,
            SaiWsCachePreferredLayer::MemoryFirst,
            SaiWsDiskCachePolicy::PreserveExisting,
            cx,
        );
        if cached.is_none() {
            return;
        }
        let _ = SaiUpdater::update(
            &cache_key,
            |context| {
                let mut root = AnyDictionaryBridge::dictionary(context.payload.as_ref()).unwrap_or_default();
                if let Some(mut more) = AnyDictionaryBridge::dictionary(root.get(key::MORE)) {
                    more.insert(key::LOGO.to_owned(), Value::String(avatar_url.clone()));
                    root.insert(key::MORE.to_owned(), Value::Object(more));
                }
                root.insert(bot::AVATAR_URL.to_owned(), Value::String(avatar_url.clone()));
                Ok(serde_json::to_vec(&Value::Object(root))?)
            },
            SaiUpdaterWriteScope::All,
            cx,
        );
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(value?);
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }
}
