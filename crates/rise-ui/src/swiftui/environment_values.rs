use gpui::{App, Global};
use rise_theme::{RiseThemeAppearance, ThemeStore};

use crate::swiftui::environment::{EnvKey, env};

/// light or dark color scheme
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorScheme {
    Light,
    Dark,
}

impl ColorScheme {
    pub fn from_appearance(appearance: RiseThemeAppearance) -> Self {
        match appearance {
            RiseThemeAppearance::Light => Self::Light,
            RiseThemeAppearance::Dark => Self::Dark,
        }
    }
}

/// color scheme pinned by a subtree, eg the media viewer is always dark
pub struct ColorSchemeKey;

impl EnvKey for ColorSchemeKey {
    type Value = Option<ColorScheme>;

    fn default_value() -> Self::Value {
        None
    }
}

/// pinned scheme of the subtree, else the window one from the theme store
pub fn color_scheme(cx: &App) -> ColorScheme {
    if let Some(pinned) = env::<ColorSchemeKey>(cx) {
        return pinned;
    }
    ThemeStore::installed(cx)
        .map(|store| ColorScheme::from_appearance(store.read(cx).resolved_appearance()))
        .unwrap_or(ColorScheme::Light)
}

/// is enabled key, a disabled ancestor sets it to false for the subtree
pub struct IsEnabledKey;

impl EnvKey for IsEnabledKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        true
    }
}

/// true unless an ancestor disabled the subtree
pub fn is_enabled(cx: &App) -> bool {
    env::<IsEnabledKey>(cx)
}

/// system reduce transparency setting the app shell last published
#[derive(Clone, Copy, Default)]
struct ReduceTransparency(bool);

impl Global for ReduceTransparency {}

/// called by the app shell with the system setting and on each change
pub fn set_accessibility_reduce_transparency(reduce: bool, cx: &mut App) {
    cx.set_global(ReduceTransparency(reduce));
}

/// true if reduce transparency is on, off until the shell publishes the setting
pub fn accessibility_reduce_transparency(cx: &App) -> bool {
    cx.try_global::<ReduceTransparency>().is_some_and(|value| value.0)
}
