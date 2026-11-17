use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetCommentHooks, set_account_state_reset_comment_hooks,
};
use crate::modules::comment::stores::comment_interactions::comment_interactions::CommentInteractionsStore;
use crate::modules::comment::stores::comment_services::comment_services::CommentServicesStore;

pub(crate) fn install_account_state_reset_comment_hooks() {
    set_account_state_reset_comment_hooks(AccountStateResetCommentHooks {
        reset_account_state: |cx| {
            CommentServicesStore::shared(cx).update(cx, |service, cx| service.reset_account_state(cx));
            CommentInteractionsStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
        },
    });
}
