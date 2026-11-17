use std::future::Future;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::RiseTimingCanon;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_timing::{RiseTimingCanonExt, RiseTimingDebounceExt};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::stickers::engine::rise_sticker_models::{
    RiseStickerMutationID, RiseStickerPickerTab, StickerMediaUploadDraft,
};
use crate::modules::stickers::engine::rise_sticker_repository::RiseStickerRepository;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_actions::{
    StickerLinkCallback, StickersActionsStore,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    GifItem, PackKind, StickerAssetType, StickerItem, StickerPack,
};
use crate::modules::stickers::stores::stickers::stickers_guide_sandbox_hooks::stickers_guide_sandbox_hooks;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

static RENAME_TITLE_RULE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("stickers_edit_pack", "/title"));

pub struct StickersInteractionsStore {
    search_task: Option<Task<()>>,
}

impl StickersInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { search_task: None }
    }

    fn actions(cx: &mut App) -> Entity<StickersActionsStore> {
        StickersActionsStore::shared(cx)
    }

    fn service(cx: &mut App) -> Entity<StickersServicesStore> {
        StickersServicesStore::shared(cx)
    }

    pub fn picker_did_open(&mut self, tab: RiseStickerPickerTab, cx: &mut Context<Self>) {
        self.with_repository(move |repository| async move { repository.picker_did_open(tab).await }, cx);
    }

    pub fn picker_did_select(&mut self, tab: RiseStickerPickerTab, cx: &mut Context<Self>) {
        self.with_repository(move |repository| async move { repository.picker_did_select(tab).await }, cx);
    }

    pub fn picker_did_close(&mut self, cx: &mut Context<Self>) {
        self.search_task = None;
        cx.notify();
        self.with_repository(move |repository| async move { repository.picker_did_close().await }, cx);
    }

    pub fn load_default_packs(&mut self, kind: PackKind, relative_id: Option<String>, cx: &mut Context<Self>) {
        Self::actions(cx).update(cx, |actions, cx| actions.get_default_packs_action(kind, relative_id, cx));
    }

    pub fn load_saved_packs(
        &mut self,
        user_id: &str,
        kind: PackKind,
        relative_id: Option<String>,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_saved_packs_action(kind, relative_id, force, cx));
    }

    pub fn load_my_packs(
        &mut self,
        user_id: &str,
        kind: PackKind,
        relative_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_my_packs_action(kind, relative_id, cx));
    }

    pub fn load_pack_stickers(&mut self, pack_id: &str, cx: &mut Context<Self>) {
        if pack_id.is_empty() || (stickers_guide_sandbox_hooks().owns_sticker_pack)(pack_id, cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_stickers_by_pack_id_action(pack_id, cx));
    }

    pub fn load_pack_by_link(
        &mut self,
        link: &str,
        on_resolved: Option<StickerLinkCallback>,
        on_failed: Option<StickerLinkCallback>,
        cx: &mut Context<Self>,
    ) {
        let trimmed = trimming_whitespaces_and_newlines(link);
        if trimmed.is_empty() {
            if let Some(on_failed) = on_failed {
                cx.defer(on_failed);
            }
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_stickers_by_pack_link_action(trimmed, on_resolved, on_failed, cx);
        });
    }

    pub fn load_favourites(&mut self, user_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_user_favourites_action(cx));
    }

    pub fn load_recents(&mut self, user_id: &str, kind: Option<PackKind>, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_user_recent_action(kind, cx));
    }

    pub fn load_gif_favourites(&mut self, user_id: &str, relative_id: Option<String>, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.get_gif_favourites_action(relative_id, cx));
    }

    pub fn search_gifs(&mut self, query: &str, pos: Option<String>, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(query);
        Self::actions(cx).update(cx, |actions, cx| actions.search_gifs_action(trimmed, pos, cx));
    }

    pub fn search_gifs_debounced(
        &mut self,
        query: &str,
        on_fire: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let trimmed = trimming_whitespaces_and_newlines(query).to_owned();
        self.search_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if !trimmed.is_empty() {
                let debounce = RiseTimingCanon::debounce("sticker_gif_search");
                cx.background_executor().timer(Duration::from_nanos(debounce.client_debounce_nanoseconds())).await;
            }
            let fired = trimmed.clone();
            cx.update(|cx| on_fire(fired, cx));
            let _ = this.update(cx, |store, cx| store.search_gifs(&trimmed, None, cx));
        }));
        cx.notify();
    }

    pub fn toggle_favourite(&mut self, user_id: &str, sticker: &StickerItem, cx: &mut Context<Self>) {
        if user_id.is_empty() || sticker.id.is_empty() {
            return;
        }
        let service = Self::service(cx);
        if service.read(cx).pending_favourite_ids().contains(&sticker.id) {
            return;
        }
        let is_favourite = service.read(cx).is_favourite(user_id, &sticker.id);
        let sandbox = stickers_guide_sandbox_hooks();
        if (sandbox.owns_stickers_scene)(cx) {
            (sandbox.toggle_practice_favourite)(sticker, is_favourite, cx);
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            if is_favourite {
                actions.remove_sticker_favourite_action(&sticker.id, cx);
            } else {
                actions.add_sticker_favourite_action(sticker.clone(), cx);
            }
        });
    }

    pub fn toggle_gif_favourite(&mut self, user_id: &str, gif: &GifItem, cx: &mut Context<Self>) {
        if user_id.is_empty() || gif.id.is_empty() {
            return;
        }
        let service = Self::service(cx);
        if service.read(cx).pending_gif_ids().contains(&gif.id) {
            return;
        }
        if let Some(saved) = service.read(cx).gif_favourite(user_id, gif) {
            if service.read(cx).pending_gif_ids().contains(&saved.id) {
                return;
            }
            Self::actions(cx).update(cx, |actions, cx| actions.remove_gif_favourite_action(&saved.id, cx));
        } else {
            Self::actions(cx).update(cx, |actions, cx| actions.add_gif_favourite_action(gif.clone(), cx));
        }
    }

    pub fn add_gif_favourite(&mut self, user_id: &str, gif: &GifItem, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        let service = Self::service(cx);
        if service.read(cx).pending_gif_ids().contains(&gif.id) || service.read(cx).is_gif_favourite(user_id, gif) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.add_gif_favourite_action(gif.clone(), cx));
    }

    pub fn write_recent(&mut self, user_id: &str, sticker_id: &str, kind: Option<PackKind>, cx: &mut Context<Self>) {
        if user_id.is_empty() || sticker_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.write_recent_action(sticker_id, kind, cx));
    }

    pub fn clear_recents(&mut self, user_id: &str, kind: PackKind, cx: &mut Context<Self>) {
        if user_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.clear_recent_action(kind, cx));
    }

    pub fn save_pack(
        &mut self,
        user_id: &str,
        pack_id: &str,
        kind: PackKind,
        pack: Option<StickerPack>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() || pack_id.is_empty() || (stickers_guide_sandbox_hooks().owns_sticker_pack)(pack_id, cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.save_pack_action(pack_id, kind, pack, cx));
    }

    pub fn remove_saved_pack(
        &mut self,
        user_id: &str,
        pack_id: &str,
        kind: PackKind,
        pack: Option<StickerPack>,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() || pack_id.is_empty() || (stickers_guide_sandbox_hooks().owns_sticker_pack)(pack_id, cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.remove_saved_pack_action(pack_id, kind, pack, cx));
    }

    pub fn delete_pack(&mut self, user_id: &str, pack_id: &str, kind: PackKind, cx: &mut Context<Self>) {
        if user_id.is_empty() || pack_id.is_empty() || (stickers_guide_sandbox_hooks().owns_sticker_pack)(pack_id, cx) {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.delete_pack_action(pack_id, kind, cx));
    }

    pub fn rename_pack(&mut self, user_id: &str, pack_id: &str, kind: PackKind, title: &str, cx: &mut Context<Self>) {
        if user_id.is_empty()
            || pack_id.is_empty()
            || trimming_whitespaces_and_newlines(title).is_empty()
            || !RENAME_TITLE_RULE.is_satisfied(title)
            || (stickers_guide_sandbox_hooks().owns_sticker_pack)(pack_id, cx)
        {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.rename_pack_action(
                pack_id,
                kind,
                title,
                |result, cx| {
                    if let Err(error) = result
                        && !error.is::<CancellationError>()
                    {
                        Self::notify_refusal(&error, cx);
                    }
                },
                cx,
            );
        });
    }

    pub fn create_pack(
        &mut self,
        user_id: &str,
        title: &str,
        kind: PackKind,
        request_id: &str,
        on_created: impl FnOnce(StickerPack, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() || request_id.is_empty() {
            return;
        }
        let sandbox = stickers_guide_sandbox_hooks();
        if (sandbox.owns_stickers_scene)(cx) && kind == PackKind::Sticker {
            if let Some(pack) = (sandbox.create_practice_pack)(title, cx) {
                cx.defer(move |cx| on_created(pack, cx));
            }
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.create_pack_action(
                title,
                kind,
                request_id,
                move |result, cx| {
                    if let Ok(pack) = result {
                        on_created(pack, cx);
                    }
                },
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_sticker(
        &mut self,
        user_id: &str,
        pack_id: &str,
        sticker_id: &str,
        draft: Option<StickerMediaUploadDraft>,
        emojis: Vec<String>,
        asset_type: Option<StickerAssetType>,
        codec: Option<String>,
        request_id: &str,
        on_edited: impl FnOnce(StickerItem, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() || pack_id.is_empty() || sticker_id.is_empty() || request_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.edit_sticker_action(
                pack_id,
                sticker_id,
                draft,
                emojis,
                asset_type,
                codec,
                request_id,
                move |result, cx| {
                    if let Ok(sticker) = result {
                        on_edited(sticker, cx);
                    }
                },
                cx,
            );
        });
    }

    pub fn delete_sticker(&mut self, user_id: &str, pack_id: &str, sticker_id: &str, cx: &mut Context<Self>) {
        if user_id.is_empty() || pack_id.is_empty() || sticker_id.is_empty() {
            return;
        }
        let Some(account_epoch) = SaiAccountCacheScope::active_epoch() else {
            return;
        };
        if !SaiAccountCacheScope::is_current_account_epoch(Some(user_id), Some(account_epoch.as_str())) {
            return;
        }
        let user_id = user_id.to_owned();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.delete_sticker_action(
                pack_id,
                sticker_id,
                move |result, cx| {
                    if let Err(error) = result
                        && !error.is::<CancellationError>()
                        && SaiAccountCacheScope::is_current_account_epoch(
                            Some(user_id.as_str()),
                            Some(account_epoch.as_str()),
                        )
                    {
                        Self::notify_refusal(&error, cx);
                    }
                },
                cx,
            );
        });
    }

    pub fn reorder_stickers(&mut self, user_id: &str, pack_id: &str, sticker_ids: Vec<String>, cx: &mut Context<Self>) {
        if user_id.is_empty() || pack_id.is_empty() || sticker_ids.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.reorder_stickers_action(pack_id, sticker_ids, |_, _| {}, cx);
        });
    }

    pub fn reorder_saved_packs(&mut self, user_id: &str, kind: PackKind, pack_ids: Vec<String>, cx: &mut Context<Self>) {
        if user_id.is_empty() || pack_ids.is_empty() {
            return;
        }
        let Some(account_epoch) = SaiAccountCacheScope::active_epoch() else {
            return;
        };
        if !SaiAccountCacheScope::is_current_account_epoch(Some(user_id), Some(account_epoch.as_str())) {
            return;
        }
        let user_id = user_id.to_owned();
        Self::actions(cx).update(cx, |actions, cx| {
            actions.reorder_saved_packs_action(
                kind,
                pack_ids,
                move |result, cx| {
                    if let Err(error) = result
                        && !error.is::<CancellationError>()
                        && SaiAccountCacheScope::is_current_account_epoch(
                            Some(user_id.as_str()),
                            Some(account_epoch.as_str()),
                        )
                    {
                        Self::notify_refusal(&error, cx);
                    }
                },
                cx,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_sticker(
        &mut self,
        user_id: &str,
        pack_id: &str,
        draft: StickerMediaUploadDraft,
        emojis: Vec<String>,
        asset_type: Option<StickerAssetType>,
        codec: Option<String>,
        request_id: &str,
        on_created: impl FnOnce(StickerItem, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if user_id.is_empty() || pack_id.is_empty() || request_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| {
            actions.add_sticker_action(
                pack_id,
                draft,
                emojis,
                asset_type,
                codec,
                request_id,
                move |result, cx| {
                    if let Ok(sticker) = result {
                        on_created(sticker, cx);
                    }
                },
                cx,
            );
        });
    }

    pub fn clear_create_pack_error(&mut self, kind: PackKind, cx: &mut Context<Self>) {
        Self::actions(cx)
            .update(cx, |actions, cx| actions.clear_mutation_error_action(RiseStickerMutationID::CreatePack(kind), cx));
    }

    pub fn clear_sticker_upload_error(&mut self, pack_id: &str, cx: &mut Context<Self>) {
        if pack_id.is_empty() {
            return;
        }
        let mutation = RiseStickerMutationID::AddSticker { pack_id: pack_id.to_owned() };
        Self::actions(cx).update(cx, |actions, cx| actions.clear_mutation_error_action(mutation, cx));
    }

    pub fn clear_sticker_edit_error(&mut self, sticker_id: &str, cx: &mut Context<Self>) {
        if sticker_id.is_empty() {
            return;
        }
        let mutation = RiseStickerMutationID::EditSticker { sticker_id: sticker_id.to_owned() };
        Self::actions(cx).update(cx, |actions, cx| actions.clear_mutation_error_action(mutation, cx));
    }

    pub fn cancel_sticker_upload(&mut self, pack_id: &str, request_id: &str, cx: &mut Context<Self>) {
        if pack_id.is_empty() || request_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.cancel_sticker_upload_action(request_id, pack_id, cx));
    }

    pub fn discard_sticker_upload_attempt(&mut self, request_id: &str, cx: &mut Context<Self>) {
        if request_id.is_empty() {
            return;
        }
        let request_id = request_id.to_owned();
        self.with_repository(
            move |repository| async move { repository.discard_sticker_upload_attempt(&request_id).await },
            cx,
        );
    }

    fn notify_refusal(error: &anyhow::Error, cx: &mut App) {
        Notifier::send(
            CanonErrors::refusal_text(error),
            NotifierSendOptions { r#type: NotifierType::Error, ..NotifierSendOptions::default() },
            cx,
        );
    }

    fn with_repository<Operation, Work>(&mut self, operation: Operation, cx: &mut Context<Self>)
    where
        Operation: FnOnce(Arc<RiseStickerRepository>) -> Work + 'static,
        Work: Future<Output = ()> + Send + 'static,
    {
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = RiseConversationActionExecutor::require_domain(cx).await else {
                return;
            };
            let _ = RiseAccountRuntime::current().spawn(operation(Arc::clone(&domain.stickers))).await;
        })
        .detach();
    }
}
