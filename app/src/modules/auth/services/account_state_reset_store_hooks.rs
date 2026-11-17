use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

macro_rules! account_state_reset_store_slot {
    ($hooks:ident, $slot:ident, $set:ident, $get:ident) => {
        pub(crate) static $slot: FeatureHookSlot<$hooks> = FeatureHookSlot::new(stringify!($slot));

        /// installs the slot as a whole, first installer wins and a second call is ignored
        pub fn $set(hooks: $hooks) {
            let _ = $slot.set(hooks);
        }

        pub fn $get() -> Option<&'static $hooks> {
            $slot.get()
        }
    };
}

pub struct AccountStateResetStickersServicesHooks {
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetShareSheetHooks {
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetStickerPackSheetHooks {
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetStickerAssetComposerHooks {
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetMediaSaveHooks {
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetAchievementEarnedHooks {
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetPremiumUpsellHooks {
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetPremiumGiftCelebrationHooks {
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetPremiumPromoHooks {
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetEmojiPickerRecentsHooks {
    pub activate_account_state: fn(cx: &mut App),
    pub remove_persisted_account: fn(user_id: &str, cx: &mut App),
    pub register: fn(cx: &mut App),
}

pub struct AccountStateResetChatMediaLedgerHooks {
    pub remove_persisted_account: fn(user_id: &str),
}

account_state_reset_store_slot!(
    AccountStateResetStickersServicesHooks,
    STICKERS_SERVICES,
    set_account_state_reset_stickers_services_hooks,
    account_state_reset_stickers_services_hooks
);
account_state_reset_store_slot!(
    AccountStateResetShareSheetHooks,
    SHARE_SHEET,
    set_account_state_reset_share_sheet_hooks,
    account_state_reset_share_sheet_hooks
);
account_state_reset_store_slot!(
    AccountStateResetStickerPackSheetHooks,
    STICKER_PACK_SHEET,
    set_account_state_reset_sticker_pack_sheet_hooks,
    account_state_reset_sticker_pack_sheet_hooks
);
account_state_reset_store_slot!(
    AccountStateResetStickerAssetComposerHooks,
    STICKER_ASSET_COMPOSER,
    set_account_state_reset_sticker_asset_composer_hooks,
    account_state_reset_sticker_asset_composer_hooks
);
account_state_reset_store_slot!(
    AccountStateResetMediaSaveHooks,
    MEDIA_SAVE,
    set_account_state_reset_media_save_hooks,
    account_state_reset_media_save_hooks
);
account_state_reset_store_slot!(
    AccountStateResetAchievementEarnedHooks,
    ACHIEVEMENT_EARNED,
    set_account_state_reset_achievement_earned_hooks,
    account_state_reset_achievement_earned_hooks
);
account_state_reset_store_slot!(
    AccountStateResetPremiumUpsellHooks,
    PREMIUM_UPSELL,
    set_account_state_reset_premium_upsell_hooks,
    account_state_reset_premium_upsell_hooks
);
account_state_reset_store_slot!(
    AccountStateResetPremiumGiftCelebrationHooks,
    PREMIUM_GIFT_CELEBRATION,
    set_account_state_reset_premium_gift_celebration_hooks,
    account_state_reset_premium_gift_celebration_hooks
);
account_state_reset_store_slot!(
    AccountStateResetPremiumPromoHooks,
    PREMIUM_PROMO,
    set_account_state_reset_premium_promo_hooks,
    account_state_reset_premium_promo_hooks
);
account_state_reset_store_slot!(
    AccountStateResetEmojiPickerRecentsHooks,
    EMOJI_PICKER_RECENTS,
    set_account_state_reset_emoji_picker_recents_hooks,
    account_state_reset_emoji_picker_recents_hooks
);
account_state_reset_store_slot!(
    AccountStateResetChatMediaLedgerHooks,
    CHAT_MEDIA_LEDGER,
    set_account_state_reset_chat_media_ledger_hooks,
    account_state_reset_chat_media_ledger_hooks
);
