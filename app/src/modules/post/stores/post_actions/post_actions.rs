use std::sync::Arc;

use crate::modules::post::engine::rise_post_domain::RisePostDomain;
use crate::modules::post::engine::rise_post_engine_models::PostToggle;
use crate::modules::post::engine::rise_post_repository::PostCommand;
use crate::modules::post::engine::rise_post_rpc::PostScope;

#[derive(Clone)]
pub struct PostActionsStore {
    domain: Arc<RisePostDomain>,
}

impl PostActionsStore {
    pub fn new(domain: Arc<RisePostDomain>) -> Self {
        Self { domain }
    }

    pub fn activate_feed_action(&self, scope: PostScope) {
        self.domain.dispatch(PostCommand::Activate { scope });
    }

    pub fn refresh_feed_action(&self, scope: PostScope) {
        self.domain.dispatch(PostCommand::Refresh { scope });
    }

    pub fn load_more_feed_action(&self, scope: PostScope) {
        self.domain.dispatch(PostCommand::LoadMore { scope });
    }

    pub fn toggle_action(&self, post_id: String, toggle: PostToggle, enabled: bool) {
        self.domain.dispatch(PostCommand::Toggle {
            post_id,
            toggle,
            enabled,
        });
    }

    pub fn set_identity_action(&self, user_id: Option<String>) {
        self.domain.dispatch(PostCommand::SetIdentity { user_id });
    }

    pub fn reset_action(&self) {
        self.domain.dispatch(PostCommand::Reset);
    }
}
