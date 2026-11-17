use std::ops::{Index, IndexMut, RangeInclusive};

use rise_canon::{RiseLimits, RiseTabBarButton};
use serde::de::DeserializeOwned;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

/// which phone a set of bar settings belongs to
/// the settings page edits both
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiseTabBarPlatform {
    Ios,
    Android,
}

impl RiseTabBarPlatform {
    pub const ALL: [Self; 2] = [Self::Ios, Self::Android];

    pub const CURRENT: Self = Self::Ios;

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|platform| platform.raw_value() == raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiseTabBarSurface {
    Auto,
    Glass,
    Legacy,
}

impl RiseTabBarSurface {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Glass, Self::Legacy];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Glass => "glass",
            Self::Legacy => "legacy",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|surface| surface.raw_value() == raw)
    }
}

/// where the strip comes to rest after a pick
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiseTabBarSettle {
    Center,
    Reveal,
    ReturnHome,
}

impl RiseTabBarSettle {
    pub const ALL: [Self; 3] = [Self::Center, Self::Reveal, Self::ReturnHome];

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Reveal => "reveal",
            Self::ReturnHome => "return_home",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|settle| settle.raw_value() == raw)
    }
}

/// every button once left to right, fixed length so settings stay a plain value
pub type RiseTabBarButtonOrder = [RiseTabBarButton; RiseTabBarButton::ALL.len()];

/// codable for the tab bar button, the canon type is foreign so statics live on a trait
pub trait RiseTabBarButtonCodable: Sized {
    const MORE_MODE_BUTTONS: [Self; 4];

    /// every button exactly once, unknown or repeated ones dropped
    /// missing ones appended in default order so a newer server or old cache never loses or doubles one
    fn normalized(order: &[Self]) -> RiseTabBarButtonOrder;
}

impl RiseTabBarButtonCodable for RiseTabBarButton {
    const MORE_MODE_BUTTONS: [Self; 4] = [Self::Posts, Self::Search, Self::Chats, Self::Riffs];

    fn normalized(order: &[Self]) -> RiseTabBarButtonOrder {
        let mut result = Self::ALL;
        let mut count = 0;
        for button in order.iter().chain(Self::ALL.iter()) {
            if !result[..count].contains(button) {
                result[count] = *button;
                count += 1;
            }
        }
        result
    }
}

fn serialize_button_order<S: Serializer>(order: &RiseTabBarButtonOrder, serializer: S) -> Result<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(order.len()))?;
    for button in order {
        seq.serialize_element(button.wire())?;
    }
    seq.end()
}

/// bar settings of one platform
/// bounds and defaults are the canon user service limits the server checks saves against
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseTabBarSettings {
    pub more_tab: bool,
    pub surface: RiseTabBarSurface,
    pub loops: bool,
    pub scroll_threshold_percent: i64,
    pub speed_tenths: i64,
    pub tap_settle: RiseTabBarSettle,
    pub swipe_settle: RiseTabBarSettle,
    pub return_delay_ms: i64,
    pub throw_back: bool,
    pub throw_speed: i64,
    pub throw_selects_first: bool,
    #[serde(serialize_with = "serialize_button_order")]
    pub button_order: RiseTabBarButtonOrder,
}

impl RiseTabBarSettings {
    pub fn threshold_range() -> RangeInclusive<i64> {
        RiseLimits::service_value("tab_bar_scroll_threshold_percent_min")
            ..=RiseLimits::service_value("tab_bar_scroll_threshold_percent_max")
    }

    pub fn speed_range() -> RangeInclusive<i64> {
        RiseLimits::service_value("tab_bar_speed_tenths_min")..=RiseLimits::service_value("tab_bar_speed_tenths_max")
    }

    pub fn return_delay_range() -> RangeInclusive<i64> {
        0..=RiseLimits::service_value("tab_bar_return_delay_ms_max")
    }

    pub fn throw_speed_range() -> RangeInclusive<i64> {
        RiseLimits::service_value("tab_bar_throw_speed_min")..=RiseLimits::service_value("tab_bar_throw_speed_max")
    }

    /// what everybody gets until they change something
    pub fn defaults() -> Self {
        Self::new(
            false,
            RiseTabBarSurface::Auto,
            false,
            RiseLimits::service_value("tab_bar_scroll_threshold_percent_default"),
            RiseLimits::service_value("tab_bar_speed_tenths_default"),
            RiseTabBarSettle::ReturnHome,
            RiseTabBarSettle::ReturnHome,
            RiseLimits::service_value("tab_bar_return_delay_ms_default"),
            false,
            RiseLimits::service_value("tab_bar_throw_speed_default"),
            false,
            &RiseTabBarButton::ALL,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        more_tab: bool,
        surface: RiseTabBarSurface,
        loops: bool,
        scroll_threshold_percent: i64,
        speed_tenths: i64,
        tap_settle: RiseTabBarSettle,
        swipe_settle: RiseTabBarSettle,
        return_delay_ms: i64,
        throw_back: bool,
        throw_speed: i64,
        throw_selects_first: bool,
        button_order: &[RiseTabBarButton],
    ) -> Self {
        Self {
            more_tab,
            surface,
            loops,
            scroll_threshold_percent,
            speed_tenths,
            tap_settle,
            swipe_settle,
            return_delay_ms,
            throw_back,
            throw_speed,
            throw_selects_first,
            button_order: RiseTabBarButton::normalized(button_order),
        }
    }

    /// order the more mode shows, its four buttons as they stand in the full list
    pub fn more_mode_order(&self) -> Vec<RiseTabBarButton> {
        self.button_order.iter().copied().filter(|button| RiseTabBarButton::MORE_MODE_BUTTONS.contains(button)).collect()
    }

    pub fn with_more_mode_order(&self, four: &[RiseTabBarButton]) -> RiseTabBarSettings {
        let mut value = *self;
        let mut rest = four.iter().copied();
        let mixed: Vec<RiseTabBarButton> = self
            .button_order
            .iter()
            .map(|button| {
                if RiseTabBarButton::MORE_MODE_BUTTONS.contains(button) { rest.next().unwrap_or(*button) } else { *button }
            })
            .collect();
        value.button_order = RiseTabBarButton::normalized(&mixed);
        value
    }

    /// true if throwing makes sense, a strip without a start has nowhere to throw to
    /// and if every pick already sends it home there is nothing left to do
    pub fn throw_available(&self) -> bool {
        !self.loops
            && !(self.tap_settle == RiseTabBarSettle::ReturnHome && self.swipe_settle == RiseTabBarSettle::ReturnHome)
    }

    pub fn returns_home(&self) -> bool {
        self.tap_settle == RiseTabBarSettle::ReturnHome || self.swipe_settle == RiseTabBarSettle::ReturnHome
    }

    /// clamps into the canon bounds whatever an old cache or newer server holds
    pub fn clamped(&self) -> Self {
        let mut value = *self;
        value.scroll_threshold_percent = Self::clamp(self.scroll_threshold_percent, Self::threshold_range());
        value.speed_tenths = Self::clamp(self.speed_tenths, Self::speed_range());
        value.return_delay_ms = Self::clamp(self.return_delay_ms, Self::return_delay_range());
        value.throw_speed = Self::clamp(self.throw_speed, Self::throw_speed_range());
        value
    }

    fn clamp(value: i64, range: RangeInclusive<i64>) -> i64 {
        value.max(*range.start()).min(*range.end())
    }
}

/// a saved enum value that doesnt decode reads as absent instead of failing the whole set
fn lenient<T: DeserializeOwned>(field: Option<serde_json::Value>) -> Option<T> {
    serde_json::from_value(field?).ok()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedRiseTabBarSettings {
    #[serde(default)]
    more_tab: Option<bool>,
    #[serde(default)]
    surface: Option<serde_json::Value>,
    #[serde(default)]
    loops: Option<bool>,
    #[serde(default)]
    scroll_threshold_percent: Option<i64>,
    #[serde(default)]
    speed_tenths: Option<i64>,
    #[serde(default)]
    tap_settle: Option<serde_json::Value>,
    #[serde(default)]
    swipe_settle: Option<serde_json::Value>,
    #[serde(default)]
    return_delay_ms: Option<i64>,
    #[serde(default)]
    throw_back: Option<bool>,
    #[serde(default)]
    throw_speed: Option<i64>,
    #[serde(default)]
    throw_selects_first: Option<bool>,
    #[serde(default)]
    button_order: Option<serde_json::Value>,
}

impl<'de> Deserialize<'de> for RiseTabBarSettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let saved = SavedRiseTabBarSettings::deserialize(deserializer)?;
        let base = Self::defaults();
        let button_order: Vec<RiseTabBarButton> = lenient::<Vec<String>>(saved.button_order)
            .map(|order| order.iter().filter_map(|raw| RiseTabBarButton::from_wire(raw)).collect())
            .unwrap_or_else(|| base.button_order.to_vec());
        let value = Self::new(
            saved.more_tab.unwrap_or(base.more_tab),
            lenient(saved.surface).unwrap_or(base.surface),
            saved.loops.unwrap_or(base.loops),
            saved.scroll_threshold_percent.unwrap_or(base.scroll_threshold_percent),
            saved.speed_tenths.unwrap_or(base.speed_tenths),
            lenient(saved.tap_settle).unwrap_or(base.tap_settle),
            lenient(saved.swipe_settle).unwrap_or(base.swipe_settle),
            saved.return_delay_ms.unwrap_or(base.return_delay_ms),
            saved.throw_back.unwrap_or(base.throw_back),
            saved.throw_speed.unwrap_or(base.throw_speed),
            saved.throw_selects_first.unwrap_or(base.throw_selects_first),
            &button_order,
        );
        Ok(value.clamped())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RiseTabBarSettingsSet {
    pub ios: RiseTabBarSettings,
    pub android: RiseTabBarSettings,
}

impl RiseTabBarSettingsSet {
    pub fn defaults() -> Self {
        Self {
            ios: RiseTabBarSettings::defaults(),
            android: RiseTabBarSettings::defaults(),
        }
    }
}

impl Index<RiseTabBarPlatform> for RiseTabBarSettingsSet {
    type Output = RiseTabBarSettings;

    fn index(&self, platform: RiseTabBarPlatform) -> &Self::Output {
        match platform {
            RiseTabBarPlatform::Ios => &self.ios,
            RiseTabBarPlatform::Android => &self.android,
        }
    }
}

impl IndexMut<RiseTabBarPlatform> for RiseTabBarSettingsSet {
    fn index_mut(&mut self, platform: RiseTabBarPlatform) -> &mut Self::Output {
        match platform {
            RiseTabBarPlatform::Ios => &mut self.ios,
            RiseTabBarPlatform::Android => &mut self.android,
        }
    }
}

/// one edit of one platform bar, as the settings page makes it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseTabBarSettingsChange {
    Set(RiseTabBarSettings),
    Reset,
}

impl RiseTabBarSettingsChange {
    pub fn settings(&self) -> RiseTabBarSettings {
        match self {
            Self::Set(settings) => *settings,
            Self::Reset => RiseTabBarSettings::defaults(),
        }
    }
}
