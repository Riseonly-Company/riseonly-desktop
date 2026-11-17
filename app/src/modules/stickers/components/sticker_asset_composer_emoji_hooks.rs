use gpui::{AnyElement, App, ElementId, Window};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

pub struct StickerAssetComposerEmojiHooks {
    pub system_emoji_tab: fn(
        ElementId,
        Box<dyn Fn(String, &mut Window, &mut App)>,
        Box<dyn Fn(StickerItem, &mut Window, &mut App)>,
        Box<dyn Fn(&mut Window, &mut App)>,
        bool,
    ) -> AnyElement,
}

pub(crate) static STICKER_ASSET_COMPOSER_EMOJI_HOOKS: FeatureHookSlot<StickerAssetComposerEmojiHooks> =
    FeatureHookSlot::new("STICKER_ASSET_COMPOSER_EMOJI_HOOKS");

pub fn set_sticker_asset_composer_emoji_hooks(hooks: StickerAssetComposerEmojiHooks) {
    let _ = STICKER_ASSET_COMPOSER_EMOJI_HOOKS.set(hooks);
}

pub fn sticker_asset_composer_emoji_hooks() -> Option<&'static StickerAssetComposerEmojiHooks> {
    STICKER_ASSET_COMPOSER_EMOJI_HOOKS.get()
}

/// system emoji tab of the sticker composer, none until the hooks are installed
pub fn sticker_asset_composer_system_emoji_tab(
    id: impl Into<ElementId>,
    on_select_emoji: impl Fn(String, &mut Window, &mut App) + 'static,
    on_select_custom_emoji: impl Fn(StickerItem, &mut Window, &mut App) + 'static,
    on_backspace: impl Fn(&mut Window, &mut App) + 'static,
    is_active: bool,
) -> Option<AnyElement> {
    sticker_asset_composer_emoji_hooks().map(|hooks| {
        (hooks.system_emoji_tab)(
            id.into(),
            Box::new(on_select_emoji),
            Box::new(on_select_custom_emoji),
            Box::new(on_backspace),
            is_active,
        )
    })
}
