use futures::future::LocalBoxFuture;
use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

pub(crate) struct CustomEmojiStickersHooks {
    pub resource_revision: fn(cx: &App) -> u64,
    pub hydrate_cached_emoji_resources: fn(account_epoch: String, cx: &mut App) -> LocalBoxFuture<'static, ()>,
    pub resolve_custom_emoji:
        fn(ids: Vec<String>, account_epoch: String, cx: &mut App) -> LocalBoxFuture<'static, bool>,
    pub custom_emoji_item: fn(id: &str, cx: &App) -> Option<StickerItem>,
}

pub(crate) static CUSTOM_EMOJI_STICKERS_HOOKS: FeatureHookSlot<CustomEmojiStickersHooks> = FeatureHookSlot::new("CUSTOM_EMOJI_STICKERS_HOOKS");

/// installs the hooks once, a second call keeps the first installer hooks
pub(crate) fn set_custom_emoji_stickers_hooks(hooks: CustomEmojiStickersHooks) {
    let _ = CUSTOM_EMOJI_STICKERS_HOOKS.set(hooks);
}

pub(crate) fn custom_emoji_stickers_hooks() -> Option<&'static CustomEmojiStickersHooks> {
    CUSTOM_EMOJI_STICKERS_HOOKS.get()
}
