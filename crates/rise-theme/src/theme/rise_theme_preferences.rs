use rise_canon::rise_request::user::update_theme_settings::Platform as CanonPlatform;
use rise_canon::rise_theme_tokens::Appearance as CanonAppearance;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiseThemePlatform {
    Mobile,
    Desktop,
    Web,
}

impl RiseThemePlatform {
    pub const ALL_CASES: [RiseThemePlatform; 3] =
        [RiseThemePlatform::Mobile, RiseThemePlatform::Desktop, RiseThemePlatform::Web];

    pub const THIS_DEVICE: RiseThemePlatform = RiseThemePlatform::Desktop;

    pub fn raw_value(self) -> &'static str {
        self.canon().raw_value()
    }

    fn canon(self) -> CanonPlatform {
        match self {
            RiseThemePlatform::Mobile => CanonPlatform::Mobile,
            RiseThemePlatform::Desktop => CanonPlatform::Desktop,
            RiseThemePlatform::Web => CanonPlatform::Web,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseThemeAppearance {
    Light,
    Dark,
}

impl RiseThemeAppearance {
    pub const ALL_CASES: [RiseThemeAppearance; 2] = [RiseThemeAppearance::Light, RiseThemeAppearance::Dark];

    pub fn raw_value(self) -> &'static str {
        self.canon().wire()
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        CanonAppearance::from_wire(raw).map(|appearance| match appearance {
            CanonAppearance::Light => RiseThemeAppearance::Light,
            CanonAppearance::Dark => RiseThemeAppearance::Dark,
        })
    }

    fn canon(self) -> CanonAppearance {
        match self {
            RiseThemeAppearance::Light => CanonAppearance::Light,
            RiseThemeAppearance::Dark => CanonAppearance::Dark,
        }
    }
}

impl Serialize for RiseThemeAppearance {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.raw_value())
    }
}

const APPEARANCE_WIRES: &[&str] = &[CanonAppearance::Light.wire(), CanonAppearance::Dark.wire()];

impl<'de> Deserialize<'de> for RiseThemeAppearance {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_raw_value(&raw)
            .ok_or_else(|| serde::de::Error::unknown_variant(&raw, APPEARANCE_WIRES))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RiseThemePreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mobile: Option<RiseThemeAppearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop: Option<RiseThemeAppearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<RiseThemeAppearance>,
}

impl RiseThemePreferences {
    pub fn new(
        mobile: Option<RiseThemeAppearance>,
        desktop: Option<RiseThemeAppearance>,
        web: Option<RiseThemeAppearance>,
    ) -> Self {
        Self { mobile, desktop, web }
    }

    pub fn appearance(&self, platform: RiseThemePlatform) -> Option<RiseThemeAppearance> {
        match platform {
            RiseThemePlatform::Mobile => self.mobile,
            RiseThemePlatform::Desktop => self.desktop,
            RiseThemePlatform::Web => self.web,
        }
    }

    pub fn setting(&self, appearance: Option<RiseThemeAppearance>, platform: RiseThemePlatform) -> RiseThemePreferences {
        let mut result = *self;
        match platform {
            RiseThemePlatform::Mobile => result.mobile = appearance,
            RiseThemePlatform::Desktop => result.desktop = appearance,
            RiseThemePlatform::Web => result.web = appearance,
        }
        result
    }
}
