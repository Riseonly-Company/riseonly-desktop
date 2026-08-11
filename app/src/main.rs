mod app;
mod core;
#[allow(dead_code)]
mod modules;

use gpui::{App, AssetSource, QuitMode};
use gpui_platform::application;
use rise_platform::single_instance::{self, Instance};
use rise_theme::AppTheme;
use rise_widgets::GlassHost;

use crate::app::composition;
use crate::app::shell::window_presenter::WindowPresenter;
use crate::app::shell::{handover, shell_actions};
use crate::app::storybook::{self, Storybook};
use crate::core::assets::RiseAssets;
use crate::core::config::{AppEnvironment, Endpoints};
use crate::core::media_http::MediaHttpClient;

fn data_directory_for(environment: AppEnvironment) -> std::path::PathBuf {
    let base = directories::ProjectDirs::from("net", "riseonly", "Riseonly")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir);

    match environment.data_dir_suffix() {
        "" => base,
        suffix => base.with_file_name(format!(
            "{}{}",
            base.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Riseonly"),
            suffix
        )),
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "riseonly=info".into()),
        )
        .init();

    let environment = AppEnvironment::compiled();
    let local_override = std::path::Path::new("config/local.json");
    let endpoints = Endpoints::from_process(environment, Some(local_override));
    let data_directory = data_directory_for(environment);
    let show_storybook = storybook::requested_by(std::env::args());

    let arguments: Vec<String> = std::env::args().collect();
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

    let assets = RiseAssets::discover();
    match assets.locales_directory() {
        Some(locales) => rise_i18n::install_directory(locales),
        None => tracing::error!(
            target: "riseonly",
            "no assets directory found; every string will render as its own key"
        ),
    }

    match assets.load("phone_countries.txt") {
        Ok(Some(bytes)) => {
            rise_ui::phone_input::install_countries(&String::from_utf8_lossy(&bytes))
        }
        _ => tracing::error!(
            target: "riseonly",
            "no phone_countries.txt; the sign-in field falls back to two countries"
        ),
    }

    let device = rise_platform::device_locale::current();
    let language = rise_i18n::resolve_interface_language(None, &device.languages);
    rise_i18n::reload(language);
    tracing::info!(
        target: "riseonly",
        "interface language {language} (device: {:?}, region {:?})",
        device.languages,
        device.region
    );

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

    application.run(move |cx: &mut App| {
        // Must precede the first window: a face added later does not re-shape laid-out text.
        let fonts = rise_ui::fonts::load_bundled_fonts(cx);
        if !fonts.is_complete() {
            tracing::error!(
                target: "riseonly",
                "font faces missing: {:?} rejected: {:?} misresolved: {:?} — \
                 the text stack will draw some weights as another face",
                fonts.missing,
                fonts.rejected,
                fonts.misresolved
            );
        }

        rise_ui::install_theme(AppTheme::dark(), cx);
        rise_ui::RiseImageCache::install(rise_ui::ImageLimits::FEED, cx);
        GlassHost::install(rise_platform::materials::current_glass_surface(), cx);
        shell_actions::install_key_bindings(cx);

        if !show_storybook {
            composition::install(&endpoints, &data_directory, cx);
        }

        cx.set_quit_mode(QuitMode::LastWindowClosed);

        if show_storybook {
            WindowPresenter::open(cx, |_, cx| Storybook::new(cx));
        } else {
            WindowPresenter::open_shell(cx);

            let routes =
                handover::routes_from(&arguments, environment.url_scheme(), endpoints.web_host());
            if !routes.is_empty() {
                handover::deliver(&routes, cx);
            }
        }

        handover::serve(
            primary,
            url_inbox,
            environment.url_scheme().to_owned(),
            endpoints.web_host().to_owned(),
            cx,
        );

        cx.activate(true);
    });
}
