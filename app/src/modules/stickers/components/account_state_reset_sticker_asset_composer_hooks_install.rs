use gpui::{App, Context};

use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetStickerAssetComposerHooks, set_account_state_reset_sticker_asset_composer_hooks,
};
use crate::modules::stickers::components::sticker_pack_add_cell::StickerAssetComposerStore;

impl AccountResettable for StickerAssetComposerStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        StickerAssetComposerStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_sticker_asset_composer_hooks() {
    set_account_state_reset_sticker_asset_composer_hooks(AccountStateResetStickerAssetComposerHooks {
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&StickerAssetComposerStore::shared(cx), cx);
        },
    });
}
