use crate::modules::auth::stores::auth_services::auth_services_guides_hooks::AuthServicesGuidesHooks;
use crate::modules::guides::stores::guides_interactions::guides_interactions::GuidesInteractionsStore;

pub fn install_auth_services_guides_hooks() {
    AuthServicesGuidesHooks::install(AuthServicesGuidesHooks {
        note_account_created: Box::new(|user_id, cx| {
            GuidesInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.note_account_created(user_id, cx));
        }),
    });
}
