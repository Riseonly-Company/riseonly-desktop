#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchQueryKind {
    FeedDefault,
    Posts,
    Hashtag,
    UsersByTag,
    UsersPlain,
    GlobalUnified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchResultsTab {
    Publications,
    Threads,
    Profiles,
    Channels,
    Groups,
}

impl SearchResultsTab {
    pub const ALL_CASES: [Self; 5] = [
        Self::Publications,
        Self::Threads,
        Self::Profiles,
        Self::Channels,
        Self::Groups,
    ];

    pub const POST: Self = Self::Publications;
    pub const USER: Self = Self::Profiles;

    pub fn raw_value(&self) -> i64 {
        match self {
            Self::Publications => 0,
            Self::Threads => 1,
            Self::Profiles => 2,
            Self::Channels => 3,
            Self::Groups => 4,
        }
    }

    pub fn from_raw(raw_value: i64) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|tab| tab.raw_value() == raw_value)
    }

    pub fn id(&self) -> Self {
        *self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SearchEntityRow {
    pub id: String,
    pub entity_type: String,
    pub display_name: String,
    pub tag_line: String,
    pub logo_url: String,
    pub user_id: Option<String>,
    pub user_tag: Option<String>,
    pub chat_id: Option<String>,
    pub is_premium: bool,
    pub is_official: bool,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
    pub is_bot: bool,
    pub bot_is_active: bool,
    pub member_count: Option<i64>,
    pub is_public: Option<bool>,
}

impl SearchEntityRow {
    pub fn is_channel(&self) -> bool {
        self.entity_type.to_uppercase() == "CHANNEL"
    }

    pub fn is_group(&self) -> bool {
        self.entity_type.to_uppercase() == "GROUP"
    }

    pub fn is_bot_user(&self) -> bool {
        self.is_bot && !self.is_channel() && !self.is_group()
    }
}
