use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetUserUiHooks, set_account_state_reset_user_ui_hooks,
};
use crate::modules::user::pages::profile::create_post::create_post_media_page::CreatePostMediaHandoff;
use crate::modules::user::pages::profile::settings::moderation_settings::appeal_sheet::AppealSheetStore;

pub fn install_account_state_reset_user_ui_hooks() {
    set_account_state_reset_user_ui_hooks(AccountStateResetUserUiHooks {
        reset_account_state: |cx: &mut App| {
            AppealSheetStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            CreatePostMediaHandoff::shared(cx).update(cx, |handoff, _| handoff.reset_account_state());
        },
    });
}
