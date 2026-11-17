use std::collections::HashSet;

use rise_canon::RiseLimits;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::{characters, drop_first_characters, trimming_whitespaces_and_newlines};
use rise_i18n::AppDateFormatter;
use rise_i18n::date_format::date_formatter::Style;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, AuthorInfoMore};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SubscriptionStatus {
    Active,
    Inactive,
    Expired,
    Pending,
    None,
    Cancelled,
    Refunded,
    GracePeriod,
    BillingRetry,
}

impl SubscriptionStatus {
    pub const ALL: [SubscriptionStatus; 9] = [
        Self::Active,
        Self::Inactive,
        Self::Expired,
        Self::Pending,
        Self::None,
        Self::Cancelled,
        Self::Refunded,
        Self::GracePeriod,
        Self::BillingRetry,
    ];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Expired => "expired",
            Self::Pending => "pending",
            Self::None => "none",
            Self::Cancelled => "cancelled",
            Self::Refunded => "refunded",
            Self::GracePeriod => "grace_period",
            Self::BillingRetry => "billing_retry",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.raw_value() == raw)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProfileSocialLink {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProfileModel {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub alias_tags: Vec<String>,
    pub description: Option<String>,
    pub more: AuthorInfoMore,
    pub posts_count: i64,
    pub subs_count: i64,
    pub subs_count_hidden: bool,
    pub subscribers_count: i64,
    pub friends_count: i64,
    pub subscription_status: SubscriptionStatus,
    pub cover_image_url: Option<String>,
    pub avatar_url: Option<String>,
    pub profile_photos_count: i64,
    pub hb: Option<String>,
    pub gender: Option<String>,
    pub level: Option<i64>,
    pub streak: Option<i64>,
    pub rating: Option<i64>,
    pub is_subbed: bool,
    pub is_subscriber: bool,
    pub is_friend: bool,
    pub friend_request_id: Option<i64>,
    pub social_links: Vec<ProfileSocialLink>,
    pub quick_reaction: String,
    pub is_phone_public: bool,
    pub is_deleted: bool,
    pub is_bot: bool,
    pub bot_owner_id: Option<String>,
    pub bot_is_active: bool,
}

impl Default for ProfileModel {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            tag: String::new(),
            alias_tags: Vec::new(),
            description: None,
            more: AuthorInfoMore::default(),
            posts_count: 0,
            subs_count: 0,
            subs_count_hidden: false,
            subscribers_count: 0,
            friends_count: 0,
            subscription_status: SubscriptionStatus::Inactive,
            cover_image_url: None,
            avatar_url: None,
            profile_photos_count: 0,
            hb: None,
            gender: None,
            level: None,
            streak: None,
            rating: None,
            is_subbed: false,
            is_subscriber: false,
            is_friend: false,
            friend_request_id: None,
            social_links: Vec::new(),
            quick_reaction: Self::DEFAULT_QUICK_REACTION.to_owned(),
            is_phone_public: false,
            is_deleted: false,
            is_bot: false,
            bot_owner_id: None,
            bot_is_active: false,
        }
    }
}

impl ProfileModel {
    pub const DEFAULT_QUICK_REACTION: &'static str = "\u{1F44D}";

    pub fn follow_button_state(&self) -> FollowButtonState {
        if self.is_friend {
            return FollowButtonState::Friends;
        }
        if self.is_subbed {
            return FollowButtonState::Following;
        }
        if self.is_subscriber {
            return FollowButtonState::SubscribeBack;
        }
        FollowButtonState::Subscribe
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FollowButtonState {
    Subscribe,
    Following,
    SubscribeBack,
    Friends,
}

impl FollowButtonState {
    pub fn title_key(&self) -> &'static str {
        match self {
            Self::Subscribe => "profile_subscribe",
            Self::Following => "profile_following",
            Self::SubscribeBack => "profile_subscribe_back",
            Self::Friends => "profile_friends",
        }
    }

    pub fn is_primary(&self) -> bool {
        *self == Self::Subscribe || *self == Self::SubscribeBack
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GoalItem {
    pub id: String,
    pub title: String,
    pub progress: f64,
    pub description: String,
    pub is_checklist: bool,
    pub subgoals: Vec<GoalSubgoalItem>,
}

impl GoalItem {
    pub fn done_subgoal_count(&self) -> usize {
        self.subgoals.iter().filter(|subgoal| subgoal.done).count()
    }

    /// bar a checklist goal shows, done over total and empty with no subgoals
    pub fn checklist_progress(&self) -> f64 {
        if self.subgoals.is_empty() {
            return 0.0;
        }
        self.done_subgoal_count() as f64 / self.subgoals.len() as f64
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GoalSubgoalItem {
    pub id: String,
    pub title: String,
    pub done: bool,
}

impl Eq for GoalItem {}

impl std::hash::Hash for GoalItem {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.title.hash(state);
        self.progress.to_bits().hash(state);
        self.description.hash(state);
        self.is_checklist.hash(state);
        self.subgoals.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlanItem {
    pub id: String,
    pub title: String,
    pub date_ms: i64,
    pub description: String,
}

impl PlanItem {
    pub fn date_display(&self) -> String {
        PlanDateFormatting::display(self.date_ms)
    }
}

pub enum PlanDateFormatting {}

impl PlanDateFormatting {
    pub fn display(date_ms: i64) -> String {
        if date_ms <= 0 {
            return String::new();
        }

        let date = Date::from_time_interval_since_1970(date_ms as f64 / 1000.0);
        let mut f = AppDateFormatter::make(None, None, None);
        f.set_date_style(Style::Medium);
        f.set_time_style(Style::None);

        f.string_from(&date)
    }

    pub fn date(date_ms: i64) -> Option<Date> {
        if date_ms <= 0 {
            return None;
        }

        Some(Date::from_time_interval_since_1970(date_ms as f64 / 1000.0))
    }

    pub fn ms(date: Date) -> i64 {
        (date.time_interval_since_1970() * 1000.0) as i64
    }
}

pub enum UserRelationshipSearchContract {}

impl UserRelationshipSearchContract {
    pub fn minimum_characters() -> i64 {
        RiseLimits::service_value("relationship_search_min_chars")
    }

    pub fn maximum_characters() -> i64 {
        RiseLimits::service_value("relationship_search_max_chars")
    }

    pub fn limited_input(value: &str) -> String {
        let limit = usize::try_from(Self::maximum_characters()).unwrap_or(0);
        value.chars().take(limit).collect()
    }

    pub fn outbound_query(value: Option<&str>) -> Option<String> {
        let value = value?;
        let limited = Self::limited_input(value);
        let trimmed = trimming_whitespaces_and_newlines(&limited);
        if trimmed.is_empty() { None } else { Some(trimmed.to_owned()) }
    }

    pub fn cache_query(value: Option<&str>) -> Option<String> {
        let query = Self::outbound_query(value)?;
        let is_tag_search = characters(&query).next() == Some("@");
        let stripped = if is_tag_search { drop_first_characters(&query, 1) } else { query.as_str() };
        let normalized = trimming_whitespaces_and_newlines(stripped).to_lowercase();
        Some(format!("{}:{}", if is_tag_search { "tag" } else { "name" }, normalized))
    }

    pub fn normalized_term(value: Option<&str>) -> Option<String> {
        let query = Self::outbound_query(value)?;
        let stripped =
            if characters(&query).next() == Some("@") { drop_first_characters(&query, 1) } else { query.as_str() };
        Some(trimming_whitespaces_and_newlines(stripped).to_lowercase())
    }

    pub fn is_short_nonempty_query(value: Option<&str>) -> bool {
        if Self::outbound_query(value).is_none() {
            return false;
        }
        let count = Self::normalized_term(value).map(|term| term.chars().count()).unwrap_or(0);
        i64::try_from(count).unwrap_or(i64::MAX) < Self::minimum_characters()
    }

    pub fn should_dispatch(value: Option<&str>) -> bool {
        !Self::is_short_nonempty_query(value)
    }
}

pub enum UserRelationshipListRenderContract {}

impl UserRelationshipListRenderContract {
    pub fn status(
        has_payload_for_cache_key: bool,
        has_pending_request_for_cache_key: bool,
        has_failure_for_cache_key: bool,
        _slice_status: SaiWsSliceStatus,
    ) -> SaiWsSliceStatus {
        if has_payload_for_cache_key {
            return SaiWsSliceStatus::Fulfilled;
        }
        if has_pending_request_for_cache_key {
            return SaiWsSliceStatus::Pending;
        }
        if has_failure_for_cache_key {
            return SaiWsSliceStatus::Rejected;
        }
        SaiWsSliceStatus::Idle
    }
}

pub enum UserRelationshipFailureCacheContract {}

impl UserRelationshipFailureCacheContract {
    pub fn maximum_entries() -> i64 {
        RiseLimits::client_value("user_relationship_failure_cache_contract_maximum_entries")
    }

    pub fn record(cache_key: &str, cache_keys: &mut HashSet<String>) {
        if !cache_keys.contains(cache_key)
            && i64::try_from(cache_keys.len()).unwrap_or(i64::MAX) >= Self::maximum_entries()
        {
            cache_keys.clear();
        }
        cache_keys.insert(cache_key.to_owned());
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UserRelationshipListCacheScope {
    pub kind: UserListKind,
    pub owner_id: String,
    pub query: Option<String>,
}

impl UserRelationshipListCacheScope {
    pub fn new(kind: UserListKind, owner_id: String, search: Option<&str>) -> Self {
        Self { kind, owner_id, query: UserRelationshipSearchContract::cache_query(search) }
    }
}

pub enum UserRelationshipListCacheRegistryContract {}

impl UserRelationshipListCacheRegistryContract {
    pub fn maximum_entries() -> i64 {
        RiseLimits::client_value("user_relationship_list_cache_registry_contract_maximum_entries")
    }

    pub fn record(
        scope: &UserRelationshipListCacheScope,
        scopes: &mut Vec<UserRelationshipListCacheScope>,
    ) -> Vec<UserRelationshipListCacheScope> {
        scopes.retain(|existing| existing != scope);
        scopes.push(scope.clone());
        let maximum = usize::try_from(Self::maximum_entries()).unwrap_or(0);
        if scopes.len() <= maximum {
            return Vec::new();
        }
        let overflow = scopes.len() - maximum;
        scopes.drain(..overflow).collect()
    }

    pub fn is_affected_by_follow_mutation(
        _scope: &UserRelationshipListCacheScope,
        viewer_id: &str,
        target_user_id: &str,
    ) -> bool {
        !viewer_id.is_empty() && !target_user_id.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UserListContinuation {
    Cursor(String),
    LegacyRelativeId(i64),
}

impl UserListContinuation {
    pub fn pagination_token(&self) -> String {
        match self {
            Self::Cursor(value) => value.clone(),
            Self::LegacyRelativeId(value) => value.to_string(),
        }
    }

    pub fn resolve(cursor: Option<&str>, legacy_relative_id: Option<i64>) -> Option<Self> {
        if let Some(cursor) = cursor.map(trimming_whitespaces_and_newlines)
            && !cursor.is_empty()
        {
            return Some(Self::Cursor(cursor.to_owned()));
        }
        if let Some(legacy_relative_id) = legacy_relative_id {
            return Some(Self::LegacyRelativeId(legacy_relative_id));
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UserListEntry {
    pub entry_key: Option<String>,
    pub entry_id: Option<i64>,
    pub author: AuthorInfo,
    pub owner_is_friend: bool,
    pub viewer_is_subbed: Option<bool>,
    pub viewer_is_subscriber: Option<bool>,
    pub viewer_is_friend: Option<bool>,
    pub viewer_friend_request_id: Option<i64>,
    pub created_at: Option<String>,
}

impl UserListEntry {
    pub fn id(&self) -> String {
        if let Some(entry_key) = self.entry_key.as_deref().map(trimming_whitespaces_and_newlines)
            && !entry_key.is_empty()
        {
            return format!("entry:{entry_key}");
        }
        if let Some(user_id) = self.author.id.as_deref().map(trimming_whitespaces_and_newlines)
            && !user_id.is_empty()
        {
            return format!("user:{user_id}");
        }
        if let Some(entry_id) = self.entry_id {
            return format!("legacy-entry:{entry_id}");
        }

        let tag = trimming_whitespaces_and_newlines(&self.author.tag).to_lowercase();
        let name = trimming_whitespaces_and_newlines(&self.author.name).to_lowercase();
        format!("anonymous:{}|{}|{}", tag, name, self.created_at.as_deref().unwrap_or(""))
    }

    pub fn resolved_relationship(&self, kind: UserListKind, owner_is_viewer: bool) -> UserListRelationship {
        let is_friend = self.viewer_is_friend.unwrap_or(owner_is_viewer && self.owner_is_friend);
        let is_subbed =
            self.viewer_is_subbed.unwrap_or(is_friend || (owner_is_viewer && kind == UserListKind::Subscriptions));
        let is_subscriber =
            self.viewer_is_subscriber.unwrap_or(is_friend || (owner_is_viewer && kind == UserListKind::Subscribers));

        UserListRelationship { is_subbed, is_subscriber, is_friend }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UserListRelationship {
    pub is_subbed: bool,
    pub is_subscriber: bool,
    pub is_friend: bool,
}

impl UserListRelationship {
    pub fn follow_button_state(&self) -> FollowButtonState {
        if self.is_friend {
            return FollowButtonState::Friends;
        }
        if self.is_subbed {
            return FollowButtonState::Following;
        }
        if self.is_subscriber {
            return FollowButtonState::SubscribeBack;
        }
        FollowButtonState::Subscribe
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrivacyRulePatch {
    pub plan_rule: Option<String>,
    pub goal_rule: Option<String>,
    pub friend_rule: Option<String>,
    pub hb_rule: Option<String>,
    pub phone_rule: Option<String>,
    pub description_rule: Option<String>,
    pub last_seen_rule: Option<String>,
    pub online_status_rule: Option<String>,
    pub profile_photo_rule: Option<String>,
    pub calls_rule: Option<String>,
    pub forwards_rule: Option<String>,
    pub groups_rule: Option<String>,
    pub subs_rule: Option<String>,
    pub comments_rule: Option<String>,
    pub reposts_rule: Option<String>,
    pub achievements_rule: Option<String>,
    pub stories_rule: Option<String>,
    pub messages_rule: Option<String>,
    pub coauthor_rule: Option<String>,
    pub tag_rule: Option<String>,
    pub sparks_rule: Option<String>,
    pub gifts_rule: Option<String>,
    pub gift_send_rule: Option<String>,
    pub subs_count_hidden: Option<bool>,
    pub message_requests_enabled: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrivacyExceptionAction {
    Allow,
    Deny,
}

impl PrivacyExceptionAction {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        [Self::Allow, Self::Deny].into_iter().find(|action| action.raw_value() == raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UserListKind {
    Friends,
    Subscribers,
    Subscriptions,
}

impl UserListKind {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Friends => "friends",
            Self::Subscribers => "subscribers",
            Self::Subscriptions => "subscriptions",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        [Self::Friends, Self::Subscribers, Self::Subscriptions].into_iter().find(|kind| kind.raw_value() == raw)
    }
}
