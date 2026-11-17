use std::sync::LazyLock;

use rise_canon::{RiseRequestBody, rise_request};

use crate::core::lib::canon_field_rule::RiseFieldRule;

/// profile field rules by wire pointer, checked against user_update_profile
/// social links use a * pointer, one rule per link
pub enum ProfileFieldRules {}

const USER_UPDATE_PROFILE: &str = <rise_request::user::UpdateProfile as RiseRequestBody>::CONTRACT;

static NAME: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(USER_UPDATE_PROFILE, "/name"));
static TAG: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(USER_UPDATE_PROFILE, "/tag"));
static DESCRIPTION: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(USER_UPDATE_PROFILE, "/more/description"));
static WHO: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new(USER_UPDATE_PROFILE, "/more/who"));
static SOCIAL_LINK_TITLE: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(USER_UPDATE_PROFILE, "/more/social_links/*/title"));
static SOCIAL_LINK_URL: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new(USER_UPDATE_PROFILE, "/more/social_links/*/url"));

impl ProfileFieldRules {
    pub fn name() -> RiseFieldRule {
        *NAME
    }

    pub fn tag() -> RiseFieldRule {
        *TAG
    }

    pub fn description() -> RiseFieldRule {
        *DESCRIPTION
    }

    pub fn who() -> RiseFieldRule {
        *WHO
    }

    pub fn social_link_title() -> RiseFieldRule {
        *SOCIAL_LINK_TITLE
    }

    pub fn social_link_url() -> RiseFieldRule {
        *SOCIAL_LINK_URL
    }
}

/// appeal text rule, separate since its another service and endpoint
pub enum ModerationFieldRules {}

static APPEAL_TEXT: LazyLock<RiseFieldRule> = LazyLock::new(|| {
    RiseFieldRule::new(<rise_request::report::CreateAppeal as RiseRequestBody>::CONTRACT, "/text")
});

impl ModerationFieldRules {
    pub fn appeal_text() -> RiseFieldRule {
        *APPEAL_TEXT
    }
}
