use gpui::App;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftRecipient;
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;
use crate::modules::user::stores::user::user_actions::user_wire_types::UserGetProfileByTagGatewayDTOExt;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub enum GiftPeople {}

impl GiftPeople {
    pub fn me(cx: &mut App) -> Option<GiftRecipient> {
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile()?;
        if profile.id.is_empty() {
            return None;
        }
        Some(GiftRecipient {
            id: profile.id.clone(),
            name: profile.name.clone(),
            tag: profile.tag.clone(),
            avatar: profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()),
            cover: profile.cover_image_url.clone(),
        })
    }

    pub fn person(raw: &str, cx: &mut App) -> Option<GiftRecipient> {
        let id = trimming_whitespaces_and_newlines(raw);
        if id.is_empty() {
            return None;
        }
        let is_current = AuthServicesStore::shared(cx).read(cx).is_current_user(id);
        if is_current {
            return Self::me(cx);
        }
        let gifts = GiftsServicesStore::shared(cx);
        if let Some(known) = gifts.read(cx).person(id) {
            return Some(known.clone());
        }
        let users = UserServicesStore::shared(cx);
        let payload = users.update(cx, |users, cx| users.foreign_profile_payload(id, cx));
        if let Some(profile) = payload.as_ref().and_then(|payload| payload.as_profile_model()) {
            return Some(GiftRecipient {
                id: profile.id.clone(),
                name: profile.name.clone(),
                tag: profile.tag.clone(),
                avatar: profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()),
                cover: profile.cover_image_url.clone(),
            });
        }
        let chats = ChatServicesStore::shared(cx);
        let chat_id = chats.read(cx).known_direct_chat_id(id, cx);
        if chat_id.is_empty() {
            return None;
        }
        let peer = chats.read(cx).chat_info(&chat_id, cx)?.participant;
        if peer.id != id {
            return None;
        }
        Some(GiftRecipient {
            id: peer.id,
            name: peer.name,
            tag: peer.tag,
            avatar: peer.more.logo,
            cover: None,
        })
    }
}
