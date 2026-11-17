use futures::FutureExt;
use futures::future::LocalBoxFuture;
use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

/// what the blocked account page needs to know about one block of the account
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthAccountBlock {
    pub is_full: bool,
    pub block_end_timestamp: i64,
}

pub struct AuthInteractionsModerationHooks {
    pub refresh_blocks_awaiting_reply:
        Box<dyn Fn(&mut App) -> LocalBoxFuture<'static, Option<Vec<AuthAccountBlock>>> + Send + Sync>,
}

pub(crate) static AUTH_INTERACTIONS_MODERATION_HOOKS: FeatureHookSlot<AuthInteractionsModerationHooks> = FeatureHookSlot::new("AUTH_INTERACTIONS_MODERATION_HOOKS");

impl AuthInteractionsModerationHooks {
    pub fn install(hooks: Self) {
        let _ = AUTH_INTERACTIONS_MODERATION_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        AUTH_INTERACTIONS_MODERATION_HOOKS.get()
    }

    pub fn refresh_blocks_awaiting_reply(cx: &mut App) -> LocalBoxFuture<'static, Option<Vec<AuthAccountBlock>>> {
        match Self::installed() {
            Some(hooks) => (hooks.refresh_blocks_awaiting_reply)(cx),
            None => futures::future::ready(None).boxed_local(),
        }
    }
}
