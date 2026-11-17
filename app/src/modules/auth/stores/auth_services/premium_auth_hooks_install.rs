use gpui::App;

use crate::core::ui::premium_auth_hooks::PremiumAuthHooks;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

pub(crate) fn install_premium_auth_hooks() {
    PremiumAuthHooks::install(PremiumAuthHooks {
        is_current_user: Box::new(|sender_id: &str, cx: &App| {
            AuthServicesStore::try_read(cx).is_some_and(|store| store.is_current_user(sender_id))
        }),
        profile: Box::new(|cx: &App| AuthServicesStore::try_read(cx).and_then(|store| store.profile().cloned())),
    });
}
