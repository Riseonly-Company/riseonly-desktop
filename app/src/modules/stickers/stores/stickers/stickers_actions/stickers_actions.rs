use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::stickers::engine::rise_sticker_models::{RiseStickerMutationID, StickerMediaUploadDraft};
use crate::modules::stickers::engine::rise_sticker_repository::RiseStickerRepository;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    GifItem, PackKind, StickerAssetType, StickerItem, StickerPack,
};

pub type StickerLinkCallback = Box<dyn FnOnce(&mut App)>;

struct StickerUploadTaskEntry {
    token: Uuid,
    _task: Task<()>,
}

struct AccountRuntimeAbortOnDrop(tokio::task::AbortHandle);

impl Drop for AccountRuntimeAbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct StickersActionsStore {
    sticker_upload_tasks: HashMap<String, StickerUploadTaskEntry>,
}

impl StickersActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { sticker_upload_tasks: HashMap::new() }
    }

    pub fn get_default_packs_action(&mut self, kind: PackKind, relative_id: Option<String>, cx: &mut Context<Self>) {
        self.run(move |repository| async move { repository.load_default_packs(kind, relative_id, false).await }, cx);
    }

    pub fn get_saved_packs_action(
        &mut self,
        kind: PackKind,
        relative_id: Option<String>,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        self.run(move |repository| async move { repository.load_saved_packs(kind, relative_id, force).await }, cx);
    }

    pub fn get_my_packs_action(&mut self, kind: PackKind, relative_id: Option<String>, cx: &mut Context<Self>) {
        self.run(move |repository| async move { repository.load_my_packs(kind, relative_id, false).await }, cx);
    }

    pub fn get_stickers_by_pack_id_action(&mut self, pack_id: &str, cx: &mut Context<Self>) {
        let pack_id = pack_id.to_owned();
        self.run(move |repository| async move { repository.load_pack(&pack_id, false).await }, cx);
    }

    pub fn get_stickers_by_pack_link_action(
        &mut self,
        link: &str,
        on_resolved: Option<StickerLinkCallback>,
        on_failed: Option<StickerLinkCallback>,
        cx: &mut Context<Self>,
    ) {
        let link = link.to_owned();
        self.run_result(
            move |repository| async move { Ok(repository.load_pack_link(&link, true).await) },
            move |result: anyhow::Result<bool>, cx| {
                if matches!(result, Ok(true)) {
                    if let Some(on_resolved) = on_resolved {
                        on_resolved(cx);
                    }
                } else if let Some(on_failed) = on_failed {
                    on_failed(cx);
                }
            },
            cx,
        );
    }

    pub fn get_user_favourites_action(&mut self, cx: &mut Context<Self>) {
        self.run(move |repository| async move { repository.load_favourites(false).await }, cx);
    }

    pub fn add_sticker_favourite_action(&mut self, sticker: StickerItem, cx: &mut Context<Self>) {
        self.run(move |repository| async move { repository.add_favourite(sticker).await }, cx);
    }

    pub fn remove_sticker_favourite_action(&mut self, sticker_id: &str, cx: &mut Context<Self>) {
        let sticker_id = sticker_id.to_owned();
        self.run(move |repository| async move { repository.remove_favourite(&sticker_id).await }, cx);
    }

    pub fn get_user_recent_action(&mut self, kind: Option<PackKind>, cx: &mut Context<Self>) {
        let kind = kind.unwrap_or(PackKind::Sticker);
        self.run(move |repository| async move { repository.load_recents(kind, false).await }, cx);
    }

    pub fn write_recent_action(&mut self, sticker_id: &str, kind: Option<PackKind>, cx: &mut Context<Self>) {
        let sticker_id = sticker_id.to_owned();
        let kind = kind.unwrap_or(PackKind::Sticker);
        self.run(move |repository| async move { repository.write_recent(&sticker_id, kind).await }, cx);
    }

    pub fn clear_recent_action(&mut self, kind: PackKind, cx: &mut Context<Self>) {
        self.run(move |repository| async move { repository.clear_recents(kind).await }, cx);
    }

    pub fn get_gif_favourites_action(&mut self, relative_id: Option<String>, cx: &mut Context<Self>) {
        self.run(move |repository| async move { repository.load_gif_favourites(relative_id, false).await }, cx);
    }

    pub fn add_gif_favourite_action(&mut self, gif: GifItem, cx: &mut Context<Self>) {
        self.run(move |repository| async move { repository.add_gif_favourite(gif).await }, cx);
    }

    pub fn remove_gif_favourite_action(&mut self, gif_id: &str, cx: &mut Context<Self>) {
        let gif_id = gif_id.to_owned();
        self.run(move |repository| async move { repository.remove_gif_favourite(&gif_id).await }, cx);
    }

    pub fn search_gifs_action(&mut self, query: &str, pos: Option<String>, cx: &mut Context<Self>) {
        let query = query.to_owned();
        self.run(move |repository| async move { repository.search_gifs(&query, pos, false).await }, cx);
    }

    pub fn save_pack_action(&mut self, pack_id: &str, kind: PackKind, pack: Option<StickerPack>, cx: &mut Context<Self>) {
        let pack_id = pack_id.to_owned();
        self.run(move |repository| async move { repository.set_pack_saved(&pack_id, kind, pack, true).await }, cx);
    }

    pub fn remove_saved_pack_action(
        &mut self,
        pack_id: &str,
        kind: PackKind,
        pack: Option<StickerPack>,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        self.run(move |repository| async move { repository.set_pack_saved(&pack_id, kind, pack, false).await }, cx);
    }

    pub fn delete_pack_action(&mut self, pack_id: &str, kind: PackKind, cx: &mut Context<Self>) {
        let pack_id = pack_id.to_owned();
        self.run(move |repository| async move { repository.delete_pack(&pack_id, kind).await }, cx);
    }

    pub fn rename_pack_action(
        &mut self,
        pack_id: &str,
        kind: PackKind,
        title: &str,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        let title = title.to_owned();
        self.run_result(
            move |repository| async move { repository.rename_pack(&pack_id, kind, &title).await },
            on_result,
            cx,
        );
    }

    pub fn create_pack_action(
        &mut self,
        title: &str,
        kind: PackKind,
        request_id: &str,
        on_result: impl FnOnce(anyhow::Result<StickerPack>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let title = title.to_owned();
        let request_id = request_id.to_owned();
        self.run_result(
            move |repository| async move { repository.create_pack(&title, kind, &request_id).await },
            on_result,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_sticker_action(
        &mut self,
        pack_id: &str,
        draft: StickerMediaUploadDraft,
        emojis: Vec<String>,
        asset_type: Option<StickerAssetType>,
        codec: Option<String>,
        request_id: &str,
        on_result: impl FnOnce(anyhow::Result<StickerItem>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        let operation_request_id = request_id.to_owned();
        self.run_sticker_upload(
            request_id,
            "RiseEngine request failed",
            move |repository| async move {
                repository.add_sticker(&pack_id, draft, emojis, asset_type, codec, &operation_request_id).await
            },
            on_result,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn edit_sticker_action(
        &mut self,
        pack_id: &str,
        sticker_id: &str,
        draft: Option<StickerMediaUploadDraft>,
        emojis: Vec<String>,
        asset_type: Option<StickerAssetType>,
        codec: Option<String>,
        request_id: &str,
        on_result: impl FnOnce(anyhow::Result<StickerItem>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        let sticker_id = sticker_id.to_owned();
        let operation_request_id = request_id.to_owned();
        self.run_sticker_upload(
            request_id,
            "Sticker edit failed",
            move |repository| async move {
                repository
                    .edit_sticker(&pack_id, &sticker_id, draft, emojis, asset_type, codec, &operation_request_id)
                    .await
            },
            on_result,
            cx,
        );
    }

    pub fn delete_sticker_action(
        &mut self,
        pack_id: &str,
        sticker_id: &str,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        let sticker_id = sticker_id.to_owned();
        self.run_result(
            move |repository| async move { repository.delete_sticker(&pack_id, &sticker_id).await },
            on_result,
            cx,
        );
    }

    pub fn reorder_stickers_action(
        &mut self,
        pack_id: &str,
        sticker_ids: Vec<String>,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let pack_id = pack_id.to_owned();
        self.run_result(
            move |repository| async move { repository.reorder_stickers(&pack_id, &sticker_ids).await },
            on_result,
            cx,
        );
    }

    pub fn reorder_saved_packs_action(
        &mut self,
        kind: PackKind,
        pack_ids: Vec<String>,
        on_result: impl FnOnce(anyhow::Result<()>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.run_result(
            move |repository| async move { repository.reorder_saved_packs(kind, &pack_ids).await },
            on_result,
            cx,
        );
    }

    pub fn cancel_sticker_upload_action(&mut self, request_id: &str, pack_id: &str, cx: &mut Context<Self>) {
        self.sticker_upload_tasks.remove(request_id);
        cx.notify();
        let request_id = request_id.to_owned();
        let pack_id = pack_id.to_owned();
        self.run(move |repository| async move { repository.cancel_sticker_upload(&request_id, &pack_id).await }, cx);
    }

    pub fn cancel_all_sticker_uploads_action(&mut self, cx: &mut Context<Self>) {
        self.sticker_upload_tasks.clear();
        cx.notify();
    }

    pub fn clear_mutation_error_action(&mut self, mutation: RiseStickerMutationID, cx: &mut Context<Self>) {
        self.run(move |repository| async move { repository.clear_mutation_error(&mutation).await }, cx);
    }

    fn run_sticker_upload<Operation, Work>(
        &mut self,
        request_id: &str,
        failure_context: &'static str,
        operation: Operation,
        on_result: impl FnOnce(anyhow::Result<StickerItem>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) where
        Operation: FnOnce(Arc<RiseStickerRepository>) -> Work + 'static,
        Work: Future<Output = anyhow::Result<StickerItem>> + Send + 'static,
    {
        self.sticker_upload_tasks.remove(request_id);
        let token = Uuid::new_v4();
        let entry_request_id = request_id.to_owned();
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = match RiseConversationActionExecutor::require_domain(cx).await {
                Ok(domain) => Self::on_account_runtime(operation(Arc::clone(&domain.stickers))).await,
                Err(error) => Err(error),
            };
            match result {
                Ok(sticker) => cx.update(|cx| on_result(Ok(sticker), cx)),
                Err(error) if error.is::<CancellationError>() => {}
                Err(error) => {
                    Self::log_failure(failure_context, &error, cx);
                    cx.update(|cx| on_result(Err(error), cx));
                }
            }
            let _ = this.update(cx, |store, cx| {
                if store.sticker_upload_tasks.get(&entry_request_id).is_some_and(|entry| entry.token == token) {
                    store.sticker_upload_tasks.remove(&entry_request_id);
                    cx.notify();
                }
            });
        });
        self.sticker_upload_tasks.insert(request_id.to_owned(), StickerUploadTaskEntry { token, _task: task });
        cx.notify();
    }

    fn run<Operation, Work>(&mut self, operation: Operation, cx: &mut Context<Self>)
    where
        Operation: FnOnce(Arc<RiseStickerRepository>) -> Work + 'static,
        Work: Future<Output = ()> + Send + 'static,
    {
        cx.spawn(async move |_, cx: &mut AsyncApp| match RiseConversationActionExecutor::require_domain(cx).await {
            Ok(domain) => {
                let _ = RiseAccountRuntime::current().spawn(operation(Arc::clone(&domain.stickers))).await;
            }
            Err(error) => Self::log_failure("RiseEngine request failed", &error, cx),
        })
        .detach();
    }

    fn run_result<Value, Operation, Work>(
        &mut self,
        operation: Operation,
        on_result: impl FnOnce(anyhow::Result<Value>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) where
        Value: Send + 'static,
        Operation: FnOnce(Arc<RiseStickerRepository>) -> Work + 'static,
        Work: Future<Output = anyhow::Result<Value>> + Send + 'static,
    {
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match RiseConversationActionExecutor::require_domain(cx).await {
                Ok(domain) => Self::on_account_runtime(operation(Arc::clone(&domain.stickers))).await,
                Err(error) => Err(error),
            };
            if let Err(error) = &result {
                Self::log_failure("RiseEngine request failed", error, cx);
            }
            cx.update(|cx| on_result(result, cx));
        })
        .detach();
    }

    async fn on_account_runtime<Value, Work>(work: Work) -> anyhow::Result<Value>
    where
        Value: Send + 'static,
        Work: Future<Output = anyhow::Result<Value>> + Send + 'static,
    {
        let task = RiseAccountRuntime::current().spawn(work);
        let _abort = AccountRuntimeAbortOnDrop(task.abort_handle());
        match task.await {
            Ok(result) => result,
            Err(_) => Err(CancellationError.into()),
        }
    }

    fn log_failure(context: &'static str, error: &anyhow::Error, cx: &mut AsyncApp) {
        let text = format!("{context}: {error}");
        cx.update(|cx| Sai::log("Stickers", move || text, SaiLogKind::Network, cx));
    }
}
