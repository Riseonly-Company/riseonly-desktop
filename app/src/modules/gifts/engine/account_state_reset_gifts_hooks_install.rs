use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetGiftsHooks, set_account_state_reset_gifts_hooks,
};
use crate::modules::gifts::engine::rise_gift_appearance_repository::RiseGiftAppearanceRepository;

pub fn install_account_state_reset_gifts_hooks() {
    set_account_state_reset_gifts_hooks(AccountStateResetGiftsHooks {
        remove_persisted_account: |user_id, _cx| {
            RiseGiftAppearanceRepository::remove_persisted_account(user_id);
        },
    });
}
