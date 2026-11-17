use gpui::{App, Context};

use crate::modules::auth::services::account_state_reset_coordinator::{AccountResettable, AccountStateResetRegistry};
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetStickerPackSheetHooks, set_account_state_reset_sticker_pack_sheet_hooks,
};
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;

impl AccountResettable for StickerPackSheetStore {
    fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        StickerPackSheetStore::reset_account_state(self, cx);
    }
}

pub fn install_account_state_reset_sticker_pack_sheet_hooks() {
    set_account_state_reset_sticker_pack_sheet_hooks(AccountStateResetStickerPackSheetHooks {
        register: |cx: &mut App| {
            AccountStateResetRegistry::register(&StickerPackSheetStore::shared(cx), cx);
        },
    });
}
