use rise_canon::rise_proto::user::LocaleSettings;
use rise_i18n::AppLanguageCatalog;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// where a stored locale value came from. part of the storage contract so dont renumber
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RiseLocaleSource {
    #[default]
    Unset,
    Device,
    Phone,
    DetectedIp,
    Explicit,
}

impl RiseLocaleSource {
    pub const ALL_CASES: [RiseLocaleSource; 5] = [
        RiseLocaleSource::Unset,
        RiseLocaleSource::Device,
        RiseLocaleSource::Phone,
        RiseLocaleSource::DetectedIp,
        RiseLocaleSource::Explicit,
    ];

    pub fn raw_value(self) -> i64 {
        match self {
            Self::Unset => 0,
            Self::Device => 1,
            Self::Phone => 2,
            Self::DetectedIp => 3,
            Self::Explicit => 4,
        }
    }

    pub fn from_raw_value(raw: i64) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|source| source.raw_value() == raw)
    }
}

impl Serialize for RiseLocaleSource {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.raw_value())
    }
}

impl<'de> Deserialize<'de> for RiseLocaleSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = i64::deserialize(deserializer)?;
        Self::from_raw_value(raw)
            .ok_or_else(|| serde::de::Error::invalid_value(serde::de::Unexpected::Signed(raw), &"a RiseLocaleSource"))
    }
}

/// region and language prefs as the server resolves them and the device stores them
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseUserLocaleSettings {
    pub region_code: String,
    pub region_source: RiseLocaleSource,
    pub app_language: String,
    pub app_language_source: RiseLocaleSource,
    pub notification_language: String,
    pub notification_language_follows_app: bool,
    pub timezone: String,
    pub detected_region: String,
    pub signup_region: String,
    pub signup_language: String,
    pub revision: i64,
}

impl Default for RiseUserLocaleSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseUserLocaleSettings {
    pub fn new() -> Self {
        Self {
            region_code: String::new(),
            region_source: RiseLocaleSource::Unset,
            app_language: AppLanguageCatalog::BASE_LANGUAGE.to_owned(),
            app_language_source: RiseLocaleSource::Unset,
            notification_language: AppLanguageCatalog::BASE_LANGUAGE.to_owned(),
            notification_language_follows_app: true,
            timezone: String::new(),
            detected_region: String::new(),
            signup_region: String::new(),
            signup_language: String::new(),
            revision: 0,
        }
    }

    pub fn empty() -> Self {
        Self::new()
    }

    pub fn from_canon(canon: &LocaleSettings) -> Self {
        let app_language = if canon.app_language.is_empty() {
            AppLanguageCatalog::BASE_LANGUAGE.to_owned()
        } else {
            canon.app_language.clone()
        };
        let notification_language = if canon.notification_language.is_empty() {
            app_language.clone()
        } else {
            canon.notification_language.clone()
        };
        Self {
            region_code: canon.region_code.clone(),
            region_source: RiseLocaleSource::from_raw_value(canon.region_source).unwrap_or(RiseLocaleSource::Unset),
            app_language,
            app_language_source: RiseLocaleSource::from_raw_value(canon.app_language_source)
                .unwrap_or(RiseLocaleSource::Unset),
            notification_language,
            notification_language_follows_app: canon.notification_language_follows_app,
            timezone: canon.timezone.clone(),
            detected_region: canon.detected_region.clone(),
            signup_region: canon.signup_region.clone(),
            signup_language: canon.signup_language.clone(),
            revision: canon.revision,
        }
    }

    pub fn detection_disagrees_with_choice(&self) -> bool {
        self.region_source == RiseLocaleSource::Explicit
            && !self.detected_region.is_empty()
            && !self.region_code.is_empty()
            && self.detected_region != self.region_code
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRiseUserLocaleSettings {
    #[serde(default)]
    region_code: Option<serde_json::Value>,
    #[serde(default)]
    region_source: Option<serde_json::Value>,
    #[serde(default)]
    app_language: Option<serde_json::Value>,
    #[serde(default)]
    app_language_source: Option<serde_json::Value>,
    #[serde(default)]
    notification_language: Option<serde_json::Value>,
    #[serde(default)]
    notification_language_follows_app: Option<serde_json::Value>,
    #[serde(default)]
    timezone: Option<serde_json::Value>,
    #[serde(default)]
    detected_region: Option<serde_json::Value>,
    #[serde(default)]
    signup_region: Option<serde_json::Value>,
    #[serde(default)]
    signup_language: Option<serde_json::Value>,
    #[serde(default)]
    revision: Option<serde_json::Value>,
}

impl<'de> Deserialize<'de> for RiseUserLocaleSettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let c = StoredRiseUserLocaleSettings::deserialize(deserializer)?;
        let app_language: String =
            lenient(c.app_language).unwrap_or_else(|| AppLanguageCatalog::BASE_LANGUAGE.to_owned());
        Ok(Self {
            region_code: lenient(c.region_code).unwrap_or_default(),
            region_source: lenient(c.region_source).unwrap_or(RiseLocaleSource::Unset),
            notification_language: lenient(c.notification_language).unwrap_or_else(|| app_language.clone()),
            app_language,
            app_language_source: lenient(c.app_language_source).unwrap_or(RiseLocaleSource::Unset),
            notification_language_follows_app: lenient(c.notification_language_follows_app).unwrap_or(true),
            timezone: lenient(c.timezone).unwrap_or_default(),
            detected_region: lenient(c.detected_region).unwrap_or_default(),
            signup_region: lenient(c.signup_region).unwrap_or_default(),
            signup_language: lenient(c.signup_language).unwrap_or_default(),
            revision: lenient(c.revision).unwrap_or(0),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum RiseUserLocaleError {
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
