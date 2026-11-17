use std::sync::LazyLock;

use rise_canon::rise_key;
use rise_core::foundation::foundation_text::{character_count, character_prefix, drop_last_characters, trimming_characters};
use rise_validate::CHARSET_TAG;

use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::modules::bot::shared::bot_ui_validation::BotFieldRules;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotModel {
    pub id: String,
    pub owner_user_id: String,
    pub bot_user_id: String,
    pub private_chat_id: String,
    pub tag: String,
    pub name: String,
    pub description: String,
    pub avatar_url: String,
    pub banner_url: String,
    pub is_active: bool,
    pub can_join_groups: bool,
    pub privacy_mode: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub bot_user_chat_id: String,
    pub is_owned_by_requester: bool,
    pub moderation_status: String,
    pub is_system: bool,
}

impl BotModel {
    pub const DEFAULT_MODERATION_STATUS: &'static str = "active";
    const DISABLED_MODERATION_STATUS: &'static str = "disabled";

    pub fn is_moderation_disabled(&self) -> bool {
        self.moderation_status == Self::DISABLED_MODERATION_STATUS
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotCommandModel {
    pub command: String,
    pub description: String,
}

impl BotCommandModel {
    pub fn id(&self) -> &str {
        &self.command
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotPublicCommandsModel {
    pub commands: Vec<BotCommandModel>,
    pub resolved_scope: String,
    pub resolved_language_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotWebhookInfoModel {
    pub bot_id: String,
    pub url: String,
    pub enabled: bool,
    pub has_custom_certificate: bool,
    pub pending_update_count: i64,
    pub last_error_date: i64,
    pub last_error_message: String,
    pub max_connections: i64,
    pub allowed_updates: Vec<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotMiniAppModel {
    pub id: String,
    pub bot_id: String,
    pub short_name: String,
    pub title: String,
    pub description: String,
    pub production_url: String,
    pub development_url: String,
    pub icon_url: String,
    pub is_active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotMiniAppLaunchData {
    pub launch_url: String,
    pub init_data: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotTagAvailability {
    pub available: bool,
    pub reason: String,
    pub suggestions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotTokenResult {
    pub bot_id: String,
    pub token: String,
    pub rotated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotCallbackEnqueueResult {
    pub update_id: i64,
    pub callback_query_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotCallbackAnswerModel {
    pub callback_query_id: String,
    pub text: String,
    pub show_alert: bool,
    pub url: String,
}

impl BotCallbackAnswerModel {
    pub fn id(&self) -> &str {
        &self.callback_query_id
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BotUpdateValues {
    pub name: Option<String>,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub banner_url: Option<String>,
    pub can_join_groups: Option<bool>,
    pub privacy_mode: Option<bool>,
    pub tag: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BotMiniAppUpdateValues {
    pub title: Option<String>,
    pub description: Option<String>,
    pub production_url: Option<String>,
    pub development_url: Option<String>,
    pub icon_url: Option<String>,
    pub is_active: Option<bool>,
}

/// carries the canon i18n key and not a local cause list
/// check order and refusal text are canon and a parallel list drifts when canon adds a check
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BotTagValidationIssue {
    pub message_key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BotMediaKind {
    Avatar,
    Banner,
}

impl BotMediaKind {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Avatar => "avatar",
            Self::Banner => "banner",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        match raw_value {
            "avatar" => Some(Self::Avatar),
            "banner" => Some(Self::Banner),
            _ => None,
        }
    }

    pub fn file_context(self) -> &'static str {
        match self {
            Self::Avatar => "bot_avatar",
            Self::Banner => "bot_banner",
        }
    }

    pub fn wire_key(self) -> &'static str {
        match self {
            Self::Avatar => rise_key::bot::AVATAR_URL,
            Self::Banner => rise_key::bot::BANNER_URL,
        }
    }
}

pub enum BotTagRules {}

static MINIMUM_LENGTH: LazyLock<usize> =
    LazyLock::new(|| BotFieldRules::tag().schema.map(|schema| schema.minimum_value()).unwrap_or(0));
static MAXIMUM_LENGTH: LazyLock<usize> = LazyLock::new(|| BotFieldRules::tag().maximum().unwrap_or(0));
static REQUIRED_SUFFIX: LazyLock<&'static str> =
    LazyLock::new(|| BotFieldRules::tag().schema.map(|schema| schema.required_suffix()).unwrap_or(""));

impl BotTagRules {
    pub const SYSTEM_TAGS: &'static [&'static str] = &["riseonly", "riseonlysupport"];

    pub fn minimum_length() -> usize {
        *MINIMUM_LENGTH
    }

    pub fn maximum_length() -> usize {
        *MAXIMUM_LENGTH
    }

    pub fn required_suffix() -> &'static str {
        *REQUIRED_SUFFIX
    }

    /// schema may be absent, the value then passes through untouched
    fn tag_normalized(raw: &str) -> String {
        match BotFieldRules::tag().schema {
            Some(schema) => schema.normalize(raw),
            None => raw.to_owned(),
        }
    }

    /// repair only, canon says invalid but not how to fix
    /// keep_trailing_underscore is for live typing, a separator typed last must survive
    pub fn sanitize(raw: &str, append_suffix: bool, keep_trailing_underscore: bool) -> String {
        let mut value: String =
            Self::tag_normalized(raw).chars().filter(|character| CHARSET_TAG.allows(*character)).collect();

        while value.contains("__") {
            value = value.replace("__", "_");
        }
        let ended_with_underscore = value.ends_with('_');
        value = trimming_characters(&value, |character| character == '_').to_owned();
        if keep_trailing_underscore && ended_with_underscore && !value.is_empty() && !append_suffix {
            value.push('_');
        }

        let required_suffix = Self::required_suffix();
        let maximum_length = Self::maximum_length();
        if append_suffix && !value.ends_with(required_suffix) {
            value.push_str(required_suffix);
        }
        if character_count(&value) > maximum_length {
            let suffix_length = if value.ends_with(required_suffix) { character_count(required_suffix) } else { 0 };
            let prefix_limit = maximum_length.saturating_sub(suffix_length);
            let prefix = character_prefix(drop_last_characters(&value, suffix_length), prefix_limit).to_owned();
            value = prefix + if suffix_length == 0 { "" } else { required_suffix };
        }
        value
    }

    pub fn validation_issue(tag: &str) -> Option<BotTagValidationIssue> {
        BotFieldRules::tag()
            .schema
            .and_then(|schema| schema.error_key_for_raw_value(tag))
            .map(|message_key| BotTagValidationIssue { message_key: message_key.to_owned() })
    }

    pub fn is_valid(tag: &str) -> bool {
        Self::SYSTEM_TAGS.contains(&Self::tag_normalized(tag).as_str()) || Self::validation_issue(tag).is_none()
    }
}
