use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Global, Subscription, Task};
use rise_i18n::language_store::LanguageStore;
use rise_i18n::tr;
use rise_platform::app_badge::{clear_tray_listener, set_tray_listener};
use rise_platform::autostart::{
    Autostart, AutostartConfig, AutostartError, AutostartState, SystemAutostart, launched_at_login,
};
use rise_platform::gpui_shim::PlatformSupport;
use rise_platform::host_os::HostOs;
use rise_platform::tray::{
    self, NativeWindowHandle, Tray, TrayAvailability, TrayConfig, TrayError, TrayEvent, TrayMenuItem,
    TrayService, native_window_handle, tray_tooltip,
};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::DESKTOP_KEEP_IN_TRAY;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, UIApplication};
use crate::core::desktop_platform::rise_app_quit_drain::request_quit;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};

const AUTOSTART_ID: &str = "net.riseonly.desktop";

const HOST_RECHECKS: u32 = 5;
const HOST_RECHECK_INTERVAL: Duration = Duration::from_secs(1);

/// one menu entry, the discriminant is its tray menu item id
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayMenuCommand {
    Open = 1,
    LaunchAtLogin = 2,
    KeepInTray = 3,
    Quit = 4,
}

impl TrayMenuCommand {
    pub const fn id(self) -> u32 {
        self as u32
    }

    pub const fn from_id(id: u32) -> Option<Self> {
        match id {
            1 => Some(Self::Open),
            2 => Some(Self::LaunchAtLogin),
            3 => Some(Self::KeepInTray),
            4 => Some(Self::Quit),
            _ => None,
        }
    }

    const fn label_key(self) -> &'static str {
        match self {
            Self::Open => "ctx_open",
            Self::LaunchAtLogin => "desktop_tray_launch_at_login",
            Self::KeepInTray => "desktop_tray_keep_in_tray",
            Self::Quit => "desktop_menu_quit",
        }
    }
}

/// menu for this state, nothing without a status area
/// autostart is none where the os cant offer it, macos has no keep in tray because the dock does that
pub fn tray_menu_items(
    host: HostOs,
    availability: TrayAvailability,
    autostart: Option<AutostartState>,
    keep_in_tray: bool,
) -> Vec<TrayMenuItem> {
    if availability != TrayAvailability::Available {
        return Vec::new();
    }

    let mut items = vec![TrayMenuItem::plain(
        TrayMenuCommand::Open.id(),
        tr(TrayMenuCommand::Open.label_key()),
    )];

    if let Some(state) = autostart {
        items.push(TrayMenuItem::check(
            TrayMenuCommand::LaunchAtLogin.id(),
            tr(TrayMenuCommand::LaunchAtLogin.label_key()),
            state.is_enabled(),
        ));
    }

    if host != HostOs::MacOs {
        items.push(TrayMenuItem::check(
            TrayMenuCommand::KeepInTray.id(),
            tr(TrayMenuCommand::KeepInTray.label_key()),
            keep_in_tray,
        ));
    }

    items.push(TrayMenuItem::plain(
        TrayMenuCommand::Quit.id(),
        tr(TrayMenuCommand::Quit.label_key()),
    ));
    items
}

/// how the first window of a launch appears
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LaunchPresentation {
    Window,
    HiddenWindow,
    MinimizedWindow,
    NoWindow,
}

pub const fn launch_presentation_for(host: HostOs, launched_at_login: bool) -> LaunchPresentation {
    if !launched_at_login {
        return LaunchPresentation::Window;
    }

    match host {
        HostOs::MacOs => LaunchPresentation::NoWindow,
        HostOs::Windows => LaunchPresentation::HiddenWindow,
        HostOs::Linux => LaunchPresentation::MinimizedWindow,
    }
}

/// true when a login setting change did not take
/// requires approval after enabling counts too since re-registering cant clear it
pub fn autostart_change_failed(
    desired: bool,
    outcome: &Result<PlatformSupport, AutostartError>,
    state: Option<AutostartState>,
) -> bool {
    match (outcome, state) {
        (Ok(PlatformSupport::Performed), Some(state)) => state.is_enabled() != desired,
        _ => true,
    }
}

/// what the person last chose, on until they turn it off
pub fn keeps_in_tray(defaults: &UserDefaults) -> bool {
    defaults
        .object(DESKTOP_KEEP_IN_TRAY)
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

fn autostart_offer(autostart: &dyn Autostart) -> Option<AutostartState> {
    if !autostart.availability().is_supported() {
        return None;
    }

    match autostart.state() {
        Ok(state) => Some(state),
        Err(AutostartError::Unavailable(mechanism)) => {
            tracing::info!(target: "riseonly", "no login start here: {mechanism}");
            None
        }
        Err(error) => {
            tracing::warn!(target: "riseonly", "the login start could not be read: {error}");
            Some(AutostartState::Disabled)
        }
    }
}

fn system_autostart(display_name: &str) -> Option<Arc<dyn Autostart>> {
    let config = AutostartConfig::for_current_process(AUTOSTART_ID, display_name)
        .inspect_err(|error| {
            tracing::warn!(target: "riseonly", "no login start without the executable path: {error}");
        })
        .ok()?
        .with_autostart_launch_argument();

    Some(Arc::new(SystemAutostart::new(config)))
}

fn first_window_handle(cx: &mut App) -> Option<NativeWindowHandle> {
    let window = *cx.windows().first()?;

    window
        .update(cx, |_, window, _| native_window_handle(&*window))
        .ok()
        .flatten()
}

fn spawn_tray_service(service: TrayService) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::warn!(target: "riseonly", "no runtime for the status icon: {error}");
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("rise-tray".to_owned())
        .spawn(move || runtime.block_on(service));

    if let Err(error) = spawned {
        tracing::warn!(target: "riseonly", "no thread for the status icon: {error}");
    }
}

/// the open status icon for the life of the process, absent if it couldnt be opened
pub struct TrayController {
    tray: Rc<RefCell<Box<dyn Tray>>>,
    host: HostOs,
    autostart: Option<Arc<dyn Autostart>>,
    autostart_state: Option<AutostartState>,
    autostart_generation: u64,
    defaults: UserDefaults,
    on_activate: fn(&mut App),
    _events: Task<()>,
    _language: Subscription,
}

impl Global for TrayController {}

impl TrayController {
    pub fn launch_presentation() -> LaunchPresentation {
        launch_presentation_for(HostOs::current(), launched_at_login())
    }

    pub fn install(cx: &mut App, on_activate: fn(&mut App)) {
        if cx.has_global::<Self>() {
            return;
        }

        let host = HostOs::current();
        let name = AppEnvironment::compiled().display_name();
        let mut config = TrayConfig::new(name, tray_tooltip(host, name, 0));

        if let Some(window) = first_window_handle(cx) {
            config = config.with_window(window);
        }

        let tray = match tray::open(config) {
            Ok(tray) => tray,
            Err(error) => {
                tracing::warn!(target: "riseonly", "no status icon: {error}");
                return;
            }
        };

        Self::install_with(
            host,
            tray,
            system_autostart(name),
            UserDefaults::standard(),
            on_activate,
            cx,
        );
    }

    pub(crate) fn install_with(
        host: HostOs,
        mut tray: Box<dyn Tray>,
        autostart: Option<Arc<dyn Autostart>>,
        defaults: UserDefaults,
        on_activate: fn(&mut App),
        cx: &mut App,
    ) {
        let (events, mut inbox) = mpsc::unbounded::<TrayEvent>();
        let routed: Result<(), TrayError> = tray.set_event_handler(Box::new(move |event| {
            let _ = events.unbounded_send(event);
        }));

        if let Err(error) = routed {
            tracing::warn!(target: "riseonly", "the status icon reports no clicks: {error}");
        }

        tray.deliver_events(cx);

        if let Some(service) = tray.take_service() {
            spawn_tray_service(service);
        }

        let event_task = cx.spawn(async move |cx| {
            while let Some(event) = inbox.next().await {
                cx.update(|cx| Self::handle_event(event, cx));
            }
        });

        let language = cx.observe_global::<LanguageStore>(|cx| Self::sync_menu(cx));
        let tray = Rc::new(RefCell::new(tray));
        let badge_tray = Rc::clone(&tray);
        let name = AppEnvironment::compiled().display_name();

        cx.set_global(Self {
            tray,
            host,
            autostart,
            autostart_state: None,
            autostart_generation: 0,
            defaults,
            on_activate,
            _events: event_task,
            _language: language,
        });

        set_tray_listener(move |count| {
            let mut tray = badge_tray.borrow_mut();
            let _ = tray.set_badge(count);
            let _ = tray.set_tooltip(&tray_tooltip(host, name, count));
        });

        Self::observe_activation(cx);

        cx.on_app_quit(|cx| {
            Self::close(cx);
            futures::future::ready(())
        })
        .detach();

        Self::sync_menu(cx);
        Self::refresh_autostart(cx);

        if Self::availability(cx) == Some(TrayAvailability::NoHost) {
            Self::await_status_host(cx);
        }
    }

    /// true if closing the last window should keep the app running in the tray
    /// always false on macos since the dock keeps it
    pub fn closing_keeps_running_in_tray(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|controller| {
            controller.host != HostOs::MacOs
                && controller.tray.borrow().availability() == TrayAvailability::Available
                && keeps_in_tray(&controller.defaults)
        })
    }

    fn availability(cx: &App) -> Option<TrayAvailability> {
        cx.try_global::<Self>()
            .map(|controller| controller.tray.borrow().availability())
    }

    fn close(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }

        clear_tray_listener();
        drop(cx.remove_global::<Self>());
    }

    fn observe_activation(cx: &App) {
        let (signals, mut inbox) = mpsc::unbounded::<()>();

        RiseAppEvents::default_center()
            .observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, move |_| {
                let _ = signals.unbounded_send(());
            })
            .detach();

        cx.spawn(async move |cx| {
            while inbox.next().await.is_some() {
                cx.update(|cx| {
                    Self::sync_menu(cx);
                    Self::refresh_autostart(cx);
                });
            }
        })
        .detach();
    }

    fn await_status_host(cx: &App) {
        cx.spawn(async move |cx| {
            for _ in 0..HOST_RECHECKS {
                cx.background_executor().timer(HOST_RECHECK_INTERVAL).await;

                let settled = cx.update(|cx| {
                    Self::sync_menu(cx);
                    Self::availability(cx) != Some(TrayAvailability::NoHost)
                });

                if settled {
                    return;
                }
            }
        })
        .detach();
    }

    fn sync_menu(cx: &App) {
        let Some(controller) = cx.try_global::<Self>() else {
            return;
        };
        let availability = controller.tray.borrow().availability();
        let items = tray_menu_items(
            controller.host,
            availability,
            controller.autostart_state,
            keeps_in_tray(&controller.defaults),
        );

        if !controller.tray.borrow_mut().set_menu(items).is_supported() {
            tracing::info!(target: "riseonly", "this status icon took no menu");
        }
    }

    fn handle_event(event: TrayEvent, cx: &mut App) {
        match event {
            TrayEvent::Activate => Self::activate(cx),
            TrayEvent::Menu(id) => match TrayMenuCommand::from_id(id) {
                Some(TrayMenuCommand::Open) => Self::activate(cx),
                Some(TrayMenuCommand::LaunchAtLogin) => Self::toggle_autostart(cx),
                Some(TrayMenuCommand::KeepInTray) => Self::toggle_keep_in_tray(cx),
                Some(TrayMenuCommand::Quit) => request_quit(cx),
                None => tracing::warn!(target: "riseonly", "unknown status menu id {id}"),
            },
        }
    }

    fn activate(cx: &mut App) {
        let on_activate = cx.try_global::<Self>().map(|controller| controller.on_activate);

        if let Some(on_activate) = on_activate {
            on_activate(cx);
        }
    }

    fn toggle_keep_in_tray(cx: &mut App) {
        let Some(controller) = cx.try_global::<Self>() else {
            return;
        };

        controller
            .defaults
            .set(DESKTOP_KEEP_IN_TRAY, !keeps_in_tray(&controller.defaults));
        Self::sync_menu(cx);
    }

    fn next_autostart_generation(cx: &mut App) -> u64 {
        let controller = cx.global_mut::<Self>();
        controller.autostart_generation += 1;
        controller.autostart_generation
    }

    fn refresh_autostart(cx: &mut App) {
        let Some(autostart) = cx.try_global::<Self>().and_then(|controller| controller.autostart.clone()) else {
            return;
        };
        let generation = Self::next_autostart_generation(cx);
        let read = cx.background_spawn(async move { autostart_offer(autostart.as_ref()) });

        cx.spawn(async move |cx| {
            let state = read.await;
            cx.update(|cx| Self::publish_autostart(generation, state, cx));
        })
        .detach();
    }

    fn toggle_autostart(cx: &mut App) {
        let Some((autostart, Some(current))) = cx
            .try_global::<Self>()
            .and_then(|controller| controller.autostart.clone().map(|autostart| (autostart, controller.autostart_state)))
        else {
            return;
        };
        let desired = !current.is_enabled();
        let generation = Self::next_autostart_generation(cx);
        let change = cx.background_spawn(async move {
            let outcome = autostart.set_enabled(desired);
            let state = autostart_offer(autostart.as_ref());
            (outcome, state)
        });

        cx.spawn(async move |cx| {
            let (outcome, state) = change.await;
            cx.update(|cx| {
                if let Err(error) = &outcome {
                    tracing::warn!(target: "riseonly", "the login start did not change: {error}");
                }

                if autostart_change_failed(desired, &outcome, state) {
                    Notifier::send(
                        tr("desktop_autostart_error"),
                        NotifierSendOptions {
                            r#type: NotifierType::Error,
                            ..Default::default()
                        },
                        cx,
                    );
                }

                Self::publish_autostart(generation, state, cx);
            });
        })
        .detach();
    }

    fn publish_autostart(generation: u64, state: Option<AutostartState>, cx: &mut App) {
        let Some(controller) = cx.try_global::<Self>() else {
            return;
        };

        if controller.autostart_generation != generation {
            return;
        }

        cx.global_mut::<Self>().autostart_state = state;
        Self::sync_menu(cx);
    }
}
