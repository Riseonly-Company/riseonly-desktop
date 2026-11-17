use crate::modules::auth::stores::auth_actions::author_info_more_hooks::AuthorInfoMore;

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
