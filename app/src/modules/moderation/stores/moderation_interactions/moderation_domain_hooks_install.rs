use futures::FutureExt;
use gpui::App;

use crate::modules::auth::stores::auth_interactions::auth_interactions_moderation_hooks::{
    AuthAccountBlock, AuthInteractionsModerationHooks,
};
use crate::modules::moderation::engine::moderation_domain_hooks::ModerationDomainHooks;
use crate::modules::moderation::stores::moderation_actions::moderation_types::UserBlockType;
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;

fn refresh_loaded_surfaces(cx: &mut App) {
    ModerationInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_loaded_surfaces(cx));
}

pub fn install_moderation_domain_hooks() {
    ModerationDomainHooks::install(ModerationDomainHooks { refresh_loaded_surfaces: Some(refresh_loaded_surfaces) });
    AuthInteractionsModerationHooks::install(AuthInteractionsModerationHooks {
        refresh_blocks_awaiting_reply: Box::new(|cx: &mut App| {
            let refreshed = ModerationInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.refresh_blocks_awaiting_reply(cx));
            async move {
                refreshed.await.map(|blocks| {
                    blocks
                        .iter()
                        .map(|block| AuthAccountBlock {
                            is_full: block.resolved_type() == UserBlockType::Full,
                            block_end_timestamp: block.block_end_timestamp,
                        })
                        .collect()
                })
            }
            .boxed_local()
        }),
    });
}
