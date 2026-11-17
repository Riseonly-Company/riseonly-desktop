use futures::FutureExt;
use gpui::App;

use crate::modules::auth::stores::auth_interactions::auth_interactions_moderation_hooks::AuthInteractionsModerationHooks;
use crate::modules::moderation::engine::moderation_domain_hooks::ModerationDomainHooks;
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

fn refresh_loaded_surfaces(cx: &mut App) {
    ModerationInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_loaded_surfaces(cx));
}

pub(crate) fn install_moderation_domain_hooks() {
    ModerationDomainHooks::install(ModerationDomainHooks { refresh_loaded_surfaces: Some(refresh_loaded_surfaces) });
    AuthInteractionsModerationHooks::install(AuthInteractionsModerationHooks {
        refresh_blocks_awaiting_reply: Box::new(|cx: &mut App| {
            ModerationInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.refresh_blocks_awaiting_reply(cx))
                .boxed_local()
        }),
    });
}
