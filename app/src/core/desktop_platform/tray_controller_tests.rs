use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, TestAppContext};
use rise_platform::autostart::{Autostart, AutostartError, AutostartMechanism, AutostartState, InMemoryAutostart};
use rise_platform::gpui_shim::PlatformSupport;
use rise_platform::host_os::HostOs;
use rise_platform::tray::{
    InMemoryTray, Tray, TrayAvailability, TrayError, TrayEvent, TrayEventHandler, TrayMenuItem,
};
use rise_platform::user_defaults::UserDefaults;

use crate::core::desktop_platform::tray_call_hooks::TrayCallState;
use crate::core::desktop_platform::tray_controller::{
    LaunchPresentation, TrayController, TrayMenuCommand, autostart_change_failed, keeps_in_tray,
    launch_presentation_for, tray_menu_items,
};

thread_local! {
    static ACTIVATIONS: Cell<usize> = const { Cell::new(0) };
}

fn count_activation(_: &mut App) {
    ACTIVATIONS.with(|count| count.set(count.get() + 1));
}

#[derive(Clone)]
struct SharedTray(Rc<RefCell<InMemoryTray>>);

impl SharedTray {
    fn new(availability: TrayAvailability) -> Self {
        Self(Rc::new(RefCell::new(InMemoryTray::with_availability(availability))))
    }

    fn click(&self, command: TrayMenuCommand) {
        assert!(self.0.borrow_mut().send(TrayEvent::Menu(command.id())));
    }

    fn checked(&self, command: TrayMenuCommand) -> Option<bool> {
        self.0
            .borrow()
            .menu()
            .iter()
            .find(|item| item.id == command.id())
            .and_then(|item| item.checked)
    }
}

impl Tray for SharedTray {
    fn set_tooltip(&mut self, text: &str) -> PlatformSupport {
        self.0.borrow_mut().set_tooltip(text)
    }

    fn set_badge(&mut self, count: u32) -> PlatformSupport {
        self.0.borrow_mut().set_badge(count)
    }

    fn set_visible(&mut self, visible: bool) -> PlatformSupport {
        self.0.borrow_mut().set_visible(visible)
    }

    fn availability(&self) -> TrayAvailability {
        self.0.borrow().availability()
    }

    fn set_menu(&mut self, items: Vec<TrayMenuItem>) -> PlatformSupport {
        self.0.borrow_mut().set_menu(items)
    }

    fn set_event_handler(&mut self, handler: TrayEventHandler) -> Result<(), TrayError> {
        self.0.borrow_mut().set_event_handler(handler)
    }
}

fn install(
    cx: &mut TestAppContext,
    host: HostOs,
    tray: &SharedTray,
    autostart: Arc<dyn Autostart>,
    defaults: &UserDefaults,
) {
    let boxed: Box<dyn Tray> = Box::new(tray.clone());
    let defaults = defaults.clone();
    cx.update(|cx| {
        TrayController::install_with(host, boxed, Some(autostart), defaults, count_activation, cx);
    });
    cx.run_until_parked();
}

fn ids(items: &[TrayMenuItem]) -> Vec<u32> {
    items.iter().map(|item| item.id).collect()
}

#[test]
fn the_keep_in_tray_item_is_absent_on_macos_where_the_dock_keeps_the_app() {
    let items = tray_menu_items(
        HostOs::MacOs,
        TrayAvailability::Available,
        Some(AutostartState::Disabled),
        true,
        None,
    );

    assert_eq!(
        ids(&items),
        vec![
            TrayMenuCommand::Open.id(),
            TrayMenuCommand::Microphone.id(),
            TrayMenuCommand::Deafen.id(),
            TrayMenuCommand::LaunchAtLogin.id(),
            TrayMenuCommand::Quit.id(),
        ]
    );
}

#[test]
fn windows_and_linux_offer_every_item_with_its_check_mark() {
    for host in [HostOs::Windows, HostOs::Linux] {
        let items = tray_menu_items(host, TrayAvailability::Available, Some(AutostartState::Enabled), false, None);

        assert_eq!(
            ids(&items),
            vec![
                TrayMenuCommand::Open.id(),
                TrayMenuCommand::Microphone.id(),
                TrayMenuCommand::Deafen.id(),
                TrayMenuCommand::LaunchAtLogin.id(),
                TrayMenuCommand::KeepInTray.id(),
                TrayMenuCommand::Quit.id(),
            ]
        );
        assert_eq!(items[0].checked, None);
        assert_eq!(items[3].checked, Some(true));
        assert_eq!(items[4].checked, Some(false));
        assert_eq!(items[5].checked, None);
    }
}

#[test]
fn no_status_area_means_no_menu() {
    for availability in [TrayAvailability::NoHost, TrayAvailability::Unsupported] {
        let items = tray_menu_items(HostOs::Linux, availability, Some(AutostartState::Enabled), true, None);
        assert!(items.is_empty());
    }
}

#[test]
fn an_os_without_the_setting_gets_no_login_item() {
    let items = tray_menu_items(HostOs::Linux, TrayAvailability::Available, None, true, None);
    assert!(!ids(&items).contains(&TrayMenuCommand::LaunchAtLogin.id()));
}

#[test]
fn awaiting_approval_is_not_shown_as_enabled() {
    let items = tray_menu_items(
        HostOs::MacOs,
        TrayAvailability::Available,
        Some(AutostartState::RequiresApproval),
        true,
        None,
    );
    assert_eq!(items[1].checked, Some(false));
}

#[test]
fn a_menu_id_round_trips_and_an_unknown_one_picks_nothing() {
    for command in [
        TrayMenuCommand::Open,
        TrayMenuCommand::LaunchAtLogin,
        TrayMenuCommand::KeepInTray,
        TrayMenuCommand::Quit,
    ] {
        assert_eq!(TrayMenuCommand::from_id(command.id()), Some(command));
    }
    assert_eq!(TrayMenuCommand::from_id(0), None);
    assert_eq!(TrayMenuCommand::from_id(5), None);
}

#[test]
fn a_start_by_the_person_always_opens_the_window() {
    for host in [HostOs::MacOs, HostOs::Windows, HostOs::Linux] {
        assert_eq!(launch_presentation_for(host, false), LaunchPresentation::Window);
    }
}

#[test]
fn a_login_start_stays_out_of_the_way_on_each_os() {
    assert_eq!(launch_presentation_for(HostOs::MacOs, true), LaunchPresentation::NoWindow);
    assert_eq!(launch_presentation_for(HostOs::Windows, true), LaunchPresentation::HiddenWindow);
    assert_eq!(launch_presentation_for(HostOs::Linux, true), LaunchPresentation::MinimizedWindow);
}

#[test]
fn a_change_that_did_not_take_is_a_failure() {
    let performed = || Ok(PlatformSupport::Performed);
    let enabled = Some(AutostartState::Enabled);
    let disabled = Some(AutostartState::Disabled);

    assert!(!autostart_change_failed(true, &performed(), enabled));
    assert!(!autostart_change_failed(false, &performed(), disabled));
    assert!(autostart_change_failed(true, &performed(), disabled));
    assert!(autostart_change_failed(true, &performed(), Some(AutostartState::RequiresApproval)));
    assert!(autostart_change_failed(true, &performed(), None));
    assert!(autostart_change_failed(true, &Ok(PlatformSupport::Unsupported), enabled));
    assert!(autostart_change_failed(
        true,
        &Err(AutostartError::Unavailable(AutostartMechanism::LoginItemService)),
        enabled,
    ));
}

#[test]
fn keeping_in_tray_is_on_until_turned_off() {
    let defaults = UserDefaults::in_memory();
    assert!(keeps_in_tray(&defaults));

    defaults.set(rise_platform::user_defaults_keys::DESKTOP_KEEP_IN_TRAY, false);
    assert!(!keeps_in_tray(&defaults));
}

#[gpui::test]
fn the_check_mark_follows_the_os_through_a_toggle(cx: &mut TestAppContext) {
    let tray = SharedTray::new(TrayAvailability::Available);
    let autostart: Arc<dyn Autostart> = Arc::new(InMemoryAutostart::new());
    install(cx, HostOs::Linux, &tray, Arc::clone(&autostart), &UserDefaults::in_memory());

    assert_eq!(tray.checked(TrayMenuCommand::LaunchAtLogin), Some(false));

    tray.click(TrayMenuCommand::LaunchAtLogin);
    cx.run_until_parked();

    assert!(matches!(autostart.state(), Ok(AutostartState::Enabled)));
    assert_eq!(tray.checked(TrayMenuCommand::LaunchAtLogin), Some(true));

    tray.click(TrayMenuCommand::LaunchAtLogin);
    cx.run_until_parked();

    assert!(matches!(autostart.state(), Ok(AutostartState::Disabled)));
    assert_eq!(tray.checked(TrayMenuCommand::LaunchAtLogin), Some(false));
}

#[gpui::test]
fn keep_in_tray_decides_what_closing_the_window_does(cx: &mut TestAppContext) {
    let tray = SharedTray::new(TrayAvailability::Available);
    let defaults = UserDefaults::in_memory();
    install(cx, HostOs::Windows, &tray, Arc::new(InMemoryAutostart::new()), &defaults);

    assert!(cx.update(|cx| TrayController::closing_keeps_running_in_tray(cx)));
    assert_eq!(tray.checked(TrayMenuCommand::KeepInTray), Some(true));

    tray.click(TrayMenuCommand::KeepInTray);
    cx.run_until_parked();

    assert!(!keeps_in_tray(&defaults));
    assert_eq!(tray.checked(TrayMenuCommand::KeepInTray), Some(false));
    assert!(!cx.update(|cx| TrayController::closing_keeps_running_in_tray(cx)));
}

#[gpui::test]
fn without_a_status_host_closing_the_window_quits(cx: &mut TestAppContext) {
    let tray = SharedTray::new(TrayAvailability::NoHost);
    install(cx, HostOs::Linux, &tray, Arc::new(InMemoryAutostart::new()), &UserDefaults::in_memory());

    assert!(tray.0.borrow().menu().is_empty());
    assert!(!cx.update(|cx| TrayController::closing_keeps_running_in_tray(cx)));
}

#[gpui::test]
fn the_dock_keeps_a_closed_app_on_macos_not_the_tray(cx: &mut TestAppContext) {
    let tray = SharedTray::new(TrayAvailability::Available);
    install(cx, HostOs::MacOs, &tray, Arc::new(InMemoryAutostart::new()), &UserDefaults::in_memory());

    assert!(!cx.update(|cx| TrayController::closing_keeps_running_in_tray(cx)));
}

#[gpui::test]
fn a_click_and_the_open_item_both_raise_the_window(cx: &mut TestAppContext) {
    ACTIVATIONS.with(|count| count.set(0));
    let tray = SharedTray::new(TrayAvailability::Available);
    install(cx, HostOs::Windows, &tray, Arc::new(InMemoryAutostart::new()), &UserDefaults::in_memory());

    assert!(tray.0.borrow_mut().send(TrayEvent::Activate));
    tray.click(TrayMenuCommand::Open);
    cx.run_until_parked();

    assert_eq!(ACTIVATIONS.with(Cell::get), 2);
}

#[gpui::test]
fn the_icon_opens_with_its_tooltip_from_the_badge_listener(cx: &mut TestAppContext) {
    let tray = SharedTray::new(TrayAvailability::Available);
    install(cx, HostOs::Windows, &tray, Arc::new(InMemoryAutostart::new()), &UserDefaults::in_memory());

    assert!(!tray.0.borrow().tooltip().is_empty());
}

#[test]
fn a_live_call_puts_its_actions_under_open() {
    let call = TrayCallState { microphone_enabled: false, can_use_microphone: true, deafened: true };
    let items = tray_menu_items(HostOs::MacOs, TrayAvailability::Available, None, false, Some(call));
    assert_eq!(
        ids(&items),
        vec![
            TrayMenuCommand::Open.id(),
            TrayMenuCommand::Microphone.id(),
            TrayMenuCommand::Deafen.id(),
            TrayMenuCommand::OpenCall.id(),
            TrayMenuCommand::LeaveCall.id(),
            TrayMenuCommand::Quit.id(),
        ]
    );
    assert_eq!(items[1].checked, Some(false));
    assert_eq!(items[2].checked, Some(true));
    assert!(items.iter().all(|item| item.enabled));
}

#[test]
fn outside_a_call_mic_and_sound_stay_greyed() {
    let items = tray_menu_items(HostOs::MacOs, TrayAvailability::Available, None, false, None);
    assert_eq!(
        ids(&items),
        vec![
            TrayMenuCommand::Open.id(),
            TrayMenuCommand::Microphone.id(),
            TrayMenuCommand::Deafen.id(),
            TrayMenuCommand::Quit.id(),
        ]
    );
    assert!(!items[1].enabled && !items[2].enabled);
    assert!(items[0].enabled && items[3].enabled);
}

#[test]
fn a_silenced_speaker_sees_the_mic_greyed() {
    let call = TrayCallState { microphone_enabled: false, can_use_microphone: false, deafened: false };
    let items = tray_menu_items(HostOs::Linux, TrayAvailability::Available, None, true, Some(call));
    assert_eq!(items[1].id, TrayMenuCommand::Microphone.id());
    assert!(!items[1].enabled);
}
