use std::path::PathBuf;
use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{
    AnyElement, App, AppContext as _, AssetSource, AsyncApp, Entity, Global, IntoElement as _, ParentElement as _, QuitMode,
};
use gpui_platform::application;
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::rise_app_events::{RiseAppEvents, RiseNotificationName, UIApplication};
use rise_diagnostics::rise_breadcrumbs::{RiseBreadcrumbs, RiseBreadcrumbsChannel};
use rise_energy::rise_energy_saving::RiseEnergySavingStore;
use rise_i18n::language_store::LanguageStore;
use rise_platform::single_instance::{self, Instance};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::environment::{LayoutDirection, LayoutDirectionKey, env_scope};
use rise_widgets::GlassHost;
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::app::composition;
use crate::app::launch_snapshot_tint::LaunchSnapshotTint;
use crate::app::rise_launch_splash::RiseLaunchSplashBoundary;
use crate::app::shell::window_presenter::WindowPresenter;
use crate::app::shell::{handover, shell_actions, system_settings_bridge};
use crate::app::storybook::{self, Storybook};
use crate::core::assets::RiseAssets;
use crate::core::config::app_environment::AppEnvironment;
use crate::core::config::endpoints::Endpoints;
use crate::core::diagnostics::rise_log_file::{LOG_FILE_FILTER, LOG_FILE_PANIC_FLUSH, RiseLogFile};
use crate::core::desktop_platform::tray_controller::{LaunchPresentation, TrayController};
use crate::core::i18n::locale_bootstrap::LocaleBootstrap;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::SaiWsDeferredOutboundAppHooks;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::media_http::MediaHttpClient;
use crate::core::preferences::rise_launch_preferences::RiseLaunchPreferences;
use crate::core::push_notifications::socket_notification_source::SocketNotificationSource;
use crate::core::restriction::restriction_store::RestrictionStore;
use crate::core::rise_engine_bridge::rise_engine_trace_bridge::RiseEngineTraceBridge;
use crate::core::stores::http::http_service::HttpService;
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::chat::components::chat::async_render::async_text_bubble_predisplay::AsyncTextBubblePredisplayCache;
use crate::modules::chat::components::chat::async_render::async_text_bubble_prepared_layout_cache::AsyncTextBubblePreparedLayoutCache;
use crate::modules::chat::components::chat::async_render::rise_message_fallback_height_cache::RiseMessageFallbackHeightCache;
use crate::modules::event::stores::event_interactions::event_interactions::EventInteractionsStore;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::rtc::media::rise_rtc_live_kit_media_controller::RiseRTCLiveKitBootstrap;
use crate::modules::system_share::desktop_share_intake::split_launch_arguments;

pub const NS_SYSTEM_TIME_ZONE_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("NSSystemTimeZoneDidChangeNotification");
pub const CURRENT_LOCALE_DID_CHANGE: RiseNotificationName =
    RiseNotificationName::new("NSCurrentLocaleDidChangeNotification");

const LAUNCH_SPLASH_ID: &str = "rise-launch-splash";

/// app entry
/// window root stays RootView since every shell lookup downcasts to it
/// so scene boundaries wrap its render
pub struct RiseonlyApp {
    is_launch_splash_presented: bool,
}

struct RiseonlyAppScene(Entity<RiseonlyApp>);

impl Global for RiseonlyAppScene {}

impl RiseonlyApp {
    fn report_renderer(cx: &mut App) {
        let Some(window) = cx.windows().first().copied() else {
            return;
        };
        let Ok(Some(specs)) = window.update(cx, |_, window, _| window.gpu_specs()) else {
            return;
        };
        let line = format!(
            "renderer {} driver {} {} software {}",
            specs.device_name, specs.driver_name, specs.driver_info, specs.is_software_emulated
        );
        tracing::info!(target: "riseonly", "{line}");
        RiseBreadcrumbs::record(RiseBreadcrumbsChannel::Lifecycle, || line.clone());
        if specs.is_software_emulated {
            tracing::warn!(
                target: "riseonly",
                device = %specs.device_name,
                "frames are drawn on the CPU: no hardware Vulkan or OpenGL driver opened the window, \
                 install the GPU driver (Mesa vulkan-intel or vulkan-radeon, or the NVIDIA driver)"
            );
        }
    }

    fn install_tracing(log_file: Option<RiseLogFile>) {
        let console = tracing_subscriber::fmt::layer().with_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "riseonly=info".into()),
        );
        let file = log_file.map(|log_file| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(log_file)
                .with_filter(tracing_subscriber::EnvFilter::new(LOG_FILE_FILTER))
        });
        tracing_subscriber::registry().with(console).with(file).init();
    }

    fn install_panic_logging(log_file: RiseLogFile) {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!(target: "riseonly::panic", "{info}");
            log_file.flush(LOG_FILE_PANIC_FLUSH);
            previous(info);
        }));
    }

    pub fn main() {
        let log_file = RiseLogFile::start();
        Self::install_tracing(log_file.clone());

        let environment = AppEnvironment::current();
        tracing::warn!(
            target: "riseonly::launch",
            version = %RiseReleaseNotesDeviceContext::app_version(),
            environment = environment.as_str(),
            "launch"
        );
        if let Some(log_file) = log_file {
            Self::install_panic_logging(log_file);
        }
        let endpoints = Endpoints::effective(environment);
        let data_directory = Self::data_directory_for(environment);
        let show_storybook = storybook::requested_by(rise_platform::process_info::arguments());

        let current_directory = std::env::current_dir().ok();
        let arguments = handover::absolute_launch_arguments(
            handover::launch_arguments_from(std::env::args_os(), current_directory.as_deref(), |path| path.is_file()),
            current_directory.as_deref(),
            |path| path.is_file(),
        );
        let primary = if show_storybook {
            None
        } else {
            match single_instance::acquire(&data_directory, &arguments) {
                Ok(Instance::Primary(primary)) => Some(primary),
                Ok(Instance::HandedOff) => {
                    tracing::info!(
                        target: "riseonly",
                        "another instance already owns {}; handed it our arguments",
                        data_directory.display()
                    );
                    return;
                }
                Err(error) => {
                    tracing::warn!(
                        target: "riseonly",
                        "single-instance lock unavailable ({error}); a second launch will not be handed over"
                    );
                    None
                }
            }
        };

        if let Some(rules) = rise_platform::device_locale::week_rules() {
            rise_core::foundation::foundation_calendar::Calendar::install_current_first_weekday(rules.first_weekday);
            rise_core::foundation::foundation_calendar::Calendar::install_current_minimum_days_in_first_week(
                rules.minimum_days_in_first_week,
            );
        }

        let assets = RiseAssets::discover();
        match assets.locales_directory() {
            Some(locales) => rise_i18n::I18n::install_directory(locales),
            None => tracing::error!(
                target: "riseonly",
                "no assets directory found; every string will render as its own key"
            ),
        }

        match assets.load("phone_countries.txt") {
            Ok(Some(bytes)) => rise_ui::phone_input::install_countries(&String::from_utf8_lossy(&bytes)),
            _ => tracing::error!(
                target: "riseonly",
                "no phone_countries.txt; the sign-in field falls back to two countries"
            ),
        }

        tracing::info!(
            target: "riseonly",
            "{} scheme={} data={} assets={:?}",
            endpoints.describe(),
            environment.url_scheme(),
            data_directory.display(),
            assets.root()
        );

        let url_inbox: handover::UrlInbox = handover::UrlInbox::default();
        let mut application = application().with_assets(RiseAssets::discover());

        if let Some(media) = MediaHttpClient::optional(&format!(
            "Riseonly/{} ({})",
            env!("CARGO_PKG_VERSION"),
            environment.display_name()
        )) {
            application = application.with_http_client(media);
        }
        {
            let inbox = url_inbox.clone();
            application.on_open_urls(move |urls| inbox.borrow_mut().extend(urls));
        }
        application.on_reopen(SocketNotificationSource::did_reopen);

        application.run(move |cx: &mut App| {
            crate::install_feature_hooks();
            crate::core::feature_hooks::mark_feature_hooks_installed();
            crate::app::dev_drive::DevDrive::from_launch().install(cx);
            crate::app::perf::rise_perf_log::RisePerfLog::install_from_launch(cx);

            let fonts = rise_ui::fonts::load_bundled_fonts(cx);
            rise_ui::fonts::load_fallback_fonts(cx);
            if !fonts.is_complete() {
                tracing::error!(
                    target: "riseonly",
                    "font faces missing: {:?} rejected: {:?} misresolved: {:?}; \
                     the text stack will draw some weights as another face",
                    fonts.missing,
                    fonts.rejected,
                    fonts.misresolved
                );
            }

            composition::install_storage();
            if show_storybook {
                Self::seed_palette(cx);
            } else {
                Self::init(cx);
            }

            rise_ui::RiseImageCache::install(
                rise_ui::ImageLimits {
                    max_entries: usize::try_from(RiseLimits::client_value("image_render_cache_entries")).unwrap_or(0),
                    max_bytes: u64::try_from(RiseLimits::client_value("image_render_cache_bytes")).unwrap_or(0),
                },
                cx,
            );
            rise_ui::image_ui::render_image_atlas::RenderImageAtlas::install(cx);
            GlassHost::install(rise_platform::materials::current_glass_surface(), cx);
            shell_actions::install_key_bindings(cx);
            system_settings_bridge::install(cx);

            if !show_storybook {
                composition::install(cx);
                composition::did_finish_launching(cx);
                SocketNotificationSource::install(cx);
            }

            cx.set_quit_mode(QuitMode::Explicit);
            SocketNotificationSource::install_last_window_policy(cx);

            let mut activates = true;
            if show_storybook {
                WindowPresenter::open(cx, |_, cx| Storybook::new(cx));
            } else {
                let presentation = TrayController::launch_presentation();
                activates = matches!(presentation, LaunchPresentation::Window);
                WindowPresenter::open_shell_for_launch(presentation, cx);
                Self::report_renderer(cx);
                TrayController::install(cx, handover::raise_app);

                let launch = split_launch_arguments(&arguments);
                if !launch.is_empty() {
                    handover::deliver_arguments(launch, cx);
                }
            }

            handover::serve(primary, url_inbox, cx);

            if activates && !crate::app::dev_drive::DevDrive::is_stealth(cx) {
                cx.activate(true);
            }
        });
    }

    pub fn init(cx: &mut App) {
        Self::install_layout_direction(cx);
        Self::seed_palette(cx);
        LaunchSnapshotTint::install(cx);
        RiseEnergySavingStore::shared(cx);

        HttpService::shared(cx).update(cx, |service, cx| service.configure(cx));
        RiseEngineTraceBridge::install(cx);
        PremiumUpsellStore::install(cx);
        RestrictionStore::install(cx);
        rise_ui::in_window_prompt::install_in_window_prompt(cx);
        cx.defer(RiseRTCLiveKitBootstrap::install);

        SaiWs::set_is_offline_ack_debt_enabled(true);

        SaiWsDeferredOutboundAppHooks::set_on_cold_persisted_replay_finished(
            Some(Rc::new(|cx: &mut App| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_feed(cx));
            })),
            cx,
        );

        let is_launch_splash_presented = RiseLaunchPreferences::shared(cx).read(cx).plays_at_launch();
        let scene = cx.new(|_| Self { is_launch_splash_presented });
        cx.set_global(RiseonlyAppScene(scene));
        RiseSystemTimeZoneBoundary::install(cx);
        RiseTextSizeBoundary::install(cx);

        cx.defer(|cx| {
            LocaleBootstrap::resolve_and_apply(cx);
            EventInteractionsStore::shared(cx).update(cx, |store, cx| store.bootstrap(cx));
            MusicPlayerState::shared(cx);
        });
    }

    /// wraps the root content in layout direction and the launch splash
    /// windows opened outside main (tests) have no launch to cover
    /// no locale override here
    pub fn body(content: AnyElement, cx: &mut App) -> AnyElement {
        let is_rtl = LanguageStore::shared(cx).is_rtl();
        rise_ui::IconUi::set_layout_is_rtl(is_rtl);
        let direction = LayoutDirection::from_is_rtl(is_rtl);
        let scope = env_scope::<LayoutDirectionKey>(direction);
        let Some(scene) = cx.try_global::<RiseonlyAppScene>() else {
            return scope.child(content).into_any_element();
        };
        let is_presented = Binding::entity(
            scene.0.clone(),
            |app: &Self| app.is_launch_splash_presented,
            |app, value, cx| {
                app.is_launch_splash_presented = value;
                cx.refresh_windows();
            },
        );
        scope
            .child(RiseLaunchSplashBoundary::new(LAUNCH_SPLASH_ID, is_presented, content))
            .into_any_element()
    }

    /// stack helpers cant read the environment before layout so they read this copy,
    /// kept equal to the language
    fn install_layout_direction(cx: &mut App) {
        Self::apply_layout_direction(cx);
        cx.observe_global::<LanguageStore>(Self::apply_layout_direction).detach();
    }

    fn apply_layout_direction(cx: &mut App) {
        let direction = LayoutDirection::from_is_rtl(LanguageStore::shared(cx).is_rtl());
        if direction == LayoutDirection::current() {
            return;
        }
        LayoutDirection::install(direction);
        cx.refresh_windows();
    }

    /// unpacks the saved palette before the scene is built,
    /// async version left a fullscreen spinner before the first frame
    fn seed_palette(cx: &mut App) {
        ThemeStore::shared(cx).update(cx, |store, cx| store.ensure_palette_seed(cx));
    }

    fn data_directory_for(environment: AppEnvironment) -> PathBuf {
        let base = directories::ProjectDirs::from("net", "riseonly", "Riseonly")
            .map(|dirs| dirs.data_local_dir().to_path_buf())
            .unwrap_or_else(std::env::temp_dir);

        match environment.data_dir_suffix() {
            "" => base,
            suffix => base.with_file_name(format!(
                "{}{}",
                base.file_name().and_then(|name| name.to_str()).unwrap_or("Riseonly"),
                suffix
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RiseSystemTimeZoneSignal {
    TimeZoneDidChange,
    LocaleDidChange,
    DidBecomeActive,
}

struct RiseSystemTimeZoneBoundary {
    time_zone: String,
}

impl Global for RiseSystemTimeZoneBoundary {}

impl RiseSystemTimeZoneBoundary {
    fn install(cx: &mut App) {
        cx.set_global(Self { time_zone: TimeZone::current().identifier() });

        let (sender, mut signals) = mpsc::unbounded::<RiseSystemTimeZoneSignal>();
        for (name, signal) in [
            (NS_SYSTEM_TIME_ZONE_DID_CHANGE, RiseSystemTimeZoneSignal::TimeZoneDidChange),
            (CURRENT_LOCALE_DID_CHANGE, RiseSystemTimeZoneSignal::LocaleDidChange),
            (UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, RiseSystemTimeZoneSignal::DidBecomeActive),
        ] {
            let sender = sender.clone();
            RiseAppEvents::default_center()
                .observe(name, move |_| {
                    let _ = sender.unbounded_send(signal);
                })
                .detach();
        }

        cx.spawn(async move |cx: &mut AsyncApp| {
            while let Some(signal) = signals.next().await {
                cx.update(|cx| Self::receive(signal, cx));
            }
        })
        .detach();
    }

    fn receive(signal: RiseSystemTimeZoneSignal, cx: &mut App) {
        match signal {
            RiseSystemTimeZoneSignal::TimeZoneDidChange => Self::refresh_time_zone(cx),
            RiseSystemTimeZoneSignal::LocaleDidChange => Self::refresh_clock_cycle(cx),
            RiseSystemTimeZoneSignal::DidBecomeActive => {
                Self::refresh_time_zone(cx);
                Self::refresh_clock_cycle(cx);
            }
        }
    }

    /// 12 or 24 hour switch changes bubble meta width so measured bubbles are dropped
    fn refresh_clock_cycle(cx: &mut App) {
        if !RiseFormat::refresh_device_clock_cycle() {
            return;
        }
        AsyncTextBubblePredisplayCache::shared().remove_all_images();
        AsyncTextBubblePreparedLayoutCache::shared().remove_all();
        RiseMessageFallbackHeightCache::shared(cx).update(cx, |cache, _| cache.remove_all());
        cx.refresh_windows();
    }

    fn refresh_time_zone(cx: &mut App) {
        TimeZone::reset_system();
        let current = TimeZone::current().identifier();
        if cx.global::<Self>().time_zone == current {
            return;
        }
        cx.global_mut::<Self>().time_zone = current;
        cx.refresh_windows();
    }
}

struct RiseTextSizeBoundary {
    text_delta: f32,
}

impl Global for RiseTextSizeBoundary {}

impl RiseTextSizeBoundary {
    fn install(cx: &mut App) {
        let theme_store = ThemeStore::shared(cx);
        let text_delta = theme_store.read(cx).text_delta();
        cx.set_global(Self { text_delta });
        cx.observe(&theme_store, |theme_store, cx| {
            let text_delta = theme_store.read(cx).text_delta();
            if cx.global::<Self>().text_delta.to_bits() == text_delta.to_bits() {
                return;
            }
            cx.global_mut::<Self>().text_delta = text_delta;
            cx.refresh_windows();
        })
        .detach();
    }
}

#[cfg(test)]
#[path = "riseonly_app_tests.rs"]
mod tests;
