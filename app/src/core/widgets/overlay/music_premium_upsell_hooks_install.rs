use gpui::App;

use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::music::stores::music_actions::music_premium_upsell_hooks::{
    MusicPremiumUpsellHooks, set_music_premium_upsell_hooks,
};

pub fn install_music_premium_upsell_hooks() {
    set_music_premium_upsell_hooks(MusicPremiumUpsellHooks {
        has_prompt: |cx: &mut App| PremiumUpsellStore::shared(cx).read(cx).prompt().is_some(),
    });
}
