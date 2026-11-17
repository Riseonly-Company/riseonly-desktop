use std::collections::HashMap;
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;

use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{PackKind, StickerItem, StickerPack};
use crate::modules::stickers::stores::stickers::stickers_guide_sandbox_hooks::stickers_guide_sandbox_hooks;
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

const SHEET_HOST_LOG: &str = "SheetHost";
const ADD_STICKERS_MARKER: &str = "addstickers/";
const CONNECTION_WAIT: Duration = Duration::from_secs(20);

pub struct StickerPackSheetStore {
    is_presented: bool,
    pack_id: Option<String>,
    kind: PackKind,
    pending_link: Option<String>,
    reconnect_watch: Option<RiseTransportDropWatch>,
    connection_deadline: Option<Task<()>>,
    owner_ids_by_pack_id: HashMap<String, String>,
}

impl StickerPackSheetStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let _ = StickersServicesStore::shared(cx);
        Self {
            is_presented: false,
            pack_id: None,
            kind: PackKind::Sticker,
            pending_link: None,
            reconnect_watch: None,
            connection_deadline: None,
            owner_ids_by_pack_id: HashMap::new(),
        }
    }

    pub fn note_owner(&mut self, pack_id: &str, owner_id: &str, _cx: &mut Context<Self>) {
        if pack_id.is_empty() || owner_id.is_empty() {
            return;
        }
        self.owner_ids_by_pack_id.insert(pack_id.to_owned(), owner_id.to_owned());
    }

    pub fn owner_id(&self, pack_id: &str) -> Option<String> {
        self.owner_ids_by_pack_id.get(pack_id).cloned()
    }

    pub fn is_presented(&self) -> bool {
        self.is_presented
    }

    pub fn pack_id(&self) -> Option<&str> {
        self.pack_id.as_deref()
    }

    pub fn kind(&self) -> PackKind {
        self.kind
    }

    pub fn pending_link(&self) -> Option<&str> {
        self.pending_link.as_deref()
    }

    pub fn present(&mut self, pack_id: &str, kind: PackKind, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(pack_id).to_owned();
        if trimmed.is_empty() {
            return;
        }

        self.pack_id = Some(trimmed.clone());
        self.kind = kind;
        self.pending_link = None;
        self.is_presented = true;
        cx.notify();

        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_stickers(&trimmed, cx);
        });
        if self.resolved_pack(cx).is_none() {
            self.warm_pack_metadata(kind, cx);
        }
    }

    pub fn present_link(&mut self, link: &str, cx: &mut Context<Self>) {
        let trimmed = Self::normalized_link(link);
        if trimmed.is_empty() {
            return;
        }

        let known = rise_ui::shared::try_read::<StickersServicesStore>(cx)
            .and_then(|services| services.pack_by_link(&trimmed))
            .filter(|pack| !pack.id.is_empty());
        if let Some(pack) = known {
            self.pack_id = None;
            self.pending_link = Some(trimmed);
            self.adopt_resolved_pack(&pack, cx);
            self.is_presented = true;
            cx.notify();
            return;
        }

        self.pack_id = None;
        self.pending_link = Some(trimmed.clone());
        self.is_presented = true;
        cx.notify();
        self.stop_connection_wait();
        let opening = format!("pack link opening unresolved: {trimmed}");
        Sai::log_channel(SHEET_HOST_LOG, move || opening, SHEET_HOST_LOG, cx);
        self.load_link(&trimmed, cx);
    }

    fn load_link(&mut self, trimmed: &str, cx: &mut Context<Self>) {
        let resolved_store = cx.weak_entity();
        let resolved_link = trimmed.to_owned();
        let failed_store = cx.weak_entity();
        let failed_link = trimmed.to_owned();
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_pack_by_link(
                trimmed,
                Some(Box::new(move |cx: &mut App| {
                    cx.defer(move |cx| {
                        let _ = resolved_store.update(cx, |store, cx| store.finish_link_resolved(&resolved_link, cx));
                    });
                })),
                Some(Box::new(move |cx: &mut App| {
                    cx.defer(move |cx| {
                        let _ = failed_store.update(cx, |store, cx| store.finish_link_failed(&failed_link, cx));
                    });
                })),
                cx,
            );
        });
    }

    fn wait_for_connection(&mut self, link: &str, cx: &mut Context<Self>) -> bool {
        if self.connection_deadline.is_none() {
            let deadline_link = link.to_owned();
            self.connection_deadline = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CONNECTION_WAIT).await;
                let _ = this.update(cx, |store, cx| store.give_up_waiting(&deadline_link, cx));
            }));
        }
        if let Some(watch) = self.reconnect_watch.take() {
            watch.stop();
        }
        let store = cx.weak_entity();
        let connect_link = link.to_owned();
        self.reconnect_watch = Some(RiseTransportStatus::on_next_connect(
            move |cx: &mut App| {
                let _ = store.update(cx, |store, cx| {
                    if store.pending_link.as_deref() != Some(connect_link.as_str()) || store.resolved_pack(cx).is_some() {
                        return;
                    }
                    store.reconnect_watch = None;
                    store.load_link(&connect_link, cx);
                });
            },
            cx,
        ));
        RiseTransportStatus::wake_if_offline(cx);
        true
    }

    fn give_up_waiting(&mut self, link: &str, cx: &mut Context<Self>) {
        self.connection_deadline = None;
        if self.pending_link.as_deref() != Some(link) || self.resolved_pack(cx).is_some() || !self.stickers(cx).is_empty() {
            return;
        }
        self.dismiss(cx);
        Notifier::send(
            tr("error_connection_lost"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn stop_connection_wait(&mut self) {
        if let Some(watch) = self.reconnect_watch.take() {
            watch.stop();
        }
        self.connection_deadline = None;
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.stop_connection_wait();
        self.is_presented = false;
        self.pack_id = None;
        self.pending_link = None;
        cx.notify();
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.dismiss(cx);
    }

    pub fn resolved_pack(&self, cx: &App) -> Option<StickerPack> {
        let services = rise_ui::shared::try_read::<StickersServicesStore>(cx)?;
        if let Some(id) = self.pack_id.as_deref() {
            if let Some(pack) = services.pack(id) {
                return Some(pack);
            }
            if let Some(pack) = self.pack_from_lists(id, services, cx) {
                return Some(pack);
            }
        }
        self.pending_link.as_deref().and_then(|link| services.pack_by_link(link))
    }

    pub fn stickers(&self, cx: &App) -> Vec<StickerItem> {
        let Some(services) = rise_ui::shared::try_read::<StickersServicesStore>(cx) else {
            return Vec::new();
        };
        if let Some(id) = self.pack_id.as_deref() {
            let by_id = services.pack_stickers(id);
            if !by_id.is_empty() {
                return by_id;
            }
        }
        self.pending_link.as_deref().map_or_else(Vec::new, |link| services.stickers_by_link(link))
    }

    pub fn is_loading(&self, cx: &App) -> bool {
        self.resolved_pack(cx).is_none() && self.stickers(cx).is_empty() && !self.is_unavailable(cx)
    }

    pub fn is_unavailable(&self, cx: &App) -> bool {
        let Some(id) = self.pack_id.as_deref() else {
            return false;
        };
        if (stickers_guide_sandbox_hooks().owns_sticker_pack)(id, cx) {
            return false;
        }
        rise_ui::shared::try_read::<StickersServicesStore>(cx).is_some_and(|services| services.is_pack_unavailable(id))
    }

    pub fn has_loaded_stickers(&self, cx: &App) -> bool {
        if let Some(id) = self.pack_id.as_deref() {
            let resolved = rise_ui::shared::try_read::<StickersServicesStore>(cx)
                .is_some_and(|services| services.has_resolved_pack(id));
            if resolved {
                return true;
            }
        }
        !self.stickers(cx).is_empty()
    }

    pub fn reconcile_resolved_link(&mut self, cx: &mut Context<Self>) {
        if self.pack_id.is_some() {
            return;
        }
        let Some(link) = self.pending_link.as_deref() else {
            return;
        };
        let pack = rise_ui::shared::try_read::<StickersServicesStore>(cx)
            .and_then(|services| services.pack_by_link(link))
            .filter(|pack| !pack.id.is_empty());
        if let Some(pack) = pack {
            self.adopt_resolved_pack(&pack, cx);
        }
    }

    fn finish_link_resolved(&mut self, link: &str, cx: &mut Context<Self>) {
        if self.pending_link.as_deref() != Some(link) {
            return;
        }
        self.stop_connection_wait();
        let pack = rise_ui::shared::try_read::<StickersServicesStore>(cx)
            .and_then(|services| services.pack_by_link(link))
            .filter(|pack| !pack.id.is_empty());
        if let Some(pack) = pack {
            self.adopt_resolved_pack(&pack, cx);
        }
    }

    fn finish_link_failed(&mut self, link: &str, cx: &mut Context<Self>) {
        if self.pending_link.as_deref() != Some(link) || self.resolved_pack(cx).is_some() || !self.stickers(cx).is_empty()
        {
            return;
        }
        if !RiseTransportStatus::is_connected(cx) && self.wait_for_connection(link, cx) {
            return;
        }
        self.stop_connection_wait();
        let failed = format!("pack link failed, closing sheet: {link}");
        Sai::log_channel(SHEET_HOST_LOG, move || failed, SHEET_HOST_LOG, cx);
        self.dismiss(cx);
        Notifier::send(
            tr("join_link_invalid"),
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn adopt_resolved_pack(&mut self, pack: &StickerPack, cx: &mut Context<Self>) {
        self.pack_id = Some(pack.id.clone());
        self.kind = pack.pack_kind;
        cx.notify();
        if !pack.id.is_empty() {
            StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                interactions.load_pack_stickers(&pack.id, cx);
            });
        }
    }

    fn warm_pack_metadata(&mut self, kind: PackKind, cx: &mut Context<Self>) {
        let Some(user_id) = rise_ui::shared::try_read::<AuthServicesStore>(cx)
            .and_then(AuthServicesStore::effective_user_id_for_wire)
            .filter(|user_id| !user_id.is_empty())
        else {
            return;
        };
        let (has_default, has_saved, has_mine) = {
            let Some(services) = rise_ui::shared::try_read::<StickersServicesStore>(cx) else {
                return;
            };
            (
                !services.default_packs(kind).is_empty(),
                !services.saved_packs(&user_id, kind).is_empty(),
                !services.my_packs(&user_id, kind).is_empty(),
            )
        };
        StickersInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            if !has_default {
                interactions.load_default_packs(kind, None, cx);
            }
            if !has_saved {
                interactions.load_saved_packs(&user_id, kind, None, false, cx);
            }
            if !has_mine {
                interactions.load_my_packs(&user_id, kind, None, cx);
            }
        });
    }

    fn pack_from_lists(&self, id: &str, services: &StickersServicesStore, cx: &App) -> Option<StickerPack> {
        let other = if self.kind == PackKind::Sticker { PackKind::Emoji } else { PackKind::Sticker };
        let user_id = rise_ui::shared::try_read::<AuthServicesStore>(cx)
            .and_then(AuthServicesStore::effective_user_id_for_wire)
            .filter(|user_id| !user_id.is_empty());
        for kind in [self.kind, other] {
            let mut pool = services.default_packs(kind);
            if let Some(user_id) = user_id.as_deref() {
                pool.extend(services.saved_packs(user_id, kind));
                pool.extend(services.my_packs(user_id, kind));
            }
            if let Some(hit) = pool.into_iter().find(|pack| pack.id == id) {
                return Some(hit);
            }
        }
        None
    }

    fn normalized_link(raw: &str) -> String {
        let mut value = trimming_whitespaces_and_newlines(raw);
        if let Some(start) = value.to_ascii_lowercase().find(ADD_STICKERS_MARKER) {
            value = &value[start + ADD_STICKERS_MARKER.len()..];
        }
        if let Some(slash) = value.find('/') {
            value = &value[..slash];
        }
        if let Some(query) = value.find('?') {
            value = &value[..query];
        }
        trimming_whitespaces_and_newlines(value).to_owned()
    }
}
