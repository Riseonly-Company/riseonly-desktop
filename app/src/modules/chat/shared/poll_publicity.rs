use std::sync::LazyLock;

use rise_canon::RiseLimits;
use rise_i18n::tr_named;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatType};

static MAX_MEMBERS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("poll_public_max_members"));

pub enum PollPublicity {}

impl PollPublicity {
    pub fn max_members() -> i64 {
        *MAX_MEMBERS
    }

    /// a private chat or a group up to the canon size may name poll voters, the server keeps the same rule
    pub fn names_voters(chat: &ChatInfo) -> bool {
        chat.r#type != ChatType::Channel && chat.member_count <= Self::max_members()
    }

    pub fn anonymous_only_hint() -> String {
        tr_named("chat_poll_public_unavailable", &[("limit", &Self::max_members().to_string())])
    }
}
