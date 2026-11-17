use gpui::App;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;

/// true if the signed in author has premium, false with no profile yet
pub fn post_author_is_premium(cx: &App) -> bool {
    AuthServicesStore::try_read(cx)
        .and_then(|auth| auth.profile())
        .is_some_and(|profile| profile.more.is_premium)
}
