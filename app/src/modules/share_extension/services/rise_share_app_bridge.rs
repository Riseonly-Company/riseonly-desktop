use std::collections::{HashMap, HashSet};

use gpui::{App, RenderImage};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_platform::file_manager::FileManager;
use rise_theme::avatar_palette::AvatarPalette;

use crate::core::lib::canon_identity::RiseIdentity;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_environment::SaiWebSocketEnvironment;
use crate::core::lib::sai::sai::Sai;
use crate::core::stores::http::http_service::HttpService;
use crate::core::ui::clever_image_disk_cache::{
    CleverImageBitmapEncoder, CleverImageBitmapLoader, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::auth::services::access_token_user_id::AccessTokenUserId;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::share_extension::shared::rise_share_container::RiseShareContainer;
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareAccount, RiseShareSnapshot, RiseShareTarget, RiseShareTargetKind,
};
use crate::modules::share_extension::stores::share_targets::share_targets_interactions::ShareTargetsInteractionsStore;
use crate::modules::share_extension::stores::share_targets::share_targets_services::ShareTargetsServicesStore;

const MAXIMUM_WARMED_AVATARS: usize = 100;
const SYNCHRONOUSLY_WARMED_AVATARS: usize = 24;
const AVATAR_THUMBNAIL_SIDE: f64 = 180.0;
const AVATAR_THUMBNAIL_JPEG_QUALITY: u8 = 82;

/// bridge between the app and the share layer
pub enum RiseShareAppBridge {}

impl RiseShareAppBridge {
    pub fn reconcile_persisted_credential() {}

    /// syncs the credential, does nothing on desktop
    pub fn synchronize_credential(_prefer_local: bool) {}

    pub fn write_snapshot(targets: &[RiseShareTarget], cx: &mut App) {
        let auth = AuthServicesStore::shared(cx);
        let account = {
            let auth = auth.read(cx);
            if !auth.is_authenticated() {
                return;
            }
            let Some(profile) = auth.profile() else {
                return;
            };
            let Some(account_id) = auth.active_account_id().map(str::to_owned) else {
                return;
            };
            let owns_account = profile.id == account_id
                || auth.effective_user_id_for_wire().as_deref() == Some(account_id.as_str());
            if !owns_account {
                return;
            }
            RiseShareAccount {
                id: account_id,
                name: profile.name.clone(),
                tag: profile.tag.clone(),
                avatar_url: Some(profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone())),
            }
        };

        let chats = ChatServicesStore::shared(cx).update(cx, |chats, cx| chats.current_chat_infos(cx));
        let mut peer_ids_by_chat_id: HashMap<String, String> = HashMap::new();
        for chat in &chats {
            peer_ids_by_chat_id
                .entry(chat.id.clone())
                .or_insert_with(|| trimming_whitespaces_and_newlines(&chat.participant.id).to_owned());
        }
        let enriched_targets: Vec<RiseShareTarget> = targets
            .iter()
            .map(|target| {
                let peer_id = if target.kind == RiseShareTargetKind::PrivateChat {
                    peer_ids_by_chat_id.get(&target.id).filter(|peer_id| !peer_id.is_empty())
                } else {
                    None
                };
                let color_key = peer_id.cloned().unwrap_or_else(|| target.id.clone());
                RiseShareTarget::new(
                    target.id.clone(),
                    target.title.clone(),
                    target.avatar_url.clone(),
                    Some(target.avatar_color_key.clone().unwrap_or(color_key)),
                    target.kind,
                    target.topics_enabled,
                    target.can_send,
                    target.last_activity_ms,
                )
            })
            .collect();
        let snapshot = RiseShareSnapshot::new(
            Date::now().ms(),
            HttpService::resolved_api_base_url(),
            SaiWebSocketEnvironment::resolved_web_socket_url(),
            account,
            enriched_targets,
        );
        match RiseShareContainer::save_snapshot(&snapshot) {
            Ok(()) => Self::warm_shared_avatar_cache(&snapshot, cx),
            Err(error) => Sai::log(
                "ShareExtension",
                || format!("snapshot write failed: {error}"),
                SaiLogKind::Network,
                cx,
            ),
        }
    }

    pub fn write_snapshot_keeping_persisted_targets(targets: &[RiseShareTarget], cx: &mut App) {
        if !targets.is_empty() {
            Self::write_snapshot(targets, cx);
            return;
        }
        let account_id = AuthServicesStore::shared(cx).read(cx).active_account_id().map(str::to_owned);
        let persisted = RiseShareContainer::load_snapshot()
            .ok()
            .filter(|snapshot| Some(snapshot.account.id.as_str()) == account_id.as_deref())
            .map(|snapshot| snapshot.targets)
            .unwrap_or_default();
        if !persisted.is_empty() {
            Self::write_snapshot(&persisted, cx);
            return;
        }
        let chats = ChatServicesStore::shared(cx).update(cx, |chats, cx| chats.current_chat_infos(cx));
        let mut from_chats: Vec<RiseShareTarget> = chats
            .iter()
            .filter(|chat| matches!(chat.r#type, ChatType::Favourites | ChatType::Direct))
            .map(Self::share_target)
            .filter(|target| !target.is_deleted)
            .collect();
        from_chats.sort_by(|lhs, rhs| rhs.last_activity_ms.cmp(&lhs.last_activity_ms).then_with(|| lhs.id.cmp(&rhs.id)));
        Self::write_snapshot(&from_chats, cx);
    }

    pub fn persisted_writable_target_ids_for_current_account(cx: &mut App) -> HashSet<String> {
        let Some(account_id) = AuthServicesStore::shared(cx).read(cx).active_account_id().map(str::to_owned) else {
            return HashSet::new();
        };
        let Ok(snapshot) = RiseShareContainer::load_snapshot() else {
            return HashSet::new();
        };
        if snapshot.account.id != account_id {
            return HashSet::new();
        }
        snapshot
            .targets
            .into_iter()
            .filter(|target| target.can_send)
            .map(|target| target.id)
            .collect()
    }

    /// ids of writable targets that were removed, sorted
    pub fn removed_writable_target_ids(previous: &HashSet<String>, fresh: &HashSet<String>) -> Vec<String> {
        let mut removed: Vec<String> = previous.difference(fresh).cloned().collect();
        removed.sort();
        removed
    }

    pub fn remove_donated_interactions(_target_ids: &[String]) {}

    /// no hint registry on desktop, but refreshing an unknown chat keeps the share snapshot complete
    pub fn donate_outgoing_interaction(chat_id: &str, cx: &mut App) {
        let listed = ShareTargetsServicesStore::shared(cx)
            .read(cx)
            .targets()
            .iter()
            .find(|target| target.id == chat_id)
            .cloned();
        if let Some(target) = listed {
            if target.can_send {
                Self::donate_outgoing_interaction_target(&target);
            } else {
                Self::donate_outgoing_from_chat_info(chat_id, cx);
            }
            return;
        }

        let account_id = AuthServicesStore::shared(cx).read(cx).active_account_id().map(str::to_owned);
        let user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |cx| {
            let snapshot_account_id = account_id.clone();
            let snapshot_chat_id = chat_id.clone();
            let snapshot_target: Option<RiseShareTarget> = cx
                .background_executor()
                .spawn(async move {
                    let snapshot = RiseShareContainer::load_snapshot().ok()?;
                    if snapshot_account_id.as_ref().is_some_and(|account_id| *account_id != snapshot.account.id) {
                        return None;
                    }
                    snapshot.targets.into_iter().find(|target| target.id == snapshot_chat_id)
                })
                .await;
            cx.update(|cx| {
                if AuthServicesStore::shared(cx).read(cx).active_account_id() != account_id.as_deref()
                    || !SaiAccountCacheScope::is_current_account_epoch(user_id.as_deref(), account_epoch.as_deref())
                {
                    return;
                }
                if let Some(snapshot_target) = snapshot_target
                    && snapshot_target.can_send
                {
                    Self::donate_outgoing_interaction_target(&snapshot_target);
                    return;
                }
                Self::donate_outgoing_from_chat_info(&chat_id, cx);
            });
        })
        .detach();
    }

    fn donate_outgoing_from_chat_info(chat_id: &str, cx: &mut App) {
        let chats = ChatServicesStore::shared(cx).update(cx, |chats, cx| chats.current_chat_infos(cx));
        if let Some(chat) = chats.iter().find(|chat| chat.id == chat_id) {
            Self::donate_outgoing_interaction_target(&Self::share_target(chat));
            ShareTargetsInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh(cx));
        }
    }

    /// donates an outgoing interaction for a target, nothing on desktop
    pub fn donate_outgoing_interaction_target(_target: &RiseShareTarget) {}

    /// donates an incoming interaction, nothing on desktop
    pub fn donate_incoming_interaction(_chat_id: &str, _sender_id: &str) {}

    pub fn clear_shared_state() {
        RiseShareContainer::clear_avatar_cache();
        if let Ok(root) = RiseShareContainer::root_url() {
            let _ = FileManager.remove_item(&root.join(RiseShareContainer::SNAPSHOT_FILENAME));
        }
    }

    pub fn clear_donated_interactions() {}

    pub fn should_adopt(shared_access_token: &str, current_access_token: &str) -> bool {
        if shared_access_token == current_access_token {
            return true;
        }
        let shared_expiration = AccessTokenUserId::expiration_date(shared_access_token);
        let current_expiration = AccessTokenUserId::expiration_date(current_access_token);
        match (shared_expiration, current_expiration) {
            (Some(shared), Some(current)) => shared >= current,
            (Some(_), None) => true,
            (None, Some(_)) => AccessTokenUserId::is_expired(current_access_token, 0.0),
            (None, None) => true,
        }
    }

    fn share_target(chat: &ChatInfo) -> RiseShareTarget {
        let title = if chat.r#type == ChatType::Direct {
            chat.direct_peer_profile_name()
        } else {
            chat.title.clone()
        };
        let private_avatar = trimming_whitespaces_and_newlines(&chat.participant.more.logo);
        let avatar_url = if chat.r#type == ChatType::Direct && !private_avatar.is_empty() {
            Some(private_avatar.to_owned())
        } else {
            chat.logo_url.clone()
        };
        let kind = match chat.r#type {
            ChatType::Direct => RiseShareTargetKind::PrivateChat,
            ChatType::Group => RiseShareTargetKind::Group,
            ChatType::Channel => RiseShareTargetKind::Channel,
            ChatType::Favourites => RiseShareTargetKind::Favourites,
        };
        let raw_activity = chat.last_activity.unwrap_or(chat.updated_at);
        let activity_ms = RiseIdentity::milliseconds_from_double(raw_activity);
        let avatar_color_key = if chat.r#type == ChatType::Direct {
            trimming_whitespaces_and_newlines(&chat.participant.id).to_owned()
        } else {
            chat.id.clone()
        };
        let mut target = RiseShareTarget::new(
            chat.id.clone(),
            title,
            avatar_url,
            Some(avatar_color_key),
            kind,
            chat.topics_enabled,
            true,
            activity_ms,
        );
        if chat.r#type == ChatType::Direct && chat.participant.is_deleted {
            target.is_deleted = true;
            target.title = tr("chat_deleted_account");
            target.avatar_url = None;
        }
        target
    }

    fn warm_shared_avatar_cache(snapshot: &RiseShareSnapshot, cx: &mut App) {
        let mut seen: HashSet<String> = HashSet::new();
        let unique: Vec<Url> = std::iter::once(snapshot.account.avatar_url.as_deref())
            .chain(snapshot.targets.iter().map(|target| target.avatar_url.as_deref()))
            .filter_map(|raw| {
                let raw = AvatarPalette::resolved_logo_url(raw)?;
                let url = Url::from_string(&raw)?;
                let scheme = url.scheme()?.to_lowercase();
                (scheme == "https" || scheme == "http").then_some(url)
            })
            .filter(|url| seen.insert(url.absolute_string().to_owned()))
            .take(MAXIMUM_WARMED_AVATARS)
            .collect();
        if unique.is_empty() {
            return;
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();

        cx.spawn(async move |cx| {
            let mut first = unique;
            let rest = first.split_off(first.len().min(SYNCHRONOUSLY_WARMED_AVATARS));
            let bitmap_user_id = account_user_id.clone();
            let bitmap_epoch = account_epoch.clone();
            let mut missing: Vec<Url> = cx
                .background_executor()
                .spawn(async move {
                    let mut missing = Vec::new();
                    for url in first {
                        if RiseShareContainer::avatar_image_data(&url).is_some() {
                            continue;
                        }
                        if let Some(image) = CleverImageBitmapLoader::image_from_memory_or_disk(
                            url.absolute_string(),
                            true,
                            bitmap_user_id.as_deref(),
                            bitmap_epoch.as_deref(),
                            CleverImageCachePolicy::Standard,
                            None,
                        ) && let Some(data) = Self::avatar_thumbnail_data(&image)
                        {
                            let _ = RiseShareContainer::store_avatar_image_data(&data, &url);
                        } else {
                            missing.push(url);
                        }
                    }
                    missing
                })
                .await;
            missing.extend(rest);

            for url in missing {
                let probe = url.clone();
                let cached = cx
                    .background_executor()
                    .spawn(async move { RiseShareContainer::avatar_image_data(&probe).is_some() })
                    .await;
                if cached {
                    continue;
                }
                let Some(image) = CleverImageBitmapLoader::load(
                    url.absolute_string(),
                    true,
                    true,
                    account_user_id.clone(),
                    account_epoch.clone(),
                    None,
                    CleverImageCachePolicy::Standard,
                    CleverImageWorkLane::Visible,
                    cx,
                )
                .await
                else {
                    continue;
                };
                cx.background_executor()
                    .spawn(async move {
                        if let Some(data) = Self::avatar_thumbnail_data(&image) {
                            let _ = RiseShareContainer::store_avatar_image_data(&data, &url);
                        }
                    })
                    .await;
            }
        })
        .detach();
    }

    fn avatar_thumbnail_data(image: &RenderImage) -> Option<Vec<u8>> {
        let bitmap: RgbaImage = CleverImageBitmapEncoder::rgba(image)?;
        if bitmap.width() == 0 || bitmap.height() == 0 {
            return None;
        }
        let width = f64::from(bitmap.width());
        let height = f64::from(bitmap.height());
        let side = AVATAR_THUMBNAIL_SIDE;
        let scale = (side / width).max(side / height);
        let draw_width = width * scale;
        let draw_height = height * scale;
        let origin_x = (side - draw_width) / 2.0;
        let origin_y = (side - draw_height) / 2.0;
        let resized = imageops::resize(
            &bitmap,
            (draw_width.round() as u32).max(1),
            (draw_height.round() as u32).max(1),
            FilterType::Triangle,
        );
        let side_pixels = side as u32;
        let mut thumbnail = RgbaImage::new(side_pixels, side_pixels);
        imageops::overlay(&mut thumbnail, &resized, origin_x.round() as i64, origin_y.round() as i64);
        let rgb = DynamicImage::ImageRgba8(thumbnail).into_rgb8();
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, AVATAR_THUMBNAIL_JPEG_QUALITY)
            .encode_image(&rgb)
            .ok()?;
        Some(data)
    }
}
