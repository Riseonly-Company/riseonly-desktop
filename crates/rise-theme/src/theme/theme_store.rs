use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, Global, Task, WindowAppearance};
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::{
    THEME_LAUNCH_APPEARANCE, THEME_LEGACY_PALETTE, THEME_MOBILE_APPEARANCE, THEME_PALETTE_BUCKETS, THEME_TEXT_DELTA,
};
use serde::{Deserialize, Serialize};

use super::app_theme::AppTheme;
use super::rise_theme_preferences::{RiseThemeAppearance, RiseThemePlatform, RiseThemePreferences};
use super::theme_palette::ThemePalette;
use crate::tokens::density::Density;

enum ThemePersistenceKey {}

impl ThemePersistenceKey {
    const LEGACY_PALETTE: &'static str = THEME_LEGACY_PALETTE;
    const PALETTE_BUCKETS: &'static str = THEME_PALETTE_BUCKETS;
    const MOBILE_APPEARANCE: &'static str = THEME_MOBILE_APPEARANCE;
    const TEXT_DELTA: &'static str = THEME_TEXT_DELTA;
    const LAUNCH_APPEARANCE: &'static str = THEME_LAUNCH_APPEARANCE;

    fn mobile_appearance(account_id: &str) -> String {
        format!("{}.{}", Self::MOBILE_APPEARANCE, account_id)
    }
}

/// text size slider, a point delta added to every text component in half point steps
/// device preference, not an account one
pub enum RiseTextSizePolicy {}

impl RiseTextSizePolicy {
    pub const RANGE: RangeInclusive<f64> = -3.0..=5.0;
    pub const STEP: f64 = 0.5;
    pub const DEFAULT_DELTA: f64 = 0.5;

    pub fn normalized(raw: f64) -> f64 {
        if !raw.is_finite() {
            return Self::DEFAULT_DELTA;
        }
        let steps = ((raw - *Self::RANGE.start()) / Self::STEP).round();
        let quantized = *Self::RANGE.start() + steps * Self::STEP;
        quantized.clamp(*Self::RANGE.start(), *Self::RANGE.end())
    }

    /// counted from the default so the standard size reads 0 and not +0.5
    pub fn label(delta: f64) -> String {
        let normalized = Self::normalized(delta) - Self::DEFAULT_DELTA;
        if normalized == 0.0 {
            return "0".to_string();
        }
        let magnitude = if normalized % 1.0 == 0.0 {
            format!("{}", normalized.abs() as i64)
        } else {
            format!("{:.1}", normalized.abs())
        };
        let sign = if normalized > 0.0 { '+' } else { '\u{2212}' };
        format!("{sign}{magnitude}")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ThemePaletteBuckets {
    light: ThemePalette,
    dark: ThemePalette,
}

impl ThemePaletteBuckets {
    fn defaults() -> Self {
        Self { light: ThemePalette::default_light(), dark: ThemePalette::default_dark() }
    }

    fn get(&self, appearance: RiseThemeAppearance) -> &ThemePalette {
        match appearance {
            RiseThemeAppearance::Light => &self.light,
            RiseThemeAppearance::Dark => &self.dark,
        }
    }

    fn set(&mut self, appearance: RiseThemeAppearance, palette: ThemePalette) {
        match appearance {
            RiseThemeAppearance::Light => self.light = palette,
            RiseThemeAppearance::Dark => self.dark = palette,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ThemePresentation {
    palette: ThemePalette,
    appearance: RiseThemeAppearance,
    mobile_preference: Option<RiseThemeAppearance>,
}

/// persistence actor, newest generation wins and an older pending write is dropped
#[derive(Default)]
struct ThemePalettePersistence {
    latest_generation: u64,
    pending_task: Option<Task<()>>,
}

const PALETTE_PERSISTENCE_DELAY: Duration = Duration::from_millis(120);
const SYSTEM_APPEARANCE_REREAD_DELAY: Duration = Duration::from_millis(350);

impl ThemePalettePersistence {
    fn schedule(
        &mut self,
        buckets: ThemePaletteBuckets,
        legacy_palette: ThemePalette,
        generation: u64,
        cx: &mut Context<ThemeStore>,
    ) {
        if generation < self.latest_generation {
            return;
        }
        self.latest_generation = generation;
        let timer = cx.background_executor().timer(PALETTE_PERSISTENCE_DELAY);
        self.pending_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |store, _| {
                if generation != store.palette_persistence.latest_generation {
                    return;
                }
                let (Ok(bucket_data), Ok(legacy_data)) =
                    (serde_json::to_vec(&buckets), serde_json::to_vec(&legacy_palette))
                else {
                    return;
                };
                let defaults = UserDefaults::standard();
                defaults.set(ThemePersistenceKey::PALETTE_BUCKETS, bucket_data);
                defaults.set(ThemePersistenceKey::LEGACY_PALETTE, legacy_data);
                store.palette_persistence.pending_task = None;
            });
        }));
    }
}

/// palettes and appearance as they stand
/// for a lesson where the reader tries the theme and gets it back after
#[derive(Clone, Debug)]
pub struct GuideSnapshot {
    buckets: ThemePaletteBuckets,
    preferences: RiseThemePreferences,
}

static TEXT_DELTA_SNAPSHOT: AtomicU32 = AtomicU32::new(0x3F00_0000);

struct SharedThemeStore(Entity<ThemeStore>);

impl Global for SharedThemeStore {}

pub struct ThemeStore {
    presentation: ThemePresentation,
    palette_buckets: ThemePaletteBuckets,
    system_appearance: RiseThemeAppearance,
    active_account_id: Option<String>,
    installed_preferences: RiseThemePreferences,
    persistence_generation: u64,
    palette_seed_task: Option<Task<()>>,
    palette_persistence: ThemePalettePersistence,
    is_palette_seed_ready: bool,
    text_delta: f32,
    current_theme: AppTheme,
    forced_window_appearance: Option<RiseThemeAppearance>,
}

impl ThemeStore {
    /// the shared store
    /// private slot rather than rise_ui shared, since rise-ui sits above this crate and depends on it
    pub fn shared(cx: &mut App) -> Entity<ThemeStore> {
        if let Some(existing) = cx.try_global::<SharedThemeStore>() {
            return existing.0.clone();
        }
        let entity = cx.new(Self::new);
        cx.set_global(SharedThemeStore(entity.clone()));
        let theme = entity.read(cx).current_theme.clone();
        theme.publish_focus_ring_color();
        cx.set_global(theme);
        entity
    }

    /// shared store for readers with only &App, eg a render that cant build it
    pub fn installed(cx: &App) -> Option<Entity<ThemeStore>> {
        cx.try_global::<SharedThemeStore>().map(|shared| shared.0.clone())
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let buckets = ThemePaletteBuckets::defaults();
        let system_appearance = Self::current_system_appearance(cx);
        let preferences = RiseThemePreferences::default().setting(Self::stored_launch_appearance(), RiseThemePlatform::THIS_DEVICE);
        let launch_appearance = preferences.appearance(RiseThemePlatform::THIS_DEVICE).unwrap_or(system_appearance);

        let presentation = ThemePresentation {
            palette: buckets.get(launch_appearance).clone(),
            appearance: launch_appearance,
            mobile_preference: preferences.appearance(RiseThemePlatform::THIS_DEVICE),
        };
        let current_theme = Self::build_theme(&presentation.palette);

        let stored_delta = UserDefaults::standard().object(ThemePersistenceKey::TEXT_DELTA).and_then(|value| value.as_f64());
        let delta = RiseTextSizePolicy::normalized(stored_delta.unwrap_or(RiseTextSizePolicy::DEFAULT_DELTA)) as f32;
        TEXT_DELTA_SNAPSHOT.store(delta.to_bits(), Ordering::Relaxed);

        let mut store = Self {
            presentation,
            palette_buckets: buckets,
            system_appearance,
            active_account_id: None,
            installed_preferences: preferences,
            persistence_generation: 0,
            palette_seed_task: None,
            palette_persistence: ThemePalettePersistence::default(),
            is_palette_seed_ready: false,
            text_delta: delta,
            current_theme,
            forced_window_appearance: None,
        };
        store.apply_window_appearance(cx);
        store
    }

    pub fn current_palette(&self) -> &ThemePalette {
        &self.presentation.palette
    }

    /// stored not computed, building a theme parses forty colors and render reads it in hundreds of places
    /// rebuilt only when the palette changes
    pub fn current_theme(&self) -> &AppTheme {
        &self.current_theme
    }

    pub fn preferences(&self) -> RiseThemePreferences {
        self.installed_preferences
    }

    pub fn resolved_appearance(&self) -> RiseThemeAppearance {
        self.presentation.appearance
    }

    pub fn is_palette_seed_ready(&self) -> bool {
        self.is_palette_seed_ready
    }

    /// points added to every text component
    pub fn text_delta(&self) -> f32 {
        self.text_delta
    }

    /// same value for readers off the main thread, chat layout math runs wherever a row is prepared
    /// written only by the store
    pub fn text_delta_snapshot() -> f32 {
        f32::from_bits(TEXT_DELTA_SNAPSHOT.load(Ordering::Relaxed))
    }

    /// window chrome follows the palette own background and not the bucket name
    /// apply writes into the current bucket so an edited palette can be dark in the light bucket
    pub fn preferred_color_scheme(&self) -> Option<RiseThemeAppearance> {
        let ink = self.ink_appearance();
        if self.presentation.mobile_preference.is_none() && ink == self.presentation.appearance {
            return None;
        }
        Some(ink)
    }

    /// for extra windows and popovers, they take the resolved appearance explicitly
    pub fn window_interface_style(&self) -> RiseThemeAppearance {
        self.ink_appearance()
    }

    /// light or dark by the palette own background, not the bucket name
    pub fn ink_appearance(&self) -> RiseThemeAppearance {
        Self::appearance_for_palette(&self.presentation.palette)
    }

    pub fn set_text_delta(&mut self, raw: f64, cx: &mut Context<Self>) {
        let delta = RiseTextSizePolicy::normalized(raw) as f32;
        if delta == self.text_delta {
            return;
        }
        self.text_delta = delta;
        TEXT_DELTA_SNAPSHOT.store(delta.to_bits(), Ordering::Relaxed);
        UserDefaults::standard().set(ThemePersistenceKey::TEXT_DELTA, f64::from(delta));
        cx.notify();
    }

    pub fn ensure_palette_seed(&mut self, cx: &mut Context<Self>) {
        if self.is_palette_seed_ready {
            return;
        }
        self.palette_seed_task = None;
        self.system_appearance = Self::current_system_appearance(cx);
        self.install_palette_seed(Self::load_palette_buckets(), cx);
    }

    pub fn prepare_palette_seed(&mut self, cx: &mut Context<Self>) {
        if self.is_palette_seed_ready || self.palette_seed_task.is_some() {
            return;
        }
        let load = cx.background_spawn(async move { Self::load_palette_buckets() });
        self.palette_seed_task = Some(cx.spawn(async move |this, cx| {
            let buckets = load.await;
            let _ = this.update(cx, |store, cx| {
                if store.is_palette_seed_ready {
                    return;
                }
                store.palette_seed_task = None;
                store.install_palette_seed(buckets, cx);
            });
        }));
    }

    fn install_palette_seed(&mut self, buckets: ThemePaletteBuckets, cx: &mut Context<Self>) {
        let appearance = self.presentation.mobile_preference.unwrap_or(self.system_appearance);
        let next_presentation = ThemePresentation {
            palette: buckets.get(appearance).clone(),
            appearance,
            mobile_preference: self.presentation.mobile_preference,
        };
        let next_theme = Self::build_theme(&next_presentation.palette);

        self.palette_buckets = buckets;
        self.presentation = next_presentation;
        self.set_current_theme(next_theme, cx);
        self.is_palette_seed_ready = true;
        self.apply_window_appearance(cx);
        cx.notify();
    }

    /// a window reported its appearance, handles the system color scheme and interface style change
    pub fn system_color_scheme_did_change(&mut self, appearance: WindowAppearance, cx: &mut Context<Self>) {
        if Self::is_taking_background_snapshots() {
            return;
        }
        self.update_system_appearance(Self::appearance_for_window(appearance), cx);
    }

    pub fn system_interface_style_did_change(&mut self, interface_style: WindowAppearance, cx: &mut Context<Self>) {
        self.system_color_scheme_did_change(interface_style, cx);
    }

    pub fn refresh_system_appearance(&mut self, cx: &mut Context<Self>) {
        let reading = Self::current_system_appearance(cx);
        self.update_system_appearance(reading, cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SYSTEM_APPEARANCE_REREAD_DELAY).await;
            let _ = this.update(cx, |store, cx| {
                if Self::is_taking_background_snapshots() {
                    return;
                }
                let reading = Self::current_system_appearance(cx);
                store.update_system_appearance(reading, cx);
            });
        })
        .detach();
    }

    /// while the root follows the system, an app wide appearance forced from an earlier palette is let go
    /// so a live system switch reaches the windows again
    pub fn release_stale_override(&mut self, follows_system: bool, cx: &mut Context<Self>) {
        if !follows_system {
            return;
        }
        if self.forced_window_appearance.take().is_some() {
            cx.set_window_appearance(None);
        }
    }

    fn is_taking_background_snapshots() -> bool {
        false
    }

    fn update_system_appearance(&mut self, reading: RiseThemeAppearance, cx: &mut Context<Self>) {
        if self.presentation.mobile_preference.is_some() {
            return;
        }
        let appearance = if self.preferred_color_scheme().is_none() {
            reading
        } else {
            self.current_unforced_system_appearance(cx).unwrap_or(self.system_appearance)
        };
        if self.system_appearance == appearance {
            return;
        }
        self.system_appearance = appearance;
        self.commit(None, cx);
    }

    pub fn activate(&mut self, account_id: Option<String>, cx: &mut Context<Self>) {
        if self.active_account_id == account_id {
            return;
        }
        let seed = account_id.as_deref().and_then(Self::stored_mobile_appearance);
        self.installed_preferences = RiseThemePreferences::default().setting(seed, RiseThemePlatform::THIS_DEVICE);
        self.active_account_id = account_id;
        self.commit(self.installed_preferences.appearance(RiseThemePlatform::THIS_DEVICE), cx);
    }

    pub fn drop_launch_seed_if_signed_out(&mut self, account_id: Option<&str>, cx: &mut Context<Self>) {
        let device_preference = self.installed_preferences.appearance(RiseThemePlatform::THIS_DEVICE);
        if account_id.is_some() || self.active_account_id.is_some() || device_preference.is_none() {
            return;
        }
        self.installed_preferences = RiseThemePreferences::default();
        self.commit(None, cx);
    }

    pub fn deactivate(&mut self, account_id: &str, cx: &mut Context<Self>) -> bool {
        if self.active_account_id.as_deref() != Some(account_id) {
            return false;
        }
        self.active_account_id = None;
        self.installed_preferences = RiseThemePreferences::default();
        self.commit(None, cx);
        true
    }

    pub fn install(&mut self, preferences: RiseThemePreferences, account_id: &str, cx: &mut Context<Self>) -> bool {
        if self.active_account_id.as_deref() != Some(account_id) {
            return false;
        }
        self.installed_preferences = preferences;
        let appearance = preferences.appearance(RiseThemePlatform::THIS_DEVICE);
        Self::store_mobile_appearance(appearance, account_id);
        self.commit(appearance, cx);
        true
    }

    pub fn apply(&mut self, palette: ThemePalette, cx: &mut Context<Self>) {
        let appearance = self.presentation.appearance;
        self.palette_buckets.set(appearance, palette.clone());
        self.commit(self.presentation.mobile_preference, cx);
        self.schedule_persistence(palette, cx);
    }

    pub fn apply_from_json(&mut self, data: &[u8], cx: &mut Context<Self>) -> anyhow::Result<()> {
        let palette: ThemePalette = serde_json::from_slice(data)?;
        self.apply(palette, cx);
        Ok(())
    }

    pub fn encode_to_json(&self) -> anyhow::Result<Vec<u8>> {
        let value = serde_json::to_value(&self.presentation.palette)?;
        let options = JSONWritingOptions { sorted_keys: true, pretty_printed: true, ..JSONWritingOptions::default() };
        JSONSerialization::data_with_json_object(&value, options)
            .ok_or_else(|| anyhow::anyhow!("palette is not a JSON object"))
    }

    pub fn set_default_dark(&mut self, cx: &mut Context<Self>) {
        self.install_builtin_palette(ThemePalette::default_dark(), RiseThemeAppearance::Dark, cx);
    }

    pub fn set_default_light(&mut self, cx: &mut Context<Self>) {
        self.install_builtin_palette(ThemePalette::default_light(), RiseThemeAppearance::Light, cx);
    }

    pub fn guide_snapshot(&self) -> GuideSnapshot {
        GuideSnapshot { buckets: self.palette_buckets.clone(), preferences: self.installed_preferences }
    }

    pub fn restore(&mut self, snapshot: GuideSnapshot, cx: &mut Context<Self>) {
        self.palette_buckets = snapshot.buckets;
        self.installed_preferences = snapshot.preferences;
        if let Some(account_id) = self.active_account_id.as_deref() {
            Self::store_mobile_appearance(snapshot.preferences.appearance(RiseThemePlatform::THIS_DEVICE), account_id);
        }
        self.commit(snapshot.preferences.appearance(RiseThemePlatform::THIS_DEVICE), cx);
        self.schedule_persistence(self.presentation.palette.clone(), cx);
    }

    /// both palettes as installed, light and dark
    /// a saved theme carries the pair so the receiver keeps their own light, dark or system choice
    pub fn preset_palettes(&self) -> (ThemePalette, ThemePalette) {
        (self.palette_buckets.light.clone(), self.palette_buckets.dark.clone())
    }

    pub fn install_preset_palettes(
        &mut self,
        light: ThemePalette,
        dark: ThemePalette,
        persist: bool,
        cx: &mut Context<Self>,
    ) {
        self.palette_buckets = ThemePaletteBuckets { light, dark };
        self.commit(self.presentation.mobile_preference, cx);
        if !persist {
            return;
        }
        self.schedule_persistence(self.presentation.palette.clone(), cx);
    }

    pub fn reference_builtin_palette(&self) -> ThemePalette {
        match self.presentation.appearance {
            RiseThemeAppearance::Light => ThemePalette::default_light(),
            RiseThemeAppearance::Dark => ThemePalette::default_dark(),
        }
    }

    fn commit(&mut self, mobile_preference: Option<RiseThemeAppearance>, cx: &mut Context<Self>) {
        Self::store_launch_appearance(mobile_preference);
        if self.presentation.mobile_preference.is_some() && mobile_preference.is_none() {
            if self.forced_window_appearance.take().is_some() {
                cx.set_window_appearance(None);
            }
            if let Some(refreshed) = self.current_unforced_system_appearance(cx) {
                self.system_appearance = refreshed;
            }
        }

        let appearance = mobile_preference.unwrap_or(self.system_appearance);
        let next_presentation = ThemePresentation {
            palette: self.palette_buckets.get(appearance).clone(),
            appearance,
            mobile_preference,
        };
        if next_presentation == self.presentation {
            return;
        }

        let next_theme =
            (next_presentation.palette != self.presentation.palette).then(|| Self::build_theme(&next_presentation.palette));
        self.presentation = next_presentation;
        if let Some(next_theme) = next_theme {
            self.set_current_theme(next_theme, cx);
        }
        self.apply_window_appearance(cx);
        cx.notify();
    }

    fn schedule_persistence(&mut self, legacy_palette: ThemePalette, cx: &mut Context<Self>) {
        self.persistence_generation = self.persistence_generation.wrapping_add(1);
        let buckets = self.palette_buckets.clone();
        let generation = self.persistence_generation;
        self.palette_persistence.schedule(buckets, legacy_palette, generation, cx);
    }

    fn install_builtin_palette(&mut self, palette: ThemePalette, appearance: RiseThemeAppearance, cx: &mut Context<Self>) {
        self.palette_buckets.set(appearance, palette.clone());
        self.installed_preferences = self.installed_preferences.setting(Some(appearance), RiseThemePlatform::THIS_DEVICE);
        self.commit(Some(appearance), cx);
        self.schedule_persistence(palette, cx);
    }

    fn build_theme(palette: &ThemePalette) -> AppTheme {
        let appearance = Self::appearance_for_palette(palette);
        AppTheme::new(palette, appearance, Density::default())
    }

    fn set_current_theme(&mut self, theme: AppTheme, cx: &mut Context<Self>) {
        theme.publish_focus_ring_color();
        cx.set_global(theme.clone());
        self.current_theme = theme;
    }

    /// sets the app wide window appearance, every new window and popover takes it too
    fn apply_window_appearance(&mut self, cx: &mut Context<Self>) {
        let preferred = self.preferred_color_scheme();
        if preferred == self.forced_window_appearance {
            return;
        }
        self.forced_window_appearance = preferred;
        cx.set_window_appearance(preferred.map(RiseThemeAppearance::color_scheme));
    }

    fn stored_mobile_appearance(account_id: &str) -> Option<RiseThemeAppearance> {
        UserDefaults::standard()
            .string(&ThemePersistenceKey::mobile_appearance(account_id))
            .and_then(|raw| RiseThemeAppearance::from_raw_value(&raw))
    }

    fn stored_launch_appearance() -> Option<RiseThemeAppearance> {
        UserDefaults::standard()
            .string(ThemePersistenceKey::LAUNCH_APPEARANCE)
            .and_then(|raw| RiseThemeAppearance::from_raw_value(&raw))
    }

    fn store_launch_appearance(appearance: Option<RiseThemeAppearance>) {
        let defaults = UserDefaults::standard();
        match appearance {
            Some(appearance) => defaults.set(ThemePersistenceKey::LAUNCH_APPEARANCE, appearance.raw_value()),
            None => defaults.remove_object(ThemePersistenceKey::LAUNCH_APPEARANCE),
        }
    }

    fn store_mobile_appearance(appearance: Option<RiseThemeAppearance>, account_id: &str) {
        let key = ThemePersistenceKey::mobile_appearance(account_id);
        let defaults = UserDefaults::standard();
        match appearance {
            Some(appearance) => defaults.set(&key, appearance.raw_value()),
            None => defaults.remove_object(&key),
        }
    }

    fn load_palette_buckets() -> ThemePaletteBuckets {
        let defaults = UserDefaults::standard();
        if let Some(buckets) = defaults
            .data(ThemePersistenceKey::PALETTE_BUCKETS)
            .and_then(|data| serde_json::from_slice::<ThemePaletteBuckets>(&data).ok())
        {
            return buckets;
        }
        let Some(palette) = defaults
            .data(ThemePersistenceKey::LEGACY_PALETTE)
            .and_then(|data| serde_json::from_slice::<ThemePalette>(&data).ok())
        else {
            return ThemePaletteBuckets::defaults();
        };
        let mut buckets = ThemePaletteBuckets::defaults();
        buckets.set(Self::appearance_for_palette(&palette), palette);
        buckets
    }

    pub fn appearance_for_palette(palette: &ThemePalette) -> RiseThemeAppearance {
        if Self::background_luminance(&palette.bg_100) >= 0.45 { RiseThemeAppearance::Light } else { RiseThemeAppearance::Dark }
    }

    fn current_system_appearance(cx: &App) -> RiseThemeAppearance {
        Self::appearance_for_window(cx.window_appearance())
    }

    /// system reading while this store isnt forcing the app appearance
    /// while it forces one the platform reports it back so no honest reading exists and its None
    fn current_unforced_system_appearance(&self, cx: &App) -> Option<RiseThemeAppearance> {
        if self.forced_window_appearance.is_some() {
            return None;
        }
        Some(Self::current_system_appearance(cx))
    }

    fn appearance_for_window(appearance: WindowAppearance) -> RiseThemeAppearance {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => RiseThemeAppearance::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => RiseThemeAppearance::Dark,
        }
    }

    fn background_luminance(hex: &str) -> f64 {
        let value = trimming_whitespaces_and_newlines(hex);
        let value = value.strip_prefix('#').unwrap_or(value);
        if value.chars().count() != 6 {
            return 0.0;
        }
        let Ok(number) = i64::from_str_radix(value, 16) else {
            return 0.0;
        };
        let red = ((number >> 16) & 0xFF) as f64 / 255.0;
        let green = ((number >> 8) & 0xFF) as f64 / 255.0;
        let blue = (number & 0xFF) as f64 / 255.0;
        0.2126 * red + 0.7152 * green + 0.0722 * blue
    }
}

impl RiseThemeAppearance {
    fn color_scheme(self) -> WindowAppearance {
        match self {
            RiseThemeAppearance::Light => WindowAppearance::Light,
            RiseThemeAppearance::Dark => WindowAppearance::Dark,
        }
    }
}
