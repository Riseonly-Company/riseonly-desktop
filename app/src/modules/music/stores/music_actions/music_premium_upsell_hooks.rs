use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct MusicPremiumUpsellHooks {
    pub has_prompt: fn(cx: &mut App) -> bool,
}

pub(crate) static MUSIC_PREMIUM_UPSELL_HOOKS: FeatureHookSlot<MusicPremiumUpsellHooks> =
    FeatureHookSlot::new("MUSIC_PREMIUM_UPSELL_HOOKS");

pub fn set_music_premium_upsell_hooks(hooks: MusicPremiumUpsellHooks) {
    let _ = MUSIC_PREMIUM_UPSELL_HOOKS.set(hooks);
}

pub fn music_premium_upsell_hooks() -> Option<&'static MusicPremiumUpsellHooks> {
    MUSIC_PREMIUM_UPSELL_HOOKS.get()
}
