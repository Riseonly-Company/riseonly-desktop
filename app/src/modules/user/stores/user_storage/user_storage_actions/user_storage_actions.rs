use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity};

use crate::core::storage::rise_storage_usage_models::{RiseStorageMediaEntry, RiseStorageSite};
use crate::core::storage::rise_storage_usage_presentation::RiseStorageUsagePresentation;

pub struct UserStorageActionsStore {}

impl UserStorageActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn presentation(cx: &mut App) -> Entity<RiseStorageUsagePresentation> {
        RiseStorageUsagePresentation::shared(cx)
    }

    pub fn scan_action(&mut self, force: bool, cx: &mut Context<Self>) {
        Self::presentation(cx).update(cx, |presentation, cx| presentation.refresh(force, cx));
    }

    pub fn scan_chats_action(&mut self, titles: HashMap<String, String>, cx: &mut Context<Self>) {
        Self::presentation(cx).update(cx, |presentation, cx| presentation.refresh_chats(titles, cx));
    }

    pub fn scan_media_action(&mut self, cx: &mut Context<Self>) {
        Self::presentation(cx).update(cx, |presentation, cx| presentation.refresh_media(cx));
    }

    pub fn clear_all_action(&mut self, cx: &mut Context<Self>) {
        Self::presentation(cx)
            .update(cx, |presentation, cx| presentation.clear_everything(cx))
            .detach();
    }

    pub fn clear_sites_action(&mut self, sites: HashSet<RiseStorageSite>, cx: &mut Context<Self>) {
        Self::presentation(cx)
            .update(cx, |presentation, cx| presentation.clear(sites, cx))
            .detach();
    }

    pub fn clear_chat_action(&mut self, chat_id: String, cx: &mut Context<Self>) {
        Self::presentation(cx)
            .update(cx, |presentation, cx| presentation.clear_chat(chat_id, cx))
            .detach();
    }

    pub fn clear_media_entry_action(&mut self, entry: RiseStorageMediaEntry, cx: &mut Context<Self>) {
        Self::presentation(cx)
            .update(cx, |presentation, cx| presentation.clear_media_entry(entry, cx))
            .detach();
    }
}
