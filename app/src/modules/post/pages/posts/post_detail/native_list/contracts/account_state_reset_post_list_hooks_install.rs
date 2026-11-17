use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetPostListHooks, set_account_state_reset_post_list_hooks,
};
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::PostListCachePurge;

pub fn install_account_state_reset_post_list_hooks() {
    set_account_state_reset_post_list_hooks(AccountStateResetPostListHooks {
        purge_all: PostListCachePurge::purge_all,
    });
}
