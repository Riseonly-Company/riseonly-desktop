use gpui::{App, Subscription};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerItem, StickerPack};

pub struct EmojiPickerStickersHooks {
    pub has_resolved_pack: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub is_loading: Box<dyn Fn(&App) -> bool + Send + Sync>,
    pub pack_stickers: Box<dyn Fn(&str, &App) -> Vec<StickerItem> + Send + Sync>,
    pub recents: Box<dyn Fn(&str, &App) -> Vec<StickerItem> + Send + Sync>,
    pub saved_pack_summaries: Box<dyn Fn(&str, &App) -> Vec<StickerPack> + Send + Sync>,
    pub saved_packs_has_more: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub saved_packs_next_relative_id: Box<dyn Fn(&str, &App) -> Option<String> + Send + Sync>,
    pub saved_packs_resolved: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub observe: Box<dyn Fn(Box<dyn Fn(&mut App)>, &mut App) -> Subscription + Send + Sync>,
    pub load_recents: Box<dyn Fn(&str, &mut App) + Send + Sync>,
    pub load_saved_packs: Box<dyn Fn(&str, Option<&str>, bool, &mut App) + Send + Sync>,
    pub load_pack_stickers: Box<dyn Fn(&str, &mut App) + Send + Sync>,
    pub write_recent: Box<dyn Fn(&str, &str, &mut App) + Send + Sync>,
    pub clear_recents: Box<dyn Fn(&str, &mut App) + Send + Sync>,
}

pub(crate) static EMOJI_PICKER_STICKERS_HOOKS: FeatureHookSlot<EmojiPickerStickersHooks> = FeatureHookSlot::new("EMOJI_PICKER_STICKERS_HOOKS");

impl EmojiPickerStickersHooks {
    pub fn install(hooks: EmojiPickerStickersHooks) {
        let _ = EMOJI_PICKER_STICKERS_HOOKS.set(hooks);
    }

    pub fn shared() -> Option<&'static EmojiPickerStickersHooks> {
        EMOJI_PICKER_STICKERS_HOOKS.get()
    }
}
