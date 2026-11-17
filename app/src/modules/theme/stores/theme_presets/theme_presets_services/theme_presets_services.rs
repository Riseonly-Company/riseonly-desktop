use gpui::{App, Context, Entity};
use rise_canon::RiseRequestJSON;
use rise_canon::rise_proto::theme::{ProtoCreateThemeResult, ProtoThemeRecord, ProtoThemeWallpaper};
use rise_canon::rise_request::theme::create_theme;
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_theme::{AppTheme, RiseThemeAppearance, ThemePalette, ThemeStore};

use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;

#[derive(Clone, Debug, PartialEq)]
pub struct ThemePresetWallpaper {
    pub preset_id: String,
    pub pattern_intensity: f64,
    pub pattern_colors: Vec<String>,
    pub motion_enabled: bool,
    pub wallpaper_id: Option<String>,
    pub image_url: Option<String>,
    pub look: ChatWallpaperLook,
}

impl ThemePresetWallpaper {
    pub fn is_empty(&self) -> bool {
        self.preset_id.is_empty() && self.wallpaper_id.as_deref().unwrap_or("").is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeReplaceRequest {
    pub preset: ThemePreset,
    pub apply: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemePreset {
    pub id: String,
    pub name: String,
    pub light: ThemePalette,
    pub dark: ThemePalette,
    pub wallpaper: Option<ThemePresetWallpaper>,
    pub source_theme_id: Option<String>,
}

impl ThemePreset {
    pub fn is_saved(&self) -> bool {
        !self.source_theme_id.as_deref().unwrap_or("").is_empty()
    }

    pub fn swatches(&self, appearance: RiseThemeAppearance) -> Vec<String> {
        let palette = if appearance == RiseThemeAppearance::Dark { &self.dark } else { &self.light };
        vec![palette.bg_100.clone(), palette.primary_100.clone(), palette.bg_000.clone()]
    }

    pub fn mode(&self) -> RiseThemeAppearance {
        Self::mode_for(&self.light, &self.dark)
    }

    pub fn mode_for(light: &ThemePalette, dark: &ThemePalette) -> RiseThemeAppearance {
        let light_changes = Self::changed_tokens(light, &ThemePalette::default_light());
        let dark_changes = Self::changed_tokens(dark, &ThemePalette::default_dark());
        if light_changes > dark_changes { RiseThemeAppearance::Light } else { RiseThemeAppearance::Dark }
    }

    pub fn changed_tokens(palette: &ThemePalette, stock: &ThemePalette) -> usize {
        let colors = ThemePalette::COLOR_KEYS
            .iter()
            .filter(|key| {
                palette.color_hex(key).map(str::to_uppercase) != stock.color_hex(key).map(str::to_uppercase)
            })
            .count();
        let lengths = ThemePalette::LENGTH_KEYS.iter().filter(|key| palette.length(key) != stock.length(key)).count();
        colors + lengths
    }
}

pub enum ThemePresetCodec {}

impl ThemePresetCodec {
    pub fn preset(record: &ProtoThemeRecord) -> Option<ThemePreset> {
        if record.id.is_empty() {
            return None;
        }
        let theme = record.theme.as_ref()?;
        let light = Self::palette(theme.palette_json.as_deref())?;
        let dark = Self::palette(theme.palette_dark_json.as_deref()).unwrap_or_else(ThemePalette::default_dark);
        Some(ThemePreset {
            id: record.id.clone(),
            name: record.name.clone(),
            light,
            dark,
            wallpaper: Self::wallpaper(theme.wallpaper.as_ref()),
            source_theme_id: record.source_theme_id.clone(),
        })
    }

    pub fn preset_from_create_result(result: &ProtoCreateThemeResult) -> Option<ThemePreset> {
        Self::preset(&ProtoThemeRecord {
            id: result.id.clone(),
            name: result.name.clone(),
            platform: result.platform.clone(),
            source_theme_id: result.source_theme_id.clone(),
            source_user_id: result.source_user_id.clone(),
            theme: result.theme.clone(),
            user_id: None,
        })
    }

    pub fn request_theme(
        light: &ThemePalette,
        dark: &ThemePalette,
        wallpaper: Option<&ThemePresetWallpaper>,
    ) -> create_theme::Theme {
        create_theme::Theme {
            palette_json: Self::json(light),
            palette_dark_json: Self::json(dark),
            wallpaper: wallpaper.map(Self::request_wallpaper),
            ..create_theme::Theme::default()
        }
    }

    fn request_wallpaper(wallpaper: &ThemePresetWallpaper) -> create_theme::theme::Wallpaper {
        create_theme::theme::Wallpaper {
            blur_percent: Some(wallpaper.look.blur_percent),
            dim_percent: Some(wallpaper.look.dim_percent),
            motion_enabled: Some(wallpaper.motion_enabled),
            pattern_colors: Some(
                wallpaper.pattern_colors.iter().map(|color| RiseRequestJSON::String(color.clone())).collect(),
            ),
            pattern_intensity: Some(Self::intensity_percent(wallpaper.pattern_intensity)),
            preset_id: Some(wallpaper.preset_id.clone()),
            wallpaper_id: Some(wallpaper.wallpaper_id.clone().unwrap_or_default()),
            ..create_theme::theme::Wallpaper::default()
        }
    }

    fn wallpaper(wire: Option<&ProtoThemeWallpaper>) -> Option<ThemePresetWallpaper> {
        let wire = wire?;
        let spec = ThemePresetWallpaper {
            preset_id: wire.preset_id.clone(),
            pattern_intensity: wire.pattern_intensity as f64 / 100.0,
            pattern_colors: wire.pattern_colors.clone(),
            motion_enabled: wire.motion_enabled,
            wallpaper_id: (!wire.wallpaper_id.is_empty()).then(|| wire.wallpaper_id.clone()),
            image_url: (!wire.image_url.is_empty()).then(|| wire.image_url.clone()),
            look: ChatWallpaperLook::new(wire.dim_percent, wire.blur_percent).clamped(),
        };
        (!spec.is_empty()).then_some(spec)
    }

    fn intensity_percent(value: f64) -> i64 {
        (value.clamp(-1.0, 1.0) * 100.0).round() as i64
    }

    fn palette(json: Option<&str>) -> Option<ThemePalette> {
        let json = json.filter(|json| !json.is_empty())?;
        serde_json::from_str(json).ok()
    }

    fn json(palette: &ThemePalette) -> Option<String> {
        let value = serde_json::to_value(palette).ok()?;
        let options = JSONWritingOptions { sorted_keys: true, ..JSONWritingOptions::default() };
        let data = JSONSerialization::data_with_json_object(&value, options)?;
        String::from_utf8(data).ok()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ThemePresetsLoadState {
    Idle,
    Loading,
    Loaded,
    Failed(String),
}

pub struct ThemePresetsServicesStore {
    presets: Vec<ThemePreset>,
    load_state: ThemePresetsLoadState,
    last_applied_id: Option<String>,
    is_mutating: bool,
    preview: Option<ThemePreset>,
    preview_theme: Option<AppTheme>,
    replace_request: Option<ThemeReplaceRequest>,
}

impl ThemePresetsServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            presets: Vec::new(),
            load_state: ThemePresetsLoadState::Idle,
            last_applied_id: None,
            is_mutating: false,
            preview: None,
            preview_theme: None,
            replace_request: None,
        }
    }

    pub fn presets(&self) -> &[ThemePreset] {
        &self.presets
    }

    pub fn load_state(&self) -> &ThemePresetsLoadState {
        &self.load_state
    }

    pub fn applied_id(&self, cx: &App) -> Option<String> {
        let (light, dark) = ThemeStore::installed(cx)?.read(cx).preset_palettes();
        let matching: Vec<&ThemePreset> =
            self.presets.iter().filter(|preset| preset.light == light && preset.dark == dark).collect();
        matching
            .iter()
            .find(|preset| self.last_applied_id.as_deref() == Some(preset.id.as_str()))
            .or(matching.first())
            .map(|preset| preset.id.clone())
    }

    pub fn is_mutating(&self) -> bool {
        self.is_mutating
    }

    pub fn preview(&self) -> Option<&ThemePreset> {
        self.preview.as_ref()
    }

    pub fn set_preview(&mut self, preview: Option<ThemePreset>, cx: &mut Context<Self>) {
        self.preview = preview;
        cx.notify();
    }

    pub fn preview_theme(&self) -> Option<&AppTheme> {
        self.preview_theme.as_ref()
    }

    pub fn set_preview_theme(&mut self, preview_theme: Option<AppTheme>, cx: &mut Context<Self>) {
        self.preview_theme = preview_theme;
        cx.notify();
    }

    pub fn applied_preset(&self, cx: &App) -> Option<ThemePreset> {
        let id = self.applied_id(cx)?;
        self.presets.iter().find(|preset| preset.id == id).cloned()
    }

    pub fn replace_request(&self) -> Option<&ThemeReplaceRequest> {
        self.replace_request.as_ref()
    }

    pub fn set_replace_request(&mut self, replace_request: Option<ThemeReplaceRequest>, cx: &mut Context<Self>) {
        self.replace_request = replace_request;
        cx.notify();
    }

    pub fn publish_presets(&mut self, presets: Vec<ThemePreset>, cx: &mut Context<Self>) {
        self.presets = presets;
        self.load_state = ThemePresetsLoadState::Loaded;
        cx.notify();
    }

    pub fn publish_load_state(&mut self, load_state: ThemePresetsLoadState, cx: &mut Context<Self>) {
        self.load_state = load_state;
        cx.notify();
    }

    pub fn publish_mutating(&mut self, mutating: bool, cx: &mut Context<Self>) {
        self.is_mutating = mutating;
        cx.notify();
    }

    pub fn publish_applied_id(&mut self, applied_id: Option<String>, cx: &mut Context<Self>) {
        self.last_applied_id = applied_id;
        cx.notify();
    }

    pub fn insert(&mut self, preset: ThemePreset, cx: &mut Context<Self>) {
        self.presets.retain(|existing| existing.id != preset.id);
        self.presets.insert(0, preset);
        cx.notify();
    }

    pub fn rename(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        let Some(preset) = self.presets.iter_mut().find(|preset| preset.id == id) else {
            return;
        };
        preset.name = name.to_owned();
        cx.notify();
    }

    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) {
        self.presets.retain(|preset| preset.id != id);
        if self.last_applied_id.as_deref() == Some(id) {
            self.last_applied_id = None;
        }
        cx.notify();
    }

    pub fn remove_oldest(&mut self, saved: bool, cx: &mut Context<Self>) {
        let Some(oldest) = self.presets.iter().rev().find(|preset| preset.is_saved() == saved).map(|preset| preset.id.clone())
        else {
            return;
        };
        self.remove(&oldest, cx);
    }

    pub fn is_own(&self, id: &str) -> bool {
        self.presets.iter().any(|preset| preset.id == id)
    }
}
