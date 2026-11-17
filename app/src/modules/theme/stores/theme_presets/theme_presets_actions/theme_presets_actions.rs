use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::{RiseLimits, rise_request};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_errors::ErrorCode;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};
use rise_i18n::tr;
use rise_theme::{AppTheme, Density, RiseThemeAppearance, RiseThemePlatform, ThemePalette, ThemeStore};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::rise_engine_bridge::rise_transport_status::{RiseTransportDropWatch, RiseTransportStatus};
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{ChatWallpaperPreset, ChatWallpaperStore};
use crate::modules::theme::engine::core::rise_theme_rpc::RiseThemeRPC;
use crate::modules::theme::engine::rise_theme_domain::{RiseThemeActionExecutor, RiseThemeDomain};
use crate::modules::theme::stores::theme_presets::theme_presets_services::theme_presets_services::{
    ThemePreset, ThemePresetCodec, ThemePresetWallpaper, ThemePresetsLoadState, ThemePresetsServicesStore,
    ThemeReplaceRequest,
};
use crate::modules::theme::stores::theme_wallpapers::theme_wallpapers_actions::theme_wallpapers_actions::ThemeWallpapersActionsStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

const LINK_CONNECTION_WAIT: Duration = Duration::from_secs(15);
const REPLACE_SHEET_DELAY: Duration = Duration::from_millis(350);
const THEMES_PAGE_LIMIT: i64 = 50;

pub struct ThemePresetsActionsStore {
    link_theme_id: Option<String>,
    link_reconnect_watch: Option<RiseTransportDropWatch>,
    link_connection_deadline: Option<Task<()>>,
}

impl ThemePresetsActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { link_theme_id: None, link_reconnect_watch: None, link_connection_deadline: None }
    }

    fn stock_preset() -> ThemePreset {
        ThemePreset {
            id: String::new(),
            name: String::new(),
            light: ThemePalette::default_light(),
            dark: ThemePalette::default_dark(),
            wallpaper: None,
            source_theme_id: None,
        }
    }

    pub fn load_action(&mut self, cx: &mut Context<Self>) {
        let services = ThemePresetsServicesStore::shared(cx);
        services.update(cx, |services, cx| services.publish_load_state(ThemePresetsLoadState::Loading, cx));
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let page = match domain.await {
                Ok(domain) => {
                    Self::rpc(domain)
                        .get_themes(rise_request::theme::GetThemes {
                            limit: Some(THEMES_PAGE_LIMIT),
                            ..rise_request::theme::GetThemes::default()
                        })
                        .await
                }
                Err(error) => Err(error),
            };
            cx.update(|cx| {
                services.update(cx, |services, cx| match page {
                    Ok(page) => {
                        services.publish_presets(page.items.iter().filter_map(ThemePresetCodec::preset).collect(), cx)
                    }
                    Err(_) => services.publish_load_state(ThemePresetsLoadState::Failed(tr("theme_unavailable")), cx),
                })
            });
        })
        .detach();
    }

    pub fn own_limit() -> i64 {
        RiseLimits::service_value("max_themes_per_user")
    }

    pub fn saved_limit() -> i64 {
        RiseLimits::service_value("max_saved_themes_per_user")
    }

    pub fn own_at_limit(cx: &App) -> bool {
        Self::kind_at_limit(false, Self::own_limit(), cx)
    }

    fn saved_at_limit(cx: &App) -> bool {
        Self::kind_at_limit(true, Self::saved_limit(), cx)
    }

    fn kind_at_limit(saved: bool, limit: i64, cx: &App) -> bool {
        let Some(services) = rise_ui::shared::try_read::<ThemePresetsServicesStore>(cx) else {
            return false;
        };
        *services.load_state() == ThemePresetsLoadState::Loaded
            && services.presets().iter().filter(|preset| preset.is_saved() == saved).count() as i64 >= limit
    }

    pub fn save_current_action(&mut self, name: &str, replace_oldest: bool, cx: &mut Context<Self>) {
        let (light, dark) = ThemeStore::shared(cx).read(cx).preset_palettes();
        let wallpaper = Self::current_wallpaper(cx);
        self.create(name.to_owned(), light, dark, wallpaper, None, None, replace_oldest, false, cx);
    }

    pub fn confirm_replace_saved_action(&mut self, replace_theme_id: Option<String>, cx: &mut Context<Self>) {
        let services = ThemePresetsServicesStore::shared(cx);
        let Some(request) = services.read(cx).replace_request().cloned() else {
            return;
        };
        services.update(cx, |services, cx| services.set_replace_request(None, cx));
        let replace_oldest = replace_theme_id.is_none();
        self.create(
            request.preset.name,
            request.preset.light,
            request.preset.dark,
            request.preset.wallpaper,
            Some(request.preset.id),
            replace_theme_id,
            replace_oldest,
            request.apply,
            cx,
        );
    }

    pub fn cancel_replace_saved_action(&mut self, cx: &mut Context<Self>) {
        ThemePresetsServicesStore::shared(cx).update(cx, |services, cx| services.set_replace_request(None, cx));
    }

    pub fn apply_action(&mut self, preset: &ThemePreset, cx: &mut Context<Self>) {
        Self::install(preset, true, cx);
        let id = preset.id.clone();
        ThemePresetsServicesStore::shared(cx).update(cx, |services, cx| services.publish_applied_id(Some(id), cx));
    }

    pub fn apply_in_its_mode_action(&mut self, preset: &ThemePreset, cx: &mut Context<Self>) {
        let mode = preset.mode();
        if ThemeStore::shared(cx).read(cx).resolved_appearance() != mode {
            UserInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.set_theme_appearance(Some(mode), RiseThemePlatform::THIS_DEVICE, cx));
        }
        self.apply_action(preset, cx);
    }

    pub fn appearance_did_change_action(&mut self, appearance: RiseThemeAppearance, cx: &mut Context<Self>) {
        let services = ThemePresetsServicesStore::shared(cx);
        let Some(applied) = services.read(cx).applied_preset(cx) else {
            return;
        };
        if applied.mode() == appearance {
            return;
        }
        Self::install(&Self::stock_preset(), true, cx);
        services.update(cx, |services, cx| services.publish_applied_id(None, cx));
    }

    pub fn rename_action(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        let trimmed = Self::trimmed_name(name);
        let services = ThemePresetsServicesStore::shared(cx);
        if trimmed.is_empty() {
            return;
        }
        let Some(preset) = services.read(cx).presets().iter().find(|preset| preset.id == id).cloned() else {
            return;
        };
        let Some(theme) = Self::update_theme(&ThemePresetCodec::request_theme(
            &preset.light,
            &preset.dark,
            preset.wallpaper.as_ref(),
        )) else {
            return;
        };
        services.update(cx, |services, cx| services.rename(id, &trimmed, cx));
        let request = rise_request::theme::UpdateTheme {
            name: Some(trimmed),
            platform: Some(RiseThemePlatform::Mobile.raw_value().to_owned()),
            theme: Some(theme),
            theme_id: Some(id.to_owned()),
        };
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => Self::rpc(domain).update_theme(request).await.map(|_| ()),
                Err(error) => Err(error),
            };
            if result.is_ok() {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                Self::notify(tr("theme_unavailable"), NotifierType::Error, cx);
                this.load_action(cx);
            });
        })
        .detach();
    }

    pub fn delete_action(&mut self, id: &str, cx: &mut Context<Self>) {
        ThemePresetsServicesStore::shared(cx).update(cx, |services, cx| services.remove(id, cx));
        let request = rise_request::theme::DeleteTheme { theme_id: Some(id.to_owned()) };
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => Self::rpc(domain).delete_theme(request).await.map(|_| ()),
                Err(error) => Err(error),
            };
            if result.is_ok() {
                return;
            }
            let _ = this.update(cx, |this, cx| this.load_action(cx));
        })
        .detach();
    }

    pub fn open_link_action(&mut self, theme_id: String, on_own: impl FnOnce(&mut App) + 'static, cx: &mut Context<Self>) {
        if ThemePresetsServicesStore::shared(cx).read(cx).is_own(&theme_id) {
            on_own(cx);
            return;
        }
        self.stop_link_connection_wait();
        self.link_theme_id = Some(theme_id.clone());
        RiseTransportStatus::wake_if_offline(cx);
        self.load_linked_theme(theme_id, false, cx);
    }

    fn load_linked_theme(&mut self, theme_id: String, retried_while_connected: bool, cx: &mut Context<Self>) {
        let request = rise_request::theme::GetTheme { theme_id: Some(theme_id.clone()) };
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => Self::rpc(domain).get_theme(request).await,
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |this, cx| {
                if this.link_theme_id.as_deref() != Some(theme_id.as_str()) {
                    return;
                }
                match result {
                    Ok(record) => {
                        this.finish_linked_theme();
                        match record.as_ref().and_then(ThemePresetCodec::preset) {
                            Some(preset) => this.begin_preview(preset, cx),
                            None => Self::notify(tr("theme_unavailable"), NotifierType::Error, cx),
                        }
                    }
                    Err(error) => {
                        if CanonErrors::is_connection_loss(&error) {
                            this.wait_for_link_connection(theme_id, retried_while_connected, cx);
                            return;
                        }
                        this.finish_linked_theme();
                        Self::notify(tr("theme_unavailable"), NotifierType::Error, cx);
                    }
                }
            });
        })
        .detach();
    }

    fn wait_for_link_connection(&mut self, theme_id: String, retried_while_connected: bool, cx: &mut Context<Self>) {
        if self.link_connection_deadline.is_none() {
            let deadline_theme_id = theme_id.clone();
            self.link_connection_deadline = Some(cx.spawn(async move |this, cx: &mut AsyncApp| {
                cx.background_executor().timer(LINK_CONNECTION_WAIT).await;
                let _ = this.update(cx, |this, cx| {
                    if this.link_theme_id.as_deref() != Some(deadline_theme_id.as_str()) {
                        return;
                    }
                    if let Some(deadline) = this.link_connection_deadline.take() {
                        deadline.detach();
                    }
                    this.finish_linked_theme();
                    Self::notify(tr("error_connection_lost"), NotifierType::Info, cx);
                });
            }));
        }
        if let Some(watch) = self.link_reconnect_watch.take() {
            watch.stop();
        }
        let this = cx.entity().downgrade();
        let watch_theme_id = theme_id.clone();
        self.link_reconnect_watch = Some(RiseTransportStatus::on_next_connect(
            move |cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.link_theme_id.as_deref() != Some(watch_theme_id.as_str()) {
                        return;
                    }
                    this.link_reconnect_watch = None;
                    this.load_linked_theme(watch_theme_id, retried_while_connected, cx);
                });
            },
            cx,
        ));
        RiseTransportStatus::wake_if_offline(cx);
        if RiseTransportStatus::is_connected(cx) && !retried_while_connected {
            if let Some(watch) = self.link_reconnect_watch.take() {
                watch.stop();
            }
            self.load_linked_theme(theme_id, true, cx);
        }
    }

    fn finish_linked_theme(&mut self) {
        self.link_theme_id = None;
        self.stop_link_connection_wait();
    }

    fn stop_link_connection_wait(&mut self) {
        if let Some(watch) = self.link_reconnect_watch.take() {
            watch.stop();
        }
        self.link_connection_deadline = None;
    }

    pub fn begin_preview(&mut self, preset: ThemePreset, cx: &mut Context<Self>) {
        let palette = if preset.mode() == RiseThemeAppearance::Dark { &preset.dark } else { &preset.light };
        let preview_theme = AppTheme::new(palette, ThemeStore::appearance_for_palette(palette), Density::default());
        ThemePresetsServicesStore::shared(cx).update(cx, |services, cx| {
            services.set_preview_theme(Some(preview_theme), cx);
            services.set_preview(Some(preset), cx);
        });
    }

    pub fn cancel_preview_action(&mut self, cx: &mut Context<Self>) {
        Self::end_preview(cx);
    }

    pub fn save_preview_action(&mut self, apply: bool, asked: Option<ThemePreset>, cx: &mut Context<Self>) {
        let Some(preset) = asked.or_else(|| ThemePresetsServicesStore::shared(cx).read(cx).preview().cloned()) else {
            return;
        };
        Self::end_preview(cx);
        if Self::saved_at_limit(cx) {
            Self::ask_which_saved_gives_way(preset, apply, cx);
            return;
        }
        self.create(preset.name, preset.light, preset.dark, preset.wallpaper, Some(preset.id), None, false, apply, cx);
    }

    fn ask_which_saved_gives_way(preset: ThemePreset, apply: bool, cx: &mut App) {
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(REPLACE_SHEET_DELAY).await;
            cx.update(|cx| {
                ThemePresetsServicesStore::shared(cx).update(cx, |services, cx| {
                    services.set_replace_request(Some(ThemeReplaceRequest { preset, apply }), cx)
                })
            });
        })
        .detach();
    }

    fn end_preview(cx: &mut App) {
        ThemePresetsServicesStore::shared(cx).update(cx, |services, cx| services.set_preview(None, cx));
    }

    #[allow(clippy::too_many_arguments)]
    fn create(
        &mut self,
        name: String,
        light: ThemePalette,
        dark: ThemePalette,
        wallpaper: Option<ThemePresetWallpaper>,
        source_theme_id: Option<String>,
        replace_theme_id: Option<String>,
        replace_oldest: bool,
        apply_after: bool,
        cx: &mut Context<Self>,
    ) {
        let trimmed = Self::trimmed_name(&name);
        let services = ThemePresetsServicesStore::shared(cx);
        if services.read(cx).is_mutating() {
            return;
        }
        services.update(cx, |services, cx| services.publish_mutating(true, cx));
        let request = rise_request::theme::CreateTheme {
            name: Some(if trimmed.is_empty() { Self::default_name() } else { trimmed.clone() }),
            platform: Some(RiseThemePlatform::Mobile.raw_value().to_owned()),
            replace_oldest: replace_oldest.then_some(true),
            replace_theme_id: replace_theme_id.clone(),
            source_theme_id: source_theme_id.clone(),
            theme: Some(ThemePresetCodec::request_theme(&light, &dark, wallpaper.as_ref())),
        };
        let domain = RiseThemeActionExecutor::require_domain(cx);
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => Self::rpc(domain).create_theme(request).await,
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(result) => {
                        match result.as_ref().and_then(ThemePresetCodec::preset_from_create_result) {
                            Some(preset) => {
                                services.update(cx, |services, cx| {
                                    if let Some(replace_theme_id) = replace_theme_id.as_deref() {
                                        services.remove(replace_theme_id, cx);
                                    } else if replace_oldest {
                                        services.remove_oldest(preset.is_saved(), cx);
                                    }
                                    services.insert(preset.clone(), cx);
                                });
                                if apply_after {
                                    this.apply_in_its_mode_action(&preset, cx);
                                }
                                Self::notify(tr("theme_saved"), NotifierType::Success, cx);
                                if replace_oldest {
                                    this.load_action(cx);
                                }
                            }
                            None => Self::notify(tr("theme_unavailable"), NotifierType::Error, cx),
                        }
                    }
                    Err(error) => {
                        let failure = CanonErrors::failure(&error);
                        let code = failure
                            .as_ref()
                            .and_then(|failure| CanonErrors::entry(failure.code.as_deref()))
                            .map(|entry| entry.code);
                        if code == Some(ErrorCode::ThemeSavedLimit) && replace_theme_id.is_none() && !replace_oldest {
                            if let Some(source_theme_id) = source_theme_id {
                                let preset = ThemePreset {
                                    id: source_theme_id,
                                    name: trimmed,
                                    light,
                                    dark,
                                    wallpaper,
                                    source_theme_id: None,
                                };
                                services.update(cx, |services, cx| services.publish_mutating(false, cx));
                                this.load_action(cx);
                                Self::ask_which_saved_gives_way(preset, apply_after, cx);
                                return;
                            }
                        }
                        if matches!(code, Some(ErrorCode::ThemeLimit | ErrorCode::ThemeSavedLimit)) {
                            this.load_action(cx);
                        }
                        let title = failure.as_ref().and_then(CanonErrors::text_for).unwrap_or_else(|| tr("theme_unavailable"));
                        Self::notify(title, NotifierType::Error, cx);
                    }
                }
                services.update(cx, |services, cx| services.publish_mutating(false, cx));
            });
        })
        .detach();
    }

    fn install(preset: &ThemePreset, persist_image: bool, cx: &mut App) {
        let (light, dark) = (preset.light.clone(), preset.dark.clone());
        ThemeStore::shared(cx).update(cx, |store, cx| store.install_preset_palettes(light, dark, persist_image, cx));
        let wallpapers = ChatWallpaperStore::shared(cx);
        let Some(spec) = preset.wallpaper.clone() else {
            wallpapers.update(cx, |wallpapers, cx| wallpapers.reset(cx));
            if persist_image {
                ThemeWallpapersActionsStore::shared(cx).update(cx, |store, cx| store.select_action(None, false, cx));
            } else {
                wallpapers.update(cx, |wallpapers, cx| wallpapers.apply_practice_image_wallpaper(None, None, None, cx));
            }
            return;
        };
        if !spec.preset_id.is_empty() {
            wallpapers.update(cx, |wallpapers, cx| {
                wallpapers.set_selected_preset_id(spec.preset_id.clone(), cx);
                wallpapers.set_pattern_intensity(spec.pattern_intensity, cx);
                wallpapers.set_motion_enabled(spec.motion_enabled, cx);
                if spec.preset_id == ChatWallpaperPreset::CUSTOM_PRESET_ID && !spec.pattern_colors.is_empty() {
                    wallpapers.set_custom_pattern_colors(spec.pattern_colors.clone(), cx);
                }
            });
        }
        if persist_image {
            let wallpaper_id = spec.wallpaper_id.clone();
            ThemeWallpapersActionsStore::shared(cx).update(cx, |store, cx| store.select_action(wallpaper_id, true, cx));
        } else {
            wallpapers.update(cx, |wallpapers, cx| {
                wallpapers.apply_practice_image_wallpaper(
                    spec.wallpaper_id.as_deref(),
                    spec.image_url.as_deref(),
                    Some(spec.look),
                    cx,
                )
            });
        }
    }

    fn current_wallpaper(cx: &mut App) -> Option<ThemePresetWallpaper> {
        let snapshot = ChatWallpaperStore::shared(cx).read(cx).guide_snapshot();
        let is_custom = snapshot.selected_preset_id == ChatWallpaperPreset::CUSTOM_PRESET_ID;
        let spec = ThemePresetWallpaper {
            preset_id: snapshot.selected_preset_id,
            pattern_intensity: snapshot.pattern_intensity,
            pattern_colors: if is_custom { snapshot.custom_pattern_colors } else { Vec::new() },
            motion_enabled: snapshot.motion_enabled,
            wallpaper_id: snapshot.image_wallpaper_id,
            image_url: snapshot.image_wallpaper_url,
            look: snapshot.image_look,
        };
        (!spec.is_empty()).then_some(spec)
    }

    fn trimmed_name(name: &str) -> String {
        trimming_whitespaces_and_newlines(name).to_owned()
    }

    pub fn default_name() -> String {
        let mut formatter = DateFormatter::new();
        formatter.set_date_style(Style::Short);
        formatter.set_time_style(Style::None);
        format!("{} {}", tr("theme_name_placeholder"), formatter.string_from(&Date::now()))
    }

    fn update_theme(
        theme: &rise_request::theme::create_theme::Theme,
    ) -> Option<rise_request::theme::update_theme::Theme> {
        let value = serde_json::to_value(theme).ok()?;
        serde_json::from_value(value).ok()
    }

    pub fn share_url(theme_id: &str) -> String {
        RiseonlyPublicLink::Theme { id: theme_id.to_owned() }
            .web_url(AppEnvironment::current())
            .map(|url| url.to_string())
            .unwrap_or_else(|| theme_id.to_owned())
    }

    fn notify(title: String, kind: NotifierType, cx: &mut App) {
        Notifier::send(title, NotifierSendOptions { r#type: kind, ..NotifierSendOptions::default() }, cx);
    }

    fn rpc(domain: Arc<RiseThemeDomain>) -> RiseThemeRPC {
        RiseThemeRPC::new(domain.engine.clone())
    }
}
