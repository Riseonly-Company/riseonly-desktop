use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::LocalBoxFuture;
use gpui::App;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::core::ui::custom_emoji_stickers_hooks::{CustomEmojiStickersHooks, set_custom_emoji_stickers_hooks};
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::stickers::engine::rise_sticker_presentation::RiseStickerPresentation;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

pub(crate) fn install_custom_emoji_stickers_hooks() {
    set_custom_emoji_stickers_hooks(CustomEmojiStickersHooks {
        resource_revision,
        hydrate_cached_emoji_resources,
        resolve_custom_emoji,
        custom_emoji_item,
    });
}

fn resource_revision(cx: &App) -> u64 {
    rise_ui::shared::try_read::<RiseStickerPresentation>(cx).map_or(0, |presentation| presentation.snapshot().revision)
}

fn hydrate_cached_emoji_resources(account_epoch: String, cx: &mut App) -> LocalBoxFuture<'static, ()> {
    let _ = StickersServicesStore::shared(cx);
    let mut cx = cx.to_async();
    async move {
        let Ok(domain) = RiseConversationActionExecutor::require_domain(&mut cx).await else {
            return;
        };
        if domain.engine.account.epoch != account_epoch {
            return;
        }
        let stickers = Arc::clone(&domain.stickers);
        let _ = RiseAccountRuntime::current().spawn(async move { stickers.hydrate_cached_emoji_resources().await }).await;
    }
    .boxed_local()
}

fn resolve_custom_emoji(ids: Vec<String>, account_epoch: String, cx: &mut App) -> LocalBoxFuture<'static, bool> {
    let _ = StickersServicesStore::shared(cx);
    let mut cx = cx.to_async();
    async move {
        let Ok(domain) = RiseConversationActionExecutor::require_domain(&mut cx).await else {
            return false;
        };
        if domain.engine.account.epoch != account_epoch {
            return false;
        }
        let stickers = Arc::clone(&domain.stickers);
        RiseAccountRuntime::current().spawn(async move { stickers.resolve_custom_emoji(&ids).await }).await.is_ok()
    }
    .boxed_local()
}

fn custom_emoji_item(id: &str, cx: &App) -> Option<StickerItem> {
    rise_ui::shared::try_read::<StickersServicesStore>(cx)?.custom_emoji_item(id, cx)
}
