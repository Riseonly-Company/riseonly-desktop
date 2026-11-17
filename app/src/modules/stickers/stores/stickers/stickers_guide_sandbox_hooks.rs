use gpui::{App, Subscription};
use rise_core::foundation::foundation_url::Url;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::stickers::engine::rise_sticker_models::RiseStickerPresentationSnapshot;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerItem, StickerPack};

pub struct StickersGuideSandboxHooks {
    pub stickers_snapshot: fn(real: RiseStickerPresentationSnapshot, cx: &App) -> RiseStickerPresentationSnapshot,
    pub owns_sticker_pack: fn(id: &str, cx: &App) -> bool,
    pub owns_stickers_scene: fn(cx: &App) -> bool,
    pub toggle_practice_favourite: fn(sticker: &StickerItem, is_favourite: bool, cx: &mut App),
    pub create_practice_pack: fn(title: &str, cx: &mut App) -> Option<StickerPack>,
    pub add_practice_sticker: fn(source: &Url, emojis: &[String], cx: &mut App),
    pub observe_sandbox: fn(on_change: Box<dyn Fn(&mut App)>, cx: &mut App) -> Option<Subscription>,
}

pub(crate) static HOOKS: FeatureHookSlot<StickersGuideSandboxHooks> = FeatureHookSlot::new("HOOKS");

static UNINSTALLED: StickersGuideSandboxHooks = StickersGuideSandboxHooks {
    stickers_snapshot: |real, _| real,
    owns_sticker_pack: |_, _| false,
    owns_stickers_scene: |_| false,
    toggle_practice_favourite: |_, _, _| {},
    create_practice_pack: |_, _| None,
    add_practice_sticker: |_, _, _| {},
    observe_sandbox: |_, _| None,
};

pub fn set_stickers_guide_sandbox_hooks(hooks: StickersGuideSandboxHooks) -> Result<(), StickersGuideSandboxHooks> {
    HOOKS.set(hooks)
}

pub fn stickers_guide_sandbox_hooks() -> &'static StickersGuideSandboxHooks {
    HOOKS.get().unwrap_or(&UNINSTALLED)
}
