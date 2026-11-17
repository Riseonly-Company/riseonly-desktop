use crate::core::storage::rise_chat_media_ledger::RiseChatMediaLedger;
use crate::modules::auth::services::account_state_reset_store_hooks::{
    AccountStateResetChatMediaLedgerHooks, set_account_state_reset_chat_media_ledger_hooks,
};

pub fn install_account_state_reset_chat_media_ledger_hooks() {
    set_account_state_reset_chat_media_ledger_hooks(AccountStateResetChatMediaLedgerHooks {
        remove_persisted_account: RiseChatMediaLedger::remove_persisted_account,
    });
}
