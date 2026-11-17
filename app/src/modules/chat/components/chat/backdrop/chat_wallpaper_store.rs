use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::LazyLock;

use gpui::{App, Context, Entity, Hsla, Rgba, Subscription, black};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{
    CHAT_WALLPAPER_ACCOUNT_SCOPE as MIGRATION_KEY, CHAT_WALLPAPER_CUSTOM_PATTERN_COLORS as CUSTOM_PATTERN_COLORS_KEY,
    CHAT_WALLPAPER_IMAGE_BLUR_PERCENT as IMAGE_BLUR_KEY, CHAT_WALLPAPER_IMAGE_DIM_PERCENT as IMAGE_DIM_KEY,
    CHAT_WALLPAPER_IMAGE_ID as IMAGE_WALLPAPER_ID_KEY, CHAT_WALLPAPER_IMAGE_URL as IMAGE_WALLPAPER_URL_KEY,
    CHAT_WALLPAPER_MOTION_ENABLED as MOTION_ENABLED_KEY, CHAT_WALLPAPER_OWN_ONLY as OWN_ONLY_KEY,
    CHAT_WALLPAPER_PATTERN_INTENSITY as PATTERN_INTENSITY_KEY, CHAT_WALLPAPER_PRESET as SELECTED_PRESET_KEY,
    CHAT_WALLPAPER_SHOW_CHAT_WALLPAPER_IN as SHOW_CHAT_WALLPAPER_IN_KEY,
    CHAT_WALLPAPER_STATIC_MOTION as MOTION_PERFORMANCE_MIGRATION_KEY,
    CHAT_WALLPAPER_USE_OWN_WALLPAPER_IN as USE_OWN_WALLPAPER_IN_KEY,
};
use rise_theme::{RiseThemeAppearance, color};
use serde::{Deserialize, Serialize};

use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::theme::stores::theme_guide_sandbox_hooks::theme_guide_sandbox_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatWallpaperPlacement;
use crate::modules::theme::engine::core::rise_theme_wallpaper_wire_models::RiseWallpaperPreferences;

#[derive(Clone, Debug, PartialEq)]
pub struct ChatWallpaperPreset {
    pub id: String,
    pub background_colors: Vec<String>,
    pub pattern_colors: Vec<String>,
    pub light_background_colors: Option<Vec<String>>,
    pub light_pattern_colors: Option<Vec<String>>,
}

static ALL_PRESETS: LazyLock<Vec<ChatWallpaperPreset>> = LazyLock::new(|| {
    let preset = |id: &str, background: [&str; 4], pattern: [&str; 4]| {
        ChatWallpaperPreset::new(id.to_owned(), strings(&background), strings(&pattern), None, None)
    };
    vec![
        ChatWallpaperPreset::new(
            "telegram".to_owned(),
            strings(&["#000000", "#040103", "#080207", "#030104"]),
            strings(&["#351522", "#48182F", "#452047", "#2C152A"]),
            Some(strings(&ChatWallpaperPreset::DEFAULT_LIGHT_BACKGROUND_COLORS)),
            Some(strings(&ChatWallpaperPreset::DEFAULT_LIGHT_PATTERN_COLORS)),
        ),
        preset(
            "nightBlue",
            ["#000000", "#010711", "#020B16", "#07030D"],
            ["#12355E", "#213B6B", "#42285D", "#512548"],
        ),
        preset(
            "violet",
            ["#000000", "#07020D", "#0C0314", "#02030A"],
            ["#242B61", "#43266B", "#68284D", "#4D245E"],
        ),
        preset(
            "ember",
            ["#000000", "#080202", "#100402", "#050108"],
            ["#39305B", "#632C42", "#74341D", "#59234A"],
        ),
        preset(
            "ocean",
            ["#000000", "#01080A", "#020B0F", "#03030B"],
            ["#12465A", "#1D4E62", "#263D68", "#412B5E"],
        ),
        preset(
            "forest",
            ["#000000", "#020804", "#060A03", "#020408"],
            ["#1E4A3B", "#3F4930", "#594126", "#30385B"],
        ),
    ]
});

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|&value| value.to_owned()).collect()
}

impl ChatWallpaperPreset {
    pub const DEFAULT_LIGHT_BACKGROUND_COLORS: [&'static str; 4] = ["#DBDDBB", "#6BA587", "#D5D88D", "#88B884"];

    pub const DEFAULT_LIGHT_PATTERN_COLORS: [&'static str; 4] = ["#000000"; 4];

    pub const LIGHT_STOP_LIFTS: [f32; 4] = [0.0, 0.10, 0.05, 0.14];

    pub const CUSTOM_PRESET_ID: &'static str = "custom";

    pub const CUSTOM_BACKGROUND_COLORS: [&'static str; 4] = ["#000000", "#040103", "#080207", "#030104"];

    pub fn new(
        id: String,
        background_colors: Vec<String>,
        pattern_colors: Vec<String>,
        light_background_colors: Option<Vec<String>>,
        light_pattern_colors: Option<Vec<String>>,
    ) -> Self {
        Self { id, background_colors, pattern_colors, light_background_colors, light_pattern_colors }
    }

    pub fn swift_ui_background_colors(&self) -> Vec<Hsla> {
        self.background_colors.iter().map(|hex| color(hex)).collect()
    }

    pub fn swift_ui_pattern_colors(&self) -> Vec<Hsla> {
        self.pattern_colors.iter().map(|hex| color(hex)).collect()
    }

    pub fn resolved(&self, for_appearance: RiseThemeAppearance) -> ChatWallpaperPreset {
        if for_appearance != RiseThemeAppearance::Light {
            return self.clone();
        }
        ChatWallpaperPreset::new(
            self.id.clone(),
            self.light_background_colors.clone().unwrap_or_else(|| self.background_colors.clone()),
            self.light_pattern_colors.clone().unwrap_or_else(|| self.pattern_colors.clone()),
            None,
            None,
        )
    }

    pub fn resolved_pattern_intensity(intensity: f64, for_appearance: RiseThemeAppearance) -> f64 {
        let bounded = intensity.clamp(0.0, 1.0);
        if for_appearance == RiseThemeAppearance::Light { bounded * 0.29 } else { bounded }
    }

    pub fn light_stops(from_colors: &[String]) -> Vec<String> {
        if from_colors.is_empty() {
            return strings(&Self::DEFAULT_LIGHT_BACKGROUND_COLORS);
        }
        Self::LIGHT_STOP_LIFTS
            .iter()
            .enumerate()
            .map(|(index, &lift)| Self::pale_hex(color(&from_colors[index % from_colors.len()]), lift))
            .collect()
    }

    pub fn pale_hex(color: Hsla, lift: f32) -> String {
        let Rgba { r, g, b, .. } = color.to_rgb();
        let (hue, saturation, _) = hue_saturation_brightness(r, g, b);
        let (red, green, blue) = rgb_from_hsb(hue, (saturation * 0.34).min(1.0), (0.84 + lift).min(1.0));
        hex_from_unit_rgb(red, green, blue)
    }

    pub fn all() -> &'static [ChatWallpaperPreset] {
        &ALL_PRESETS
    }

    pub fn default_preset() -> &'static ChatWallpaperPreset {
        &ALL_PRESETS[0]
    }

    pub fn default_custom_pattern_colors() -> Vec<String> {
        Self::default_preset().pattern_colors.clone()
    }
}

fn hue_saturation_brightness(red: f32, green: f32, blue: f32) -> (f32, f32, f32) {
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let delta = max - min;
    let saturation = if max > 0.0 { delta / max } else { 0.0 };
    if delta <= 0.0 {
        return (0.0, saturation, max);
    }
    let sector = if max == red {
        (green - blue) / delta
    } else if max == green {
        2.0 + (blue - red) / delta
    } else {
        4.0 + (red - green) / delta
    };
    let hue = (sector / 6.0).rem_euclid(1.0);
    (hue, saturation, max)
}

fn rgb_from_hsb(hue: f32, saturation: f32, brightness: f32) -> (f32, f32, f32) {
    if saturation <= 0.0 {
        return (brightness, brightness, brightness);
    }
    let scaled = hue.rem_euclid(1.0) * 6.0;
    let sector = scaled.floor();
    let fraction = scaled - sector;
    let p = brightness * (1.0 - saturation);
    let q = brightness * (1.0 - saturation * fraction);
    let t = brightness * (1.0 - saturation * (1.0 - fraction));
    match sector as i32 {
        0 => (brightness, t, p),
        1 => (q, brightness, p),
        2 => (p, brightness, t),
        3 => (p, q, brightness),
        4 => (t, p, brightness),
        _ => (brightness, p, q),
    }
}

/// rgb floats in 0 to 1 as a hex string like #FF8800
fn hex_from_unit_rgb(red: f32, green: f32, blue: f32) -> String {
    let channel = |value: f32| (value * 255.0).round() as i64;
    format!("#{:02X}{:02X}{:02X}", channel(red), channel(green), channel(blue))
}

/// how a photo wallpaper is drawn under bubbles, in percent
/// lives on the server wallpaper row and gets copied to a chat when pinned
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatWallpaperLook {
    pub dim_percent: i64,
    pub blur_percent: i64,
}

impl ChatWallpaperLook {
    pub const DEFAULT_DIM_PERCENT: i64 = 28;
    pub const MAX_PERCENT: i64 = 100;
    pub const MAX_DIM_PERCENT: i64 = 70;
    pub const DEFAULT_LOOK: ChatWallpaperLook =
        ChatWallpaperLook { dim_percent: Self::DEFAULT_DIM_PERCENT, blur_percent: 0 };

    pub const fn new(dim_percent: i64, blur_percent: i64) -> Self {
        Self { dim_percent, blur_percent }
    }

    pub fn clamp_percent(value: i64) -> i64 {
        value.clamp(0, Self::MAX_PERCENT)
    }

    pub fn clamped(&self) -> ChatWallpaperLook {
        ChatWallpaperLook::new(Self::clamp_percent(self.dim_percent), Self::clamp_percent(self.blur_percent))
    }

    pub fn dim(&self) -> f64 {
        Self::clamp_percent(self.dim_percent) as f64 / 100.0
    }

    pub fn blur(&self) -> f64 {
        Self::clamp_percent(self.blur_percent) as f64 / 100.0
    }
}

/// the final picture a backdrop paints, which wallpaper won and how its drawn
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ChatWallpaperRender {
    pub image_url: String,
    pub look: ChatWallpaperLook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatWallpaperMotionMigrationResult {
    pub is_enabled: bool,
    pub should_persist: bool,
}

pub enum ChatWallpaperMotionPreferencePolicy {}

impl ChatWallpaperMotionPreferencePolicy {
    pub const DEFAULT_ENABLED: bool = false;

    pub fn resolve(stored_value: Option<bool>, migration_completed: bool) -> ChatWallpaperMotionMigrationResult {
        if migration_completed {
            return ChatWallpaperMotionMigrationResult {
                is_enabled: stored_value.unwrap_or(Self::DEFAULT_ENABLED),
                should_persist: stored_value.is_none(),
            };
        }
        ChatWallpaperMotionMigrationResult { is_enabled: false, should_persist: true }
    }
}

/// pure rule for which wallpaper a chat draws
/// chat own one wins if the account allows it then pinned photo then gradient preset (none)
pub enum ChatWallpaperResolutionPolicy {}

impl ChatWallpaperResolutionPolicy {
    pub fn shows_chat_wallpaper(
        chat_id: &str,
        own_only: bool,
        show_chat_wallpaper_in: &HashSet<String>,
        use_own_wallpaper_in: &HashSet<String>,
    ) -> bool {
        if own_only { show_chat_wallpaper_in.contains(chat_id) } else { !use_own_wallpaper_in.contains(chat_id) }
    }

    pub fn render(
        chat_id: Option<&str>,
        chat_wallpaper: Option<&ChatWallpaperPlacement>,
        own_image_url: Option<&str>,
        own_look: ChatWallpaperLook,
        own_only: bool,
        show_chat_wallpaper_in: &HashSet<String>,
        use_own_wallpaper_in: &HashSet<String>,
    ) -> Option<ChatWallpaperRender> {
        if let Some(chat_id) = chat_id.filter(|chat_id| !chat_id.is_empty())
            && let Some(chat_wallpaper) = chat_wallpaper.filter(|wallpaper| !wallpaper.image_url.is_empty())
            && Self::shows_chat_wallpaper(chat_id, own_only, show_chat_wallpaper_in, use_own_wallpaper_in)
        {
            return Some(ChatWallpaperRender { image_url: chat_wallpaper.image_url.clone(), look: chat_wallpaper.look });
        }
        if let Some(own_image_url) = own_image_url.filter(|url| !url.is_empty()) {
            return Some(ChatWallpaperRender { image_url: own_image_url.to_owned(), look: own_look });
        }
        None
    }
}

const DEFAULT_PATTERN_INTENSITY: f64 = 0.86;

const PERSISTED_KEYS: [&str; 11] = [
    SELECTED_PRESET_KEY,
    PATTERN_INTENSITY_KEY,
    MOTION_ENABLED_KEY,
    CUSTOM_PATTERN_COLORS_KEY,
    IMAGE_WALLPAPER_ID_KEY,
    IMAGE_WALLPAPER_URL_KEY,
    IMAGE_DIM_KEY,
    IMAGE_BLUR_KEY,
    OWN_ONLY_KEY,
    SHOW_CHAT_WALLPAPER_IN_KEY,
    USE_OWN_WALLPAPER_IN_KEY,
];

/// snapshot of everything the wallpaper lesson may touch, taken when it starts
#[derive(Clone, Debug, PartialEq)]
pub struct ChatWallpaperStoreGuideSnapshot {
    pub selected_preset_id: String,
    pub pattern_intensity: f64,
    pub motion_enabled: bool,
    pub custom_pattern_colors: Vec<String>,
    pub image_wallpaper_id: Option<String>,
    pub image_wallpaper_url: Option<String>,
    pub image_look: ChatWallpaperLook,
}

pub struct ChatWallpaperStore {
    selected_preset_id: String,
    pattern_intensity: f64,
    motion_enabled: bool,
    custom_pattern_colors: Vec<String>,
    image_wallpaper_id: Option<String>,
    image_wallpaper_url: Option<String>,
    image_look: ChatWallpaperLook,
    own_wallpapers_only: bool,
    show_chat_wallpaper_in: HashSet<String>,
    use_own_wallpaper_in: HashSet<String>,
    custom_light_memo: RefCell<Option<(Vec<String>, Vec<String>)>>,
}

/// the two account values a preset gradient is drawn with
/// notifies only when one changes so picking another preset doesnt rerender fixed ones
pub struct ChatWallpaperGradientTuning {
    pattern_intensity: f64,
    motion_enabled: bool,
    _store_observation: Subscription,
}

impl ChatWallpaperGradientTuning {
    pub fn shared(cx: &mut App) -> Entity<ChatWallpaperGradientTuning> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let store = ChatWallpaperStore::shared(cx);
        let (pattern_intensity, motion_enabled) = {
            let store = store.read(cx);
            (store.pattern_intensity(), store.motion_enabled())
        };
        let store_observation = cx.observe(&store, |tuning, store, cx| {
            let (pattern_intensity, motion_enabled) = {
                let store = store.read(cx);
                (store.pattern_intensity(), store.motion_enabled())
            };
            if pattern_intensity == tuning.pattern_intensity && motion_enabled == tuning.motion_enabled {
                return;
            }
            tuning.pattern_intensity = pattern_intensity;
            tuning.motion_enabled = motion_enabled;
            cx.notify();
        });
        Self { pattern_intensity, motion_enabled, _store_observation: store_observation }
    }

    pub fn pattern_intensity(&self) -> f64 {
        self.pattern_intensity
    }

    pub fn motion_enabled(&self) -> bool {
        self.motion_enabled
    }
}

impl ChatWallpaperStore {
    pub fn shared(cx: &mut App) -> Entity<ChatWallpaperStore> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        let mut store = Self {
            selected_preset_id: ChatWallpaperPreset::default_preset().id.clone(),
            pattern_intensity: DEFAULT_PATTERN_INTENSITY,
            motion_enabled: ChatWallpaperMotionPreferencePolicy::DEFAULT_ENABLED,
            custom_pattern_colors: ChatWallpaperPreset::default_custom_pattern_colors(),
            image_wallpaper_id: None,
            image_wallpaper_url: None,
            image_look: ChatWallpaperLook::DEFAULT_LOOK,
            own_wallpapers_only: false,
            show_chat_wallpaper_in: HashSet::new(),
            use_own_wallpaper_in: HashSet::new(),
            custom_light_memo: RefCell::new(None),
        };
        store.migrate_legacy_state_if_needed();
        store.load_current_account_state();
        store
    }

    pub fn selected_preset_id(&self) -> String {
        self.selected_preset_id.clone()
    }

    pub fn set_selected_preset_id(&mut self, value: String, cx: &mut Context<Self>) {
        self.selected_preset_id = value;
        self.persist();
        cx.notify();
    }

    pub fn pattern_intensity(&self) -> f64 {
        self.pattern_intensity
    }

    pub fn set_pattern_intensity(&mut self, value: f64, cx: &mut Context<Self>) {
        self.pattern_intensity = value;
        self.persist();
        GuideSignalBus::emit(RiseGuideSignal::WallpaperGradientChanged);
        cx.notify();
    }

    pub fn motion_enabled(&self) -> bool {
        self.motion_enabled
    }

    pub fn set_motion_enabled(&mut self, value: bool, cx: &mut Context<Self>) {
        self.motion_enabled = value;
        self.persist();
        GuideSignalBus::emit(RiseGuideSignal::WallpaperGradientChanged);
        cx.notify();
    }

    pub fn custom_pattern_colors(&self) -> &[String] {
        &self.custom_pattern_colors
    }

    pub fn set_custom_pattern_colors(&mut self, value: Vec<String>, cx: &mut Context<Self>) {
        self.custom_pattern_colors = value;
        self.persist();
        GuideSignalBus::emit(RiseGuideSignal::WallpaperPaletteChanged);
        cx.notify();
    }

    pub fn image_wallpaper_id(&self) -> Option<&str> {
        self.image_wallpaper_id.as_deref()
    }

    pub fn image_wallpaper_url(&self) -> Option<&str> {
        self.image_wallpaper_url.as_deref()
    }

    pub fn image_look(&self) -> ChatWallpaperLook {
        self.image_look
    }

    pub fn own_wallpapers_only(&self) -> bool {
        self.own_wallpapers_only
    }

    pub fn show_chat_wallpaper_in(&self) -> &HashSet<String> {
        &self.show_chat_wallpaper_in
    }

    pub fn use_own_wallpaper_in(&self) -> &HashSet<String> {
        &self.use_own_wallpaper_in
    }

    pub fn uses_image_wallpaper(&self) -> bool {
        self.image_wallpaper_url.as_deref().is_some_and(|url| !url.is_empty())
    }

    pub fn current_preset(&self) -> ChatWallpaperPreset {
        if self.selected_preset_id == ChatWallpaperPreset::CUSTOM_PRESET_ID {
            return self.custom_preset();
        }
        ChatWallpaperPreset::all()
            .iter()
            .find(|preset| preset.id == self.selected_preset_id)
            .unwrap_or_else(ChatWallpaperPreset::default_preset)
            .clone()
    }

    pub fn custom_preset(&self) -> ChatWallpaperPreset {
        ChatWallpaperPreset::new(
            ChatWallpaperPreset::CUSTOM_PRESET_ID.to_owned(),
            strings(&ChatWallpaperPreset::CUSTOM_BACKGROUND_COLORS),
            self.custom_pattern_colors.clone(),
            Some(self.custom_light_background_colors()),
            Some(strings(&ChatWallpaperPreset::DEFAULT_LIGHT_PATTERN_COLORS)),
        )
    }

    fn custom_light_background_colors(&self) -> Vec<String> {
        let mut memo = self.custom_light_memo.borrow_mut();
        if let Some((source, stops)) = memo.as_ref()
            && *source == self.custom_pattern_colors
        {
            return stops.clone();
        }
        let stops = ChatWallpaperPreset::light_stops(&self.custom_pattern_colors);
        *memo = Some((self.custom_pattern_colors.clone(), stops.clone()));
        stops
    }

    pub fn shows_chat_wallpaper(&self, chat_id: &str, cx: &App) -> bool {
        if let Some(invented) = (theme_guide_sandbox_hooks().shows_chat_wallpaper)(chat_id, cx) {
            return invented;
        }
        ChatWallpaperResolutionPolicy::shows_chat_wallpaper(
            chat_id,
            self.own_wallpapers_only,
            &self.show_chat_wallpaper_in,
            &self.use_own_wallpaper_in,
        )
    }

    pub fn render(
        &self,
        chat_id: Option<&str>,
        chat_wallpaper: Option<&ChatWallpaperPlacement>,
        cx: &App,
    ) -> Option<ChatWallpaperRender> {
        if let Some(chat_id) = chat_id
            && let Some(invented) = (theme_guide_sandbox_hooks().shows_chat_wallpaper)(chat_id, cx)
        {
            let use_own_wallpaper_in: HashSet<String> =
                if invented { HashSet::new() } else { HashSet::from([chat_id.to_owned()]) };
            return ChatWallpaperResolutionPolicy::render(
                Some(chat_id),
                chat_wallpaper,
                self.image_wallpaper_url.as_deref(),
                self.image_look,
                false,
                &HashSet::new(),
                &use_own_wallpaper_in,
            );
        }
        ChatWallpaperResolutionPolicy::render(
            chat_id,
            chat_wallpaper,
            self.image_wallpaper_url.as_deref(),
            self.image_look,
            self.own_wallpapers_only,
            &self.show_chat_wallpaper_in,
            &self.use_own_wallpaper_in,
        )
    }

    pub fn select(&mut self, preset: &ChatWallpaperPreset, cx: &mut Context<Self>) {
        self.set_selected_preset_id(preset.id.clone(), cx);
    }

    /// applies the server chat background
    /// ignored when it names another account, a late answer must not repaint the current one
    pub fn apply_image_wallpaper(
        &mut self,
        id: Option<&str>,
        url: Option<&str>,
        look: Option<ChatWallpaperLook>,
        account_id: &str,
        account_epoch: &str,
        cx: &mut Context<Self>,
    ) {
        if !SaiAccountCacheScope::is_current_account_epoch(Some(account_id), Some(account_epoch)) {
            return;
        }
        if (theme_guide_sandbox_hooks().owns_app_wallpaper_scene)(cx) {
            return;
        }
        self.set_image_wallpaper(id, url, look, cx);
    }

    /// practice background of the wallpaper lesson, this device only
    pub fn apply_practice_image_wallpaper(
        &mut self,
        id: Option<&str>,
        url: Option<&str>,
        look: Option<ChatWallpaperLook>,
        cx: &mut Context<Self>,
    ) {
        self.set_image_wallpaper(id, url, look, cx);
    }

    fn set_image_wallpaper(
        &mut self,
        id: Option<&str>,
        url: Option<&str>,
        look: Option<ChatWallpaperLook>,
        cx: &mut Context<Self>,
    ) {
        let next_id = id.map(trimming_whitespaces_and_newlines).filter(|value| !value.is_empty()).map(str::to_owned);
        let next_url =
            url.map(trimming_whitespaces_and_newlines).filter(|value| !value.is_empty()).map(str::to_owned);
        let next_look = look.unwrap_or(ChatWallpaperLook::DEFAULT_LOOK).clamped();
        if next_id == self.image_wallpaper_id && next_url == self.image_wallpaper_url && next_look == self.image_look {
            return;
        }
        self.image_wallpaper_id = next_id;
        self.image_wallpaper_url = next_url;
        self.image_look = next_look;
        self.persist();
        cx.notify();
    }

    pub fn apply_preferences(
        &mut self,
        preferences: &RiseWallpaperPreferences,
        account_id: &str,
        account_epoch: &str,
        cx: &mut Context<Self>,
    ) {
        if !SaiAccountCacheScope::is_current_account_epoch(Some(account_id), Some(account_epoch)) {
            return;
        }
        if preferences.own_only == self.own_wallpapers_only
            && preferences.show_chat_wallpaper_in == self.show_chat_wallpaper_in
            && preferences.use_own_wallpaper_in == self.use_own_wallpaper_in
        {
            return;
        }
        self.own_wallpapers_only = preferences.own_only;
        self.show_chat_wallpaper_in = preferences.show_chat_wallpaper_in.clone();
        self.use_own_wallpaper_in = preferences.use_own_wallpaper_in.clone();
        self.persist();
        cx.notify();
    }

    pub fn apply_own_only_optimistic(&mut self, own_only: bool, cx: &mut Context<Self>) {
        if self.own_wallpapers_only == own_only {
            return;
        }
        self.own_wallpapers_only = own_only;
        self.persist();
        cx.notify();
    }

    pub fn apply_shows_chat_wallpaper_optimistic(&mut self, shows: bool, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty()
            || ChatWallpaperResolutionPolicy::shows_chat_wallpaper(
                chat_id,
                self.own_wallpapers_only,
                &self.show_chat_wallpaper_in,
                &self.use_own_wallpaper_in,
            ) == shows
        {
            return;
        }
        if self.own_wallpapers_only {
            if shows {
                self.show_chat_wallpaper_in.insert(chat_id.to_owned());
            } else {
                self.show_chat_wallpaper_in.remove(chat_id);
            }
        } else if shows {
            self.use_own_wallpaper_in.remove(chat_id);
        } else {
            self.use_own_wallpaper_in.insert(chat_id.to_owned());
        }
        self.persist();
        cx.notify();
    }

    pub fn guide_snapshot(&self) -> ChatWallpaperStoreGuideSnapshot {
        ChatWallpaperStoreGuideSnapshot {
            selected_preset_id: self.selected_preset_id.clone(),
            pattern_intensity: self.pattern_intensity,
            motion_enabled: self.motion_enabled,
            custom_pattern_colors: self.custom_pattern_colors.clone(),
            image_wallpaper_id: self.image_wallpaper_id.clone(),
            image_wallpaper_url: self.image_wallpaper_url.clone(),
            image_look: self.image_look,
        }
    }

    pub fn restore(&mut self, snapshot: ChatWallpaperStoreGuideSnapshot, cx: &mut Context<Self>) {
        self.selected_preset_id = snapshot.selected_preset_id;
        self.pattern_intensity = snapshot.pattern_intensity;
        self.motion_enabled = snapshot.motion_enabled;
        self.custom_pattern_colors = snapshot.custom_pattern_colors;
        self.image_wallpaper_id = snapshot.image_wallpaper_id;
        self.image_wallpaper_url = snapshot.image_wallpaper_url;
        self.image_look = snapshot.image_look;
        self.persist();
        cx.notify();
    }

    pub fn custom_color(&self, at_index: usize) -> Hsla {
        match self.custom_pattern_colors.get(at_index) {
            Some(hex) => color(hex),
            None => black(),
        }
    }

    pub fn update_custom_color(&mut self, at_index: usize, color: Hsla, cx: &mut Context<Self>) {
        if at_index >= self.custom_pattern_colors.len() {
            return;
        }
        let hex = Self::hex_string(color);
        if self.selected_preset_id != ChatWallpaperPreset::CUSTOM_PRESET_ID {
            let colors = self.current_preset().pattern_colors;
            self.set_custom_pattern_colors(colors, cx);
        }
        if at_index >= self.custom_pattern_colors.len() {
            return;
        }
        let mut colors = self.custom_pattern_colors.clone();
        colors[at_index] = hex;
        self.set_custom_pattern_colors(colors, cx);
        self.set_selected_preset_id(ChatWallpaperPreset::CUSTOM_PRESET_ID.to_owned(), cx);
    }

    /// back to the shipped look
    /// image pin and prefs stay since they live on the server and the screen releases them
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.set_selected_preset_id(ChatWallpaperPreset::default_preset().id.clone(), cx);
        self.set_pattern_intensity(DEFAULT_PATTERN_INTENSITY, cx);
        self.set_motion_enabled(ChatWallpaperMotionPreferencePolicy::DEFAULT_ENABLED, cx);
        self.set_custom_pattern_colors(ChatWallpaperPreset::default_custom_pattern_colors(), cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.selected_preset_id = ChatWallpaperPreset::default_preset().id.clone();
        self.pattern_intensity = DEFAULT_PATTERN_INTENSITY;
        self.motion_enabled = ChatWallpaperMotionPreferencePolicy::DEFAULT_ENABLED;
        self.custom_pattern_colors = ChatWallpaperPreset::default_custom_pattern_colors();
        self.image_wallpaper_id = None;
        self.image_wallpaper_url = None;
        self.image_look = ChatWallpaperLook::DEFAULT_LOOK;
        self.own_wallpapers_only = false;
        self.show_chat_wallpaper_in = HashSet::new();
        self.use_own_wallpaper_in = HashSet::new();
        cx.notify();
    }

    pub fn activate_account_state(&mut self, cx: &mut Context<Self>) {
        self.migrate_legacy_state_if_needed();
        self.load_current_account_state();
        cx.notify();
    }

    /// wipes every wallpaper key of this account including the motion migration flag
    pub fn remove_persisted_account(&self, user_id: &str) {
        let defaults = UserDefaults::standard();
        for key in PERSISTED_KEYS {
            defaults.remove_object(&Self::scoped_key_for(key, Some(user_id)));
        }
        defaults.remove_object(&Self::scoped_key_for(MOTION_PERFORMANCE_MIGRATION_KEY, Some(user_id)));
    }

    fn persist(&self) {
        let defaults = UserDefaults::standard();
        defaults.set(&Self::scoped_key(SELECTED_PRESET_KEY), self.selected_preset_id.clone());
        defaults.set(&Self::scoped_key(PATTERN_INTENSITY_KEY), self.pattern_intensity);
        defaults.set(&Self::scoped_key(MOTION_ENABLED_KEY), self.motion_enabled);
        defaults.set(&Self::scoped_key(CUSTOM_PATTERN_COLORS_KEY), self.custom_pattern_colors.clone());
        defaults.set(&Self::scoped_key(IMAGE_DIM_KEY), self.image_look.dim_percent);
        defaults.set(&Self::scoped_key(IMAGE_BLUR_KEY), self.image_look.blur_percent);
        defaults.set(&Self::scoped_key(OWN_ONLY_KEY), self.own_wallpapers_only);
        defaults.set(
            &Self::scoped_key(SHOW_CHAT_WALLPAPER_IN_KEY),
            self.show_chat_wallpaper_in.iter().cloned().collect::<Vec<String>>(),
        );
        defaults.set(
            &Self::scoped_key(USE_OWN_WALLPAPER_IN_KEY),
            self.use_own_wallpaper_in.iter().cloned().collect::<Vec<String>>(),
        );
        if let (Some(image_wallpaper_id), Some(image_wallpaper_url)) =
            (self.image_wallpaper_id.as_ref(), self.image_wallpaper_url.as_ref())
        {
            defaults.set(&Self::scoped_key(IMAGE_WALLPAPER_ID_KEY), image_wallpaper_id.clone());
            defaults.set(&Self::scoped_key(IMAGE_WALLPAPER_URL_KEY), image_wallpaper_url.clone());
        } else {
            defaults.remove_object(&Self::scoped_key(IMAGE_WALLPAPER_ID_KEY));
            defaults.remove_object(&Self::scoped_key(IMAGE_WALLPAPER_URL_KEY));
        }
    }

    fn load_custom_pattern_colors(defaults: &UserDefaults) -> Vec<String> {
        match defaults.string_array(&Self::scoped_key(CUSTOM_PATTERN_COLORS_KEY)) {
            Some(colors) if colors.len() == 4 => colors,
            _ => ChatWallpaperPreset::default_custom_pattern_colors(),
        }
    }

    fn load_current_account_state(&mut self) {
        let defaults = UserDefaults::standard();
        let saved_preset_id = defaults.string(&Self::scoped_key(SELECTED_PRESET_KEY));
        let is_known = saved_preset_id.as_deref().is_some_and(|saved| {
            ChatWallpaperPreset::all().iter().any(|preset| preset.id == saved)
                || saved == ChatWallpaperPreset::CUSTOM_PRESET_ID
        });
        self.selected_preset_id = match saved_preset_id {
            Some(saved) if is_known => saved,
            _ => ChatWallpaperPreset::default_preset().id.clone(),
        };
        self.custom_pattern_colors = Self::load_custom_pattern_colors(&defaults);
        self.image_wallpaper_id = defaults.string(&Self::scoped_key(IMAGE_WALLPAPER_ID_KEY));
        self.image_wallpaper_url = defaults.string(&Self::scoped_key(IMAGE_WALLPAPER_URL_KEY));
        let dim_key = Self::scoped_key(IMAGE_DIM_KEY);
        let blur_key = Self::scoped_key(IMAGE_BLUR_KEY);
        self.image_look = ChatWallpaperLook::new(
            if defaults.object(&dim_key).is_none() {
                ChatWallpaperLook::DEFAULT_DIM_PERCENT
            } else {
                defaults.integer(&dim_key)
            },
            defaults.integer(&blur_key),
        )
        .clamped();
        self.own_wallpapers_only = defaults.bool(&Self::scoped_key(OWN_ONLY_KEY));
        self.show_chat_wallpaper_in = defaults
            .string_array(&Self::scoped_key(SHOW_CHAT_WALLPAPER_IN_KEY))
            .unwrap_or_default()
            .into_iter()
            .collect();
        self.use_own_wallpaper_in = defaults
            .string_array(&Self::scoped_key(USE_OWN_WALLPAPER_IN_KEY))
            .unwrap_or_default()
            .into_iter()
            .collect();

        let intensity_key = Self::scoped_key(PATTERN_INTENSITY_KEY);
        self.pattern_intensity = if defaults.object(&intensity_key).is_none() {
            DEFAULT_PATTERN_INTENSITY
        } else {
            defaults.double(&intensity_key)
        };
        let motion_key = Self::scoped_key(MOTION_ENABLED_KEY);
        let stored_motion = if defaults.object(&motion_key).is_none() { None } else { Some(defaults.bool(&motion_key)) };
        let migration_key = Self::scoped_key(MOTION_PERFORMANCE_MIGRATION_KEY);
        let motion = ChatWallpaperMotionPreferencePolicy::resolve(stored_motion, defaults.bool(&migration_key));
        self.motion_enabled = motion.is_enabled;
        if motion.should_persist {
            defaults.set(&motion_key, motion.is_enabled);
        }
        if !defaults.bool(&migration_key) {
            defaults.set(&migration_key, true);
        }
    }

    fn migrate_legacy_state_if_needed(&self) {
        let defaults = UserDefaults::standard();
        if SaiAccountCacheScope::active_user_id().is_none() || defaults.bool(MIGRATION_KEY) {
            return;
        }
        for key in [
            SELECTED_PRESET_KEY,
            PATTERN_INTENSITY_KEY,
            MOTION_ENABLED_KEY,
            CUSTOM_PATTERN_COLORS_KEY,
            IMAGE_WALLPAPER_ID_KEY,
            IMAGE_WALLPAPER_URL_KEY,
        ] {
            let Some(value) = defaults.object(key) else {
                continue;
            };
            defaults.set(&Self::scoped_key(key), value);
            defaults.remove_object(key);
        }
        defaults.set(MIGRATION_KEY, true);
    }

    fn scoped_key(key: &str) -> String {
        Self::scoped_key_for(key, SaiAccountCacheScope::active_user_id().as_deref())
    }

    fn scoped_key_for(key: &str, user_id: Option<&str>) -> String {
        let value = user_id.map(trimming_whitespaces_and_newlines).unwrap_or("");
        if value.is_empty() { format!("account:anonymous:{key}") } else { format!("account:{value}:{key}") }
    }

    fn hex_string(color: Hsla) -> String {
        let Rgba { r, g, b, .. } = color.to_rgb();
        hex_from_unit_rgb(r, g, b)
    }
}
