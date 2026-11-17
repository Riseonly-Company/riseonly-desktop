use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, Context, Entity, WeakEntity};
use rise_canon::{RiseTimingCanon, RiseValidation};
use rise_core::foundation::foundation_text::trimming_characters;
use rise_core::foundation::foundation_url::{Url, UrlComponents, UrlQueryItem};
use rise_validate::{
    CHARSET_TAG, DEVELOPMENT_HOST_EXACT, DEVELOPMENT_HOST_SUFFIXES, DEVELOPMENT_IPV4_BLOCKS, DEVELOPMENT_IPV6_RULES,
    UrlPolicy,
};

use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_rules::{RiseValidationCanonRules, ValidationSchemaCanonRules};
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::{SaiDebouncer, SaiDebouncerOptions};
use crate::core::rise_engine_bridge::rise_canon_fields::ValidationSchema;
use crate::modules::bot::stores::bot::bot_actions::bot_types::{
    BotModel, BotTagAvailability, BotTagRules, BotTagValidationIssue,
};
use crate::modules::bot::stores::bot::bot_interactions::bot_interactions::BotInteractionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;

/// bot field rules addressed by endpoint plus wire pointer and not by schema name
/// name lookup let a screen pick a neighbour field rule and nothing noticed
pub enum BotFieldRules {}

static NAME: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("bot_create_bot", "/name"));
static TAG: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("bot_create_bot", "/tag"));
static DESCRIPTION: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("bot_create_bot", "/description"));
static COMMAND: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("bot_set_bot_commands", "/commands/*/command"));
static COMMAND_DESCRIPTION: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("bot_set_bot_commands", "/commands/*/description"));
static MINI_APP_SHORT_NAME: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("bot_create_mini_app", "/short_name"));
static MINI_APP_TITLE: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("bot_create_mini_app", "/title"));
static MINI_APP_DESCRIPTION: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("bot_create_mini_app", "/description"));
static WEBHOOK_URL: LazyLock<RiseFieldRule> = LazyLock::new(|| RiseFieldRule::new("bot_set_bot_webhook", "/url"));
static WEBHOOK_SECRET: LazyLock<RiseFieldRule> =
    LazyLock::new(|| RiseFieldRule::new("bot_set_bot_webhook", "/secret_token"));

impl BotFieldRules {
    pub fn name() -> RiseFieldRule {
        *NAME
    }

    pub fn tag() -> RiseFieldRule {
        *TAG
    }

    pub fn description() -> RiseFieldRule {
        *DESCRIPTION
    }

    pub fn command() -> RiseFieldRule {
        *COMMAND
    }

    pub fn command_description() -> RiseFieldRule {
        *COMMAND_DESCRIPTION
    }

    pub fn mini_app_short_name() -> RiseFieldRule {
        *MINI_APP_SHORT_NAME
    }

    pub fn mini_app_title() -> RiseFieldRule {
        *MINI_APP_TITLE
    }

    pub fn mini_app_description() -> RiseFieldRule {
        *MINI_APP_DESCRIPTION
    }

    pub fn webhook_url() -> RiseFieldRule {
        *WEBHOOK_URL
    }

    pub fn webhook_secret() -> RiseFieldRule {
        *WEBHOOK_SECRET
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BotMiniAppRouteSource {
    LaunchUrl(Url),
    Unresolved { tag: String, slug: String, start_param: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BotTagAvailabilityState {
    Idle,
    Invalid(BotTagValidationIssue),
    Checking,
    Available,
    Unavailable { suggestions: Vec<String> },
    Failed,
}

pub enum BotChatManagement {}

impl BotChatManagement {
    pub fn normalized_tag(chat: &ChatInfo) -> String {
        BotTagRules::sanitize(&chat.participant.tag, false, false)
    }

    pub fn can_manage(chat: &ChatInfo, current_user_id: &str, bot: Option<&BotModel>) -> bool {
        if !chat.is_bot_chat() {
            return false;
        }
        if bot.is_some_and(|bot| bot.is_owned_by_requester) {
            return true;
        }
        !current_user_id.is_empty() && chat.participant.bot_owner_id.as_deref() == Some(current_user_id)
    }
}

pub struct BotCreateFlowState {
    pub name: String,
    pub tag: String,
    pub description: String,
    pub can_join_groups: bool,
    pub privacy_mode: bool,
    pub tag_availability: BotTagAvailabilityState,
    pub created_bot: Option<BotModel>,
    pub one_time_token: Option<String>,
    pub token_presented: bool,

    tag_check_generation: i64,
}

impl BotCreateFlowState {
    const DEBOUNCE_ID: &'static str = "botCreateTagAvailability";
    const DEBOUNCE_GROUP: &'static str = "botCreateFlow";
    const TAG_CHECK_TIMEOUT: Duration = Duration::from_secs(8);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            name: String::new(),
            tag: String::new(),
            description: String::new(),
            can_join_groups: true,
            privacy_mode: true,
            tag_availability: BotTagAvailabilityState::Idle,
            created_bot: None,
            one_time_token: None,
            token_presented: false,
            tag_check_generation: 0,
        }
    }

    fn tag_check_delay_ms() -> i64 {
        static DELAY: LazyLock<i64> =
            LazyLock::new(|| RiseTimingCanon::debounce("bot_tag_availability").client_debounce_ms);
        *DELAY
    }

    pub fn first_step_valid(&self) -> bool {
        BotFormValidation::is_valid_bot_name(&self.name)
            && BotFieldRules::description().is_satisfied(&self.description)
            && BotTagRules::is_valid(&self.tag)
            && self.tag_availability == BotTagAvailabilityState::Available
    }

    pub fn reset_form(&mut self, cx: &mut Context<Self>) {
        self.name = String::new();
        self.tag = String::new();
        self.description = String::new();
        self.can_join_groups = true;
        self.privacy_mode = true;
        self.tag_availability = BotTagAvailabilityState::Idle;
        self.tag_check_generation += 1;
        SaiDebouncer::cancel(Self::DEBOUNCE_ID, Self::DEBOUNCE_GROUP, cx);
        cx.notify();
    }

    pub fn set_tag(&mut self, raw: &str, cx: &mut Context<Self>) {
        let sanitized = BotTagRules::sanitize(raw, false, true);
        if self.tag != sanitized {
            self.tag = sanitized;
            cx.notify();
        }
        self.schedule_tag_check(cx);
    }

    pub fn complete_creation(&mut self, bot: BotModel, token: String, cx: &mut Context<Self>) {
        self.created_bot = Some(bot);
        self.one_time_token = Some(token);
        self.reset_form(cx);
    }

    pub fn present_pending_token(&mut self, cx: &mut Context<Self>) {
        if self.created_bot.is_none() || self.one_time_token.is_none() || self.token_presented {
            return;
        }
        self.token_presented = true;
        cx.notify();
    }

    pub fn clear_creation_result(&mut self, cx: &mut Context<Self>) {
        self.created_bot = None;
        self.one_time_token = None;
        self.token_presented = false;
        cx.notify();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| store.clear_sensitive_results(cx));
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.reset_form(cx);
        self.clear_creation_result(cx);
    }

    fn schedule_tag_check(&mut self, cx: &mut Context<Self>) {
        self.tag_check_generation += 1;
        let generation = self.tag_check_generation;
        SaiDebouncer::cancel(Self::DEBOUNCE_ID, Self::DEBOUNCE_GROUP, cx);

        if self.tag.is_empty() {
            self.tag_availability = BotTagAvailabilityState::Idle;
            cx.notify();
            return;
        }
        if let Some(issue) = BotTagRules::validation_issue(&self.tag) {
            self.tag_availability = BotTagAvailabilityState::Invalid(issue);
            cx.notify();
            return;
        }

        self.tag_availability = BotTagAvailabilityState::Checking;
        cx.notify();
        let candidate = self.tag.clone();
        let this = cx.weak_entity();
        SaiDebouncer::debounced_action_id(
            Self::DEBOUNCE_ID,
            Box::new(move |cx: &mut App| Self::run_tag_check(this, generation, candidate, cx)),
            SaiDebouncerOptions {
                delay_milliseconds: Self::tag_check_delay_ms(),
                group_key: Self::DEBOUNCE_GROUP.to_owned(),
                ..SaiDebouncerOptions::default()
            },
            cx,
        );
    }

    fn run_tag_check(this: WeakEntity<Self>, generation: i64, candidate: String, cx: &mut App) {
        let is_current = this
            .read_with(cx, |state, _| generation == state.tag_check_generation && state.tag == candidate)
            .unwrap_or(false);
        if !is_current {
            return;
        }
        let result_state = this.clone();
        BotInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.check_tag_availability(
                &candidate,
                Some(Rc::new(move |checked_tag: String, result: BotTagAvailability, cx: &mut App| {
                    result_state
                        .update(cx, |state, cx| {
                            if generation != state.tag_check_generation || checked_tag != state.tag {
                                return;
                            }
                            state.tag_availability = if result.available {
                                BotTagAvailabilityState::Available
                            } else {
                                BotTagAvailabilityState::Unavailable { suggestions: result.suggestions }
                            };
                            cx.notify();
                        })
                        .ok();
                })),
                cx,
            );
        });
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Self::TAG_CHECK_TIMEOUT).await;
            this.update(cx, |state, cx| {
                if generation != state.tag_check_generation
                    || state.tag != candidate
                    || state.tag_availability != BotTagAvailabilityState::Checking
                {
                    return;
                }
                state.tag_availability = BotTagAvailabilityState::Failed;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

pub enum BotFormValidation {}

impl BotFormValidation {
    pub fn is_valid_bot_name(value: &str) -> bool {
        BotFieldRules::name().is_satisfied(value)
    }

    /// canon only reports invalid, repairing is the client job so foreign chars are dropped
    /// alphabet and cap match is_valid_command below
    pub fn normalized_command(raw: &str) -> String {
        Self::sanitized_tag_like(raw, BotFieldRules::command().schema)
    }

    pub fn is_valid_command(value: &str) -> bool {
        let normalized = Self::normalized_command(value);
        !normalized.is_empty() && normalized == value && BotFieldRules::command().is_satisfied(&normalized)
    }

    pub fn normalized_mini_app_short_name(raw: &str) -> String {
        Self::sanitized_tag_like(raw, BotFieldRules::mini_app_short_name().schema)
    }

    pub fn is_valid_mini_app_short_name(value: &str) -> bool {
        BotFieldRules::mini_app_short_name().is_satisfied(value) && Self::normalized_mini_app_short_name(value) == value
    }

    pub fn is_valid_webhook_url(raw: &str) -> bool {
        if !BotFieldRules::webhook_url().is_satisfied(raw) {
            return false;
        }
        let Some(url) = Url::from_string(raw) else {
            return false;
        };
        BotMiniAppURLPolicy::allows_policy(&url, &RiseValidation::url_policy("bot_webhook"))
    }

    pub fn is_valid_webhook_secret(value: &str) -> bool {
        BotFieldRules::webhook_secret().is_satisfied(value)
    }

    /// schema is absent when canon declares nothing for the field
    /// return the value as is, dont sanitize against an invented alphabet
    fn sanitized_tag_like(raw: &str, schema: Option<ValidationSchema>) -> String {
        let Some(schema) = schema else {
            return raw.to_owned();
        };
        let allowed: String = schema.input(raw).chars().filter(|character| CHARSET_TAG.allows(*character)).collect();
        schema.clamped(&allowed)
    }
}

pub enum BotMiniAppURLPolicy {}

impl BotMiniAppURLPolicy {
    const DEVELOPMENT_POLICY: &'static str = "mini_app_development";
    const PRODUCTION_POLICY: &'static str = "mini_app_production";
    const HTTP_SCHEME: &'static str = "http";

    pub fn allows(url: &Url, allows_development_http: bool) -> bool {
        Self::allows_policy(
            url,
            &RiseValidation::url_policy(if allows_development_http {
                Self::DEVELOPMENT_POLICY
            } else {
                Self::PRODUCTION_POLICY
            }),
        )
    }

    /// url parsing is the platform job, the policy only sees scheme and host and credentials
    /// plain http is allowed only to a development host, a public host over http never passes
    pub fn allows_policy(url: &Url, policy: &UrlPolicy) -> bool {
        if !policy.allow_credentials && (url.user().is_some() || url.password().is_some()) {
            return false;
        }
        if url.absolute_string().len() > policy.max_length_bytes {
            return false;
        }
        let Some(host) = url.host().map(|host| host.to_lowercase()).filter(|host| !host.is_empty()) else {
            return !policy.require_host;
        };

        let scheme = url.scheme().map(str::to_lowercase).unwrap_or_default();
        if policy.secure_schemes.contains(&scheme.as_str()) {
            return policy.allow_development_hosts || !Self::is_development_host(&host);
        }
        if policy.insecure_schemes.contains(&scheme.as_str()) {
            return Self::is_development_host(&host);
        }
        false
    }

    pub fn is_development_host(raw_host: &str) -> bool {
        let lowered = raw_host.to_lowercase();
        let host = trimming_characters(&lowered, |character| matches!(character, '.' | '[' | ']'));
        if DEVELOPMENT_HOST_EXACT.contains(&host) {
            return true;
        }
        if DEVELOPMENT_HOST_SUFFIXES.iter().any(|suffix| host.ends_with(suffix)) {
            return true;
        }
        if host.contains(':') { Self::is_development_ipv6(host) } else { Self::is_development_ipv4(host) }
    }

    fn is_development_ipv6(host: &str) -> bool {
        let groups: Vec<&str> = host.split(':').collect();
        let first = groups.first().and_then(|group| u32::from_str_radix(group, 16).ok()).unwrap_or(0);
        let second = groups.get(1).and_then(|group| u32::from_str_radix(group, 16).ok()).unwrap_or(0);
        DEVELOPMENT_IPV6_RULES.iter().any(|(first_mask, first_value, second_mask, second_value)| {
            (first & u32::from(*first_mask)) == u32::from(*first_value)
                && (second & u32::from(*second_mask)) == u32::from(*second_value)
        })
    }

    fn is_development_ipv4(host: &str) -> bool {
        let labels: Vec<&str> = host.split('.').filter(|label| !label.is_empty()).collect();
        if labels.len() != 4 {
            return false;
        }
        let octets: Vec<u32> = labels.iter().filter_map(|label| label.parse::<u32>().ok()).collect();
        if octets.len() != 4 || !octets.iter().all(|octet| *octet <= 255) {
            return false;
        }

        let address = (octets[0] << 24) | (octets[1] << 16) | (octets[2] << 8) | octets[3];
        DEVELOPMENT_IPV4_BLOCKS.iter().any(|(base, bits)| {
            let mask: u32 = if *bits == 0 { 0 } else { !0u32 << (32 - bits) };
            (address & mask) == *base
        })
    }

    pub fn allows_development_http(url: &Url) -> bool {
        if url.scheme().map(str::to_lowercase).as_deref() != Some(Self::HTTP_SCHEME) {
            return false;
        }
        let Some(host) = url.host() else {
            return false;
        };
        Self::is_development_host(&host)
    }
}

pub enum BotMiniAppInitData {}

impl BotMiniAppInitData {
    const EXACT_NAME: &'static str = "riseonlyInitData";
    const ALIASES: &'static [&'static str] = &["initData", "tgWebAppData"];

    pub fn extract(url: &Url) -> String {
        let fragment_items: Vec<UrlQueryItem> = url
            .fragment()
            .and_then(|fragment| UrlComponents::from_string(&format!("?{fragment}")))
            .and_then(|components| components.query_items())
            .unwrap_or_default();
        if let Some(value) = Self::last_value(&fragment_items, |name| name == Self::EXACT_NAME) {
            return value;
        }
        let query_items: Vec<UrlQueryItem> =
            UrlComponents::from_url(url, false).and_then(|components| components.query_items()).unwrap_or_default();
        if let Some(value) = Self::last_value(&query_items, |name| name == Self::EXACT_NAME) {
            return value;
        }
        if let Some(value) = Self::last_value(&fragment_items, |name| Self::ALIASES.contains(&name)) {
            return value;
        }
        Self::last_value(&query_items, |name| Self::ALIASES.contains(&name)).unwrap_or_default()
    }

    fn last_value(items: &[UrlQueryItem], matches: impl Fn(&str) -> bool) -> Option<String> {
        items.iter().rev().find(|item| matches(&item.name)).and_then(|item| item.value.clone())
    }
}
