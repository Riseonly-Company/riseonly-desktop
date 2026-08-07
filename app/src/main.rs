mod app;
mod core;
// The feature modules are complete domains whose SCREENS arrive later in Phase
// 7: presence is drawn by the chat list and the profile, and half of auth's
// commands are reached from settings. Their APIs are exercised by their own
// tests today and by nothing else, which `-D warnings` reads as dead code.
//
// Scoped here rather than crate-wide, and removed module by module as each one's
// screens land — a feature module that still needs this after its screens exist
// has API nobody wanted.
#[allow(dead_code)]
mod modules;

use gpui::{App, QuitMode};
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

/// Each environment stores its data separately, so switching from staging to
/// production never opens a database written by another backend.
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

    // Before anything opens a window or a database. A desktop app is launched
    // twice constantly — from the dock, from a link, from a file association —
    // and the second launch must hand what it was asked to do to the process
    // that already holds the lock, then leave. The in-memory engine lease
    // cannot see across processes; an advisory file lock can.
    //
    // The storybook is exempt: it opens no database and is a development
    // harness, so contending for the app's lock would only mean it silently
    // exits whenever the app it is meant to be compared against is running.
    //
    // The whole argv, argv[0] included: `single_instance` records a launch
    // verbatim and `handover::routes_from` is the one place that knows the first
    // entry is the executable.
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
                // Not fatal: a read-only or exotic filesystem must not stop the
                // app from starting, it only means a second launch opens a
                // second one.
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

    // The interface language follows the machine. There is no explicit setting
    // to honour yet — when there is, it is passed here and detection never
    // overwrites it, which is the rule the region system is built on.
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

    // Registered before `run`, because that is the only place gpui offers it and
    // because macOS delivers a link to an ALREADY RUNNING app as an Apple Event,
    // not as argv — the single-instance inbox never sees those. The callback has
    // no `App` to talk to, so it parks the links and the serving task drains them.
    let url_inbox: handover::UrlInbox = handover::UrlInbox::default();
    let application = application().with_assets(RiseAssets::discover());
    {
        let inbox = url_inbox.clone();
        application.on_open_urls(move |urls| inbox.borrow_mut().extend(urls));
    }

    application.run(move |cx: &mut App| {
        // Before the first window: adding a face later does not re-shape
        // text that has already been laid out.
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
        GlassHost::install(rise_platform::materials::current_glass_surface(), cx);
        shell_actions::install_key_bindings(cx);

        // Before the first window, so a window opened by a deep link already
        // knows whether anybody is signed in.
        if !show_storybook {
            composition::install(&endpoints, &data_directory, cx);
        }

        // A windowless process would hold the advisory lock with no way to
        // open a window again: the shortcuts live in a window's key context
        // and there is no menu bar yet. A real macOS menu bar plus on_reopen
        // is Phase 8; until then, closing the last window ends the app, as
        // it already does everywhere else.
        cx.set_quit_mode(QuitMode::LastWindowClosed);

        if show_storybook {
            WindowPresenter::open(cx, |_, cx| Storybook::new(cx));
        } else {
            WindowPresenter::open_shell(cx);

            // The launch that STARTED the app carries its own link, and
            // nothing else will ever hand it over — the inbox only holds
            // what LATER launches wrote.
            let routes =
                handover::routes_from(&arguments, environment.url_scheme(), endpoints.web_host());
            if !routes.is_empty() {
                handover::deliver(&routes, cx);
            }
        }

        // The serving task owns the lock, so it is held for exactly as long
        // as the app runs: a released lock means the next launch opens its
        // own window instead of handing its link to nobody.
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
