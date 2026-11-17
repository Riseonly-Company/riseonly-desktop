use rise_canon::rise_event::user::AchievementEarned;
use rise_canon::rise_integer;
use rise_canon::rise_proto::user::AchievementDto;
use rise_catalog::{ACHIEVEMENTS_RARITIES_DEFAULT, AchievementRarity};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// how an achievement card draws its artwork,
/// unknown kind decodes as builtin so a new server kind never blanks a list
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RiseAchievementMediaKind {
    #[default]
    Builtin,
    Lottie,
    Image,
}

impl RiseAchievementMediaKind {
    pub const ALL_CASES: [RiseAchievementMediaKind; 3] =
        [RiseAchievementMediaKind::Builtin, RiseAchievementMediaKind::Lottie, RiseAchievementMediaKind::Image];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Lottie => "lottie",
            Self::Image => "image",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|kind| kind.raw_value() == raw)
    }
}

impl Serialize for RiseAchievementMediaKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.raw_value())
    }
}

impl<'de> Deserialize<'de> for RiseAchievementMediaKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_raw_value(&raw).ok_or_else(|| serde::de::Error::unknown_variant(&raw, &["builtin", "lottie", "image"]))
    }
}

/// one achievement as the device keeps it, wire dtos convert here
/// fields have defaults so older rows dont fail the list
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseAchievement {
    pub id: i64,
    pub code_key: String,
    pub media_kind: RiseAchievementMediaKind,
    pub media_ref: String,
    pub accent_color: String,
    pub title: String,
    pub description: String,
    pub how_to_earn: String,
    pub granted_at_ms: i64,
    pub seen_at_ms: i64,
    pub hidden: bool,
    pub owner_only: bool,
    pub granted_count: i64,
    pub rarity: String,
}

impl RiseAchievement {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: i64,
        code_key: String,
        media_kind: RiseAchievementMediaKind,
        media_ref: String,
        accent_color: String,
        title: String,
        description: String,
        how_to_earn: String,
        granted_at_ms: i64,
        seen_at_ms: i64,
        hidden: bool,
        owner_only: bool,
        granted_count: i64,
        rarity: String,
    ) -> Self {
        Self {
            id,
            code_key,
            media_kind,
            media_ref,
            accent_color,
            title,
            description,
            how_to_earn,
            granted_at_ms,
            seen_at_ms,
            hidden,
            owner_only,
            granted_count,
            rarity,
        }
    }

    pub fn from_canon(canon: &AchievementDto) -> Self {
        Self::new(
            canon.id,
            canon.code_key.clone(),
            RiseAchievementMediaKind::from_raw_value(&canon.media_kind).unwrap_or(RiseAchievementMediaKind::Builtin),
            canon.media_ref.clone(),
            canon.accent_color.clone(),
            canon.title.clone(),
            canon.description.clone(),
            canon.how_to_earn.clone(),
            canon.granted_at_ms,
            canon.seen_at_ms,
            canon.hidden,
            canon.owner_only,
            canon.granted_count,
            canon.rarity.clone(),
        )
    }

    pub fn from_event(event: &AchievementEarned) -> Self {
        Self::new(
            event.id,
            event.code_key.clone(),
            RiseAchievementMediaKind::from_raw_value(&event.media_kind).unwrap_or(RiseAchievementMediaKind::Builtin),
            event.media_ref.clone(),
            event.accent_color.clone(),
            event.title.clone(),
            event.description.clone(),
            event.how_to_earn.clone(),
            event.granted_at_ms,
            event.seen_at_ms,
            event.hidden,
            event.owner_only,
            event.granted_count,
            event.rarity.clone(),
        )
    }

    /// seen_at_ms 0 means never seen, proto3 has no scalar presence
    pub fn is_seen(&self) -> bool {
        self.seen_at_ms > 0
    }

    /// the one place the wire string becomes a tier, empty or unknown values fall back to the canon default
    pub fn rarity_tier(&self) -> AchievementRarity {
        AchievementRarity::from_wire(&self.rarity).unwrap_or(ACHIEVEMENTS_RARITIES_DEFAULT)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedRiseAchievement {
    #[serde(default)]
    id: Option<serde_json::Value>,
    #[serde(default)]
    code_key: Option<serde_json::Value>,
    #[serde(default)]
    media_kind: Option<serde_json::Value>,
    #[serde(default)]
    media_ref: Option<serde_json::Value>,
    #[serde(default)]
    accent_color: Option<serde_json::Value>,
    #[serde(default)]
    title: Option<serde_json::Value>,
    #[serde(default)]
    description: Option<serde_json::Value>,
    #[serde(default)]
    how_to_earn: Option<serde_json::Value>,
    #[serde(default)]
    granted_at_ms: Option<serde_json::Value>,
    #[serde(default)]
    seen_at_ms: Option<serde_json::Value>,
    #[serde(default)]
    hidden: Option<serde_json::Value>,
    #[serde(default)]
    owner_only: Option<serde_json::Value>,
    #[serde(default)]
    granted_count: Option<serde_json::Value>,
    #[serde(default)]
    rarity: Option<serde_json::Value>,
}

impl<'de> Deserialize<'de> for RiseAchievement {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let c = CachedRiseAchievement::deserialize(deserializer)?;
        Ok(Self {
            id: rise_achievement_int64(c.id).unwrap_or(0),
            code_key: lenient(c.code_key).unwrap_or_default(),
            media_kind: lenient(c.media_kind).unwrap_or(RiseAchievementMediaKind::Builtin),
            media_ref: lenient(c.media_ref).unwrap_or_default(),
            accent_color: lenient(c.accent_color).unwrap_or_default(),
            title: lenient(c.title).unwrap_or_default(),
            description: lenient(c.description).unwrap_or_default(),
            how_to_earn: lenient(c.how_to_earn).unwrap_or_default(),
            granted_at_ms: rise_achievement_int64(c.granted_at_ms).unwrap_or(0),
            seen_at_ms: rise_achievement_int64(c.seen_at_ms).unwrap_or(0),
            hidden: lenient(c.hidden).unwrap_or(false),
            owner_only: lenient(c.owner_only).unwrap_or(false),
            granted_count: rise_achievement_int64(c.granted_count).unwrap_or(0),
            rarity: lenient(c.rarity).unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum RiseUserAchievementsError {
    #[error("missingActiveSession")]
    MissingActiveSession,
    #[error("staleAccount")]
    StaleAccount,
    #[error("invalidResponse")]
    InvalidResponse,
    #[error("rejected({0})")]
    Rejected(String),
}

fn lenient<T: DeserializeOwned>(field: Option<serde_json::Value>) -> Option<T> {
    serde_json::from_value(field?).ok()
}

/// reads an int64 that may come as a string or a number,
/// so a serializer change on the server cant zero every id
fn rise_achievement_int64(field: Option<serde_json::Value>) -> Option<i64> {
    match field? {
        serde_json::Value::Number(number) => rise_integer(&number),
        serde_json::Value::String(text) => text.parse::<i64>().ok(),
        _ => None,
    }
}
