use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

pub use crate::app_badge::{BADGE_CAP, badge_text};

/// macos dock badge label, only macos has an os drawn numeric badge
pub fn dock_badge_label(host: HostOs, count: u32) -> Option<String> {
    match host {
        HostOs::MacOs => badge_text(count),
        HostOs::Windows | HostOs::Linux => None,
    }
}

/// title of the macos status item button, never empty since an untitled item cant be clicked
pub fn menu_bar_title(label: &str, count: u32) -> String {
    match badge_text(count) {
        Some(badge) => format!("{label} {badge}"),
        None => label.to_owned(),
    }
}

/// hover text with the unread count added where the os draws no badge
pub fn tray_tooltip(host: HostOs, base: &str, count: u32) -> String {
    let composed = match badge_text(count) {
        Some(badge) if tray_mechanism(host).numeric_badge_api.is_none() => {
            format!("{base} ({badge})")
        }
        _ => base.to_owned(),
    };

    truncate_utf16(&composed, tooltip_limit(host))
}

/// tooltip cap in utf16 units, szTip leaves 127 usable
pub const fn tooltip_limit(host: HostOs) -> Option<usize> {
    match host {
        HostOs::Windows => Some(127),
        HostOs::MacOs | HostOs::Linux => None,
    }
}

fn truncate_utf16(text: &str, limit: Option<usize>) -> String {
    let Some(limit) = limit else {
        return text.to_owned();
    };

    let mut fitted = String::new();
    let mut units = 0usize;

    for character in text.chars() {
        let width = character.len_utf16();
        if units + width > limit {
            break;
        }
        fitted.push(character);
        units += width;
    }

    fitted
}

/// how a status icon reaches the screen on a given os, as data any host can assert
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TrayMechanism {
    pub icon_api: &'static str,
    pub fallback_icon_api: Option<&'static str>,
    pub numeric_badge_api: Option<&'static str>,
    pub attaches_to_a_window: bool,
    pub transport: TrayTransport,
    pub host_guarantee: TrayHostGuarantee,
}

/// how far the call travels, decides if it can fail outside this process
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayTransport {
    InProcess,
    DBus,
}

/// whether the status area belongs to the os or to a program in the session
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayHostGuarantee {
    Guaranteed,
    SessionDependent,
}

pub const fn tray_mechanism(host: HostOs) -> TrayMechanism {
    match host {
        HostOs::MacOs => TrayMechanism {
            icon_api: "NSStatusItem",
            fallback_icon_api: None,
            numeric_badge_api: Some("NSApplication.dockTile.badgeLabel"),
            attaches_to_a_window: false,
            transport: TrayTransport::InProcess,
            host_guarantee: TrayHostGuarantee::Guaranteed,
        },
        HostOs::Windows => TrayMechanism {
            icon_api: "Shell_NotifyIconW",
            fallback_icon_api: None,
            numeric_badge_api: None,
            attaches_to_a_window: true,
            transport: TrayTransport::InProcess,
            host_guarantee: TrayHostGuarantee::Guaranteed,
        },
        HostOs::Linux => TrayMechanism {
            icon_api: "StatusNotifierItem",
            fallback_icon_api: Some("freedesktop System Tray Protocol (XEmbed)"),
            numeric_badge_api: None,
            attaches_to_a_window: false,
            transport: TrayTransport::DBus,
            host_guarantee: TrayHostGuarantee::SessionDependent,
        },
    }
}

/// whether this machine has anywhere to put a status icon
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayAvailability {
    Available,
    NoHost,
    Unsupported,
}

/// native window handle as a plain integer, HWND on windows and ignored elsewhere
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NativeWindowHandle(pub u64);

/// handle for TrayConfig with_window, None on an os whose icon needs no window
pub fn native_window_handle(window: &impl raw_window_handle::HasWindowHandle) -> Option<NativeWindowHandle> {
    let handle = window.window_handle().ok()?;

    match handle.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(win32) => {
            Some(NativeWindowHandle(win32.hwnd.get() as u64))
        }
        _ => None,
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrayConfig {
    pub label: String,
    pub tooltip: String,
    pub window: Option<NativeWindowHandle>,
    pub menu_items: Option<Vec<TrayMenuItem>>,
}

/// one entry of the status icon menu, the app owns the ids and acts on TrayEvent Menu
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrayMenuItem {
    pub id: u32,
    pub label: String,
    pub checked: Option<bool>,
    pub enabled: bool,
}

impl TrayMenuItem {
    pub fn plain(id: u32, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            checked: None,
            enabled: true,
        }
    }

    pub fn check(id: u32, label: impl Into<String>, checked: bool) -> Self {
        Self {
            id,
            label: label.into(),
            checked: Some(checked),
            enabled: true,
        }
    }

    /// shown greyed, a click on it does nothing
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// valid when ids are unique and nonzero 32 bit signed
/// zero means dismissed to TrackPopupMenuEx and root to dbusmenu, dupes cant be told apart on click
pub fn tray_menu_is_valid(items: &[TrayMenuItem]) -> bool {
    items.iter().enumerate().all(|(index, item)| {
        (1..=i32::MAX as u32).contains(&item.id) && items[..index].iter().all(|earlier| earlier.id != item.id)
    })
}

/// the event a popup menu command stands for, zero is a menu closed with no pick
pub const fn tray_menu_pick(command: u32) -> Option<TrayEvent> {
    match command {
        0 => None,
        id => Some(TrayEvent::Menu(id)),
    }
}

/// label as AppendMenuW text, a lone & marks the access key and isnt drawn
/// && draws one
pub fn win32_menu_text(label: &str) -> String {
    label.replace('&', "&&")
}

/// macos status button title, beside an icon only the count and nothing for zero
/// with no icon the label, an untitled imageless item cant be clicked
pub fn status_button_title(has_icon: bool, label: &str, count: u32) -> String {
    if has_icon {
        badge_text(count).unwrap_or_default()
    } else {
        menu_bar_title(label, count)
    }
}

/// what the user asked of the status icon, quitting is a normal TrayMenuItem the app decides on
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayEvent {
    Activate,
    Menu(u32),
}

/// called on the thread that opened the tray, once per user action on the icon
pub type TrayEventHandler = Box<dyn FnMut(TrayEvent) + 'static>;

pub const TRAY_NIN_SELECT: u32 = 0x400;
pub const TRAY_NINF_KEY: u32 = 0x1;
pub const TRAY_NIN_KEYSELECT: u32 = TRAY_NIN_SELECT | TRAY_NINF_KEY;
pub const TRAY_WM_CONTEXTMENU: u32 = 0x7B;
pub const TRAY_WM_LBUTTONUP: u32 = 0x202;
pub const TRAY_WM_RBUTTONUP: u32 = 0x205;

/// what one status icon callback message asks for
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayIconAction {
    Activate,
    Menu { anchor: Option<(i32, i32)> },
}

/// decodes a status icon callback by the encoding the icon was added with
/// version 4 gives the event in LOWORD of lParam and the anchor in wParam, sign extended
/// older ones pass the whole mouse message with no point
pub const fn tray_icon_action(
    version4: bool,
    wparam: usize,
    lparam: isize,
) -> Option<TrayIconAction> {
    let event = (lparam as usize & 0xFFFF) as u32;

    if version4 {
        let x = (wparam & 0xFFFF) as u16 as i16 as i32;
        let y = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
        return match event {
            TRAY_NIN_SELECT | TRAY_NIN_KEYSELECT => Some(TrayIconAction::Activate),
            TRAY_WM_CONTEXTMENU => Some(TrayIconAction::Menu {
                anchor: Some((x, y)),
            }),
            _ => None,
        };
    }

    match event {
        TRAY_WM_LBUTTONUP => Some(TrayIconAction::Activate),
        TRAY_WM_RBUTTONUP => Some(TrayIconAction::Menu { anchor: None }),
        _ => None,
    }
}

impl TrayConfig {
    pub fn new(label: impl Into<String>, tooltip: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            tooltip: tooltip.into(),
            window: None,
            menu_items: None,
        }
    }

    pub fn with_window(mut self, window: NativeWindowHandle) -> Self {
        self.window = Some(window);
        self
    }

    pub fn with_menu_items(mut self, items: Vec<TrayMenuItem>) -> Self {
        self.menu_items = Some(items);
        self
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TrayError {
    #[error("a status icon can only be created on the main thread")]
    NotMainThread,
    #[error("this platform hangs its status icon off a window and none was given")]
    NoWindowHandle,
    #[error("no status icon on this platform: {0}")]
    Unsupported(&'static str),
    #[error("the system refused the status icon: {0}")]
    Refused(String),
}

/// the status icon for as long as this value lives
/// not Send or Sync since it holds appkit objects and drop removes the icon
pub trait Tray {
    fn set_tooltip(&mut self, text: &str) -> PlatformSupport;
    fn set_badge(&mut self, count: u32) -> PlatformSupport;
    fn set_visible(&mut self, visible: bool) -> PlatformSupport;
    fn availability(&self) -> TrayAvailability;

    fn take_service(&mut self) -> Option<TrayService> {
        None
    }

    fn set_menu(&mut self, items: Vec<TrayMenuItem>) -> PlatformSupport {
        drop(items);
        PlatformSupport::Unsupported
    }

    fn set_event_handler(&mut self, handler: TrayEventHandler) -> Result<(), TrayError> {
        drop(handler);
        Err(TrayError::Unsupported("this status icon does not report clicks yet"))
    }

    fn deliver_events(&mut self, app: &gpui::App) {
        let _ = app;
    }
}

/// status icon service that has to run on the engine runtime while the icon shows
pub type TrayService = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub const STATUS_NOTIFIER_ITEM_PATH: &str = "/StatusNotifierItem";

pub const STATUS_NOTIFIER_NO_MENU_PATH: &str = "/NO_DBUSMENU";

/// bus name an item owns, org.kde.StatusNotifierItem-PID-ID per the spec
pub fn status_notifier_bus_name(pid: u32, instance: u32) -> String {
    format!("org.kde.StatusNotifierItem-{pid}-{instance}")
}

/// item Status, a hidden tray is Passive so the host can tuck it away
pub const fn status_notifier_status(visible: bool) -> &'static str {
    if visible {
        "Active"
    } else {
        "Passive"
    }
}

/// what the linux item service learns about org.kde.StatusNotifierWatcher
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusNotifierWatcherEvent {
    WatcherAppeared,
    Registered,
    RegisterFailed,
    WatcherGone,
}

/// service answer to a watcher event
/// if a host draws the item now and if to call RegisterStatusNotifierItem, None keeps the last one
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StatusNotifierStep {
    pub registered: Option<bool>,
    pub register_now: bool,
}

/// object and bus name outlive the watcher, each new watcher owner gets the item registered again
/// tray says NoHost only while none is registered
pub const fn status_notifier_step(event: StatusNotifierWatcherEvent) -> StatusNotifierStep {
    match event {
        StatusNotifierWatcherEvent::WatcherAppeared => StatusNotifierStep {
            registered: None,
            register_now: true,
        },
        StatusNotifierWatcherEvent::Registered => StatusNotifierStep {
            registered: Some(true),
            register_now: false,
        },
        StatusNotifierWatcherEvent::RegisterFailed | StatusNotifierWatcherEvent::WatcherGone => {
            StatusNotifierStep {
                registered: Some(false),
                register_now: false,
            }
        }
    }
}

/// a NameOwnerChanged of the watcher name as an event, empty new owner means it went away
pub const fn status_notifier_owner_event(new_owner_present: bool) -> StatusNotifierWatcherEvent {
    if new_owner_present {
        StatusNotifierWatcherEvent::WatcherAppeared
    } else {
        StatusNotifierWatcherEvent::WatcherGone
    }
}

/// tray availability from the last registration outcome
pub const fn status_notifier_availability(registered: bool) -> TrayAvailability {
    if registered {
        TrayAvailability::Available
    } else {
        TrayAvailability::NoHost
    }
}

/// whether a status icon could be placed, on windows open still needs a window handle
pub fn availability() -> TrayAvailability {
    availability_for(HostOs::current())
}

/// macos and windows always have a status area
/// linux has the binding but only the session knows if a host listens
/// tray says NoHost until a watcher has the item
pub const fn availability_for(host: HostOs) -> TrayAvailability {
    match host {
        HostOs::MacOs | HostOs::Windows | HostOs::Linux => TrayAvailability::Available,
    }
}

/// places the icon, keep the value alive while it shows and drop it on the creating thread
pub fn open(config: TrayConfig) -> Result<Box<dyn Tray>, TrayError> {
    super::os::open(config)
}

/// test tray, records what it was asked to show instead of drawing it
pub struct InMemoryTray {
    availability: TrayAvailability,
    tooltip: String,
    badge: Option<String>,
    visible: bool,
    menu: Vec<TrayMenuItem>,
    handler: Option<TrayEventHandler>,
}

impl InMemoryTray {
    pub fn new() -> Self {
        Self::with_availability(TrayAvailability::Available)
    }

    pub fn with_availability(availability: TrayAvailability) -> Self {
        Self {
            availability,
            tooltip: String::new(),
            badge: None,
            visible: true,
            menu: Vec::new(),
            handler: None,
        }
    }

    pub fn tooltip(&self) -> &str {
        &self.tooltip
    }

    pub fn badge(&self) -> Option<&str> {
        self.badge.as_deref()
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn menu(&self) -> &[TrayMenuItem] {
        &self.menu
    }

    /// plays a user action to the handler, false when none is set
    pub fn send(&mut self, event: TrayEvent) -> bool {
        match self.handler.as_mut() {
            Some(handler) => {
                handler(event);
                true
            }
            None => false,
        }
    }

    fn accepts(&self) -> bool {
        self.availability == TrayAvailability::Available
    }
}

impl Default for InMemoryTray {
    fn default() -> Self {
        Self::new()
    }
}

impl Tray for InMemoryTray {
    fn set_tooltip(&mut self, text: &str) -> PlatformSupport {
        if !self.accepts() {
            return PlatformSupport::Unsupported;
        }
        self.tooltip = text.to_owned();
        PlatformSupport::Performed
    }

    fn set_badge(&mut self, count: u32) -> PlatformSupport {
        if !self.accepts() {
            return PlatformSupport::Unsupported;
        }
        self.badge = badge_text(count);
        PlatformSupport::Performed
    }

    fn set_visible(&mut self, visible: bool) -> PlatformSupport {
        if !self.accepts() {
            return PlatformSupport::Unsupported;
        }
        self.visible = visible;
        PlatformSupport::Performed
    }

    fn availability(&self) -> TrayAvailability {
        self.availability
    }

    fn set_menu(&mut self, items: Vec<TrayMenuItem>) -> PlatformSupport {
        if !self.accepts() || !tray_menu_is_valid(&items) {
            return PlatformSupport::Unsupported;
        }
        self.menu = items;
        PlatformSupport::Performed
    }

    fn set_event_handler(&mut self, handler: TrayEventHandler) -> Result<(), TrayError> {
        self.handler = Some(handler);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_unread_shows_no_badge_at_all() {
        assert_eq!(
            badge_text(0),
            None,
            "a badge that reads zero draws the eye to say nothing happened"
        );
    }

    #[test]
    fn a_single_unread_shows_its_own_digit() {
        assert_eq!(badge_text(1), Some("1".to_owned()));
    }

    #[test]
    fn the_cap_itself_is_still_shown_as_a_number() {
        assert_eq!(badge_text(BADGE_CAP), Some("999".to_owned()));
    }

    #[test]
    fn one_past_the_cap_collapses_to_the_overflow_marker() {
        assert_eq!(badge_text(BADGE_CAP + 1), Some("999+".to_owned()));
        assert_eq!(badge_text(u32::MAX), Some("999+".to_owned()));
    }

    #[test]
    fn the_dock_badge_follows_the_same_rule_as_the_tray_badge() {
        for count in [0, 1, 998, 999, 1_000, u32::MAX] {
            assert_eq!(
                dock_badge_label(HostOs::MacOs, count),
                badge_text(count),
                "the two badges must not drift apart at {count}"
            );
        }
    }

    #[test]
    fn a_dock_badge_is_offered_exactly_where_the_os_can_draw_one() {
        for host in HostOs::ALL {
            assert_eq!(
                dock_badge_label(host, 7).is_some(),
                tray_mechanism(host).numeric_badge_api.is_some(),
                "{host:?} offers a badge label it has no API to draw"
            );
        }
    }

    #[test]
    fn every_host_names_the_call_that_places_its_icon() {
        let icon_api = |host| tray_mechanism(host).icon_api;

        assert_eq!(icon_api(HostOs::MacOs), "NSStatusItem");
        assert_eq!(icon_api(HostOs::Windows), "Shell_NotifyIconW");
        assert_eq!(icon_api(HostOs::Linux), "StatusNotifierItem");

        assert_eq!(
            tray_mechanism(HostOs::MacOs).numeric_badge_api,
            Some("NSApplication.dockTile.badgeLabel")
        );
    }

    #[test]
    fn only_the_linux_status_area_lives_in_another_process() {
        for host in HostOs::ALL {
            let mechanism = tray_mechanism(host);

            assert_eq!(
                mechanism.transport == TrayTransport::DBus,
                host == HostOs::Linux
            );
            assert_eq!(
                mechanism.transport == TrayTransport::DBus,
                mechanism.host_guarantee == TrayHostGuarantee::SessionDependent,
                "{host:?}: a call that leaves the process has no guaranteed listener"
            );
        }
    }

    #[test]
    fn only_linux_has_an_older_protocol_to_fall_back_to() {
        for host in HostOs::ALL {
            assert_eq!(
                tray_mechanism(host).fallback_icon_api.is_some(),
                host == HostOs::Linux
            );
        }
    }

    #[test]
    fn only_windows_needs_a_window_before_it_can_have_an_icon() {
        for host in HostOs::ALL {
            assert_eq!(
                tray_mechanism(host).attaches_to_a_window,
                host == HostOs::Windows
            );
        }
    }

    #[test]
    fn the_unread_count_rides_in_the_tooltip_where_no_badge_can_be_drawn() {
        for host in HostOs::ALL {
            let tooltip = tray_tooltip(host, "Riseonly", 3);

            if tray_mechanism(host).numeric_badge_api.is_some() {
                assert_eq!(
                    tooltip, "Riseonly",
                    "{host:?} already draws the count; repeating it is noise"
                );
            } else {
                assert!(
                    tooltip.contains('3'),
                    "{host:?} has nowhere else for the count, but got {tooltip:?}"
                );
            }
        }
    }

    #[test]
    fn a_tooltip_carries_no_count_when_there_is_nothing_unread() {
        for host in HostOs::ALL {
            assert_eq!(tray_tooltip(host, "Riseonly", 0), "Riseonly");
        }
    }

    #[test]
    fn a_tooltip_is_capped_only_where_the_os_caps_it() {
        assert_eq!(tooltip_limit(HostOs::Windows), Some(127));
        assert_eq!(tooltip_limit(HostOs::MacOs), None);
        assert_eq!(tooltip_limit(HostOs::Linux), None);
    }

    #[test]
    fn an_over_long_tooltip_is_cut_to_fit_the_windows_buffer() {
        let long = "a".repeat(500);
        let capped = tray_tooltip(HostOs::Windows, &long, 0);

        assert_eq!(
            capped.encode_utf16().count(),
            127,
            "szTip is [u16; 128] and the last slot is the terminator"
        );
        assert_eq!(
            tray_tooltip(HostOs::MacOs, &long, 0).len(),
            500,
            "macOS caps nothing, and cutting there would lose text for free"
        );
    }

    #[test]
    fn a_tooltip_is_never_cut_through_a_surrogate_pair() {
        let emoji = "🙂".repeat(200);
        let fitted = tray_tooltip(HostOs::Windows, &emoji, 0);

        assert_eq!(
            fitted.chars().count(),
            63,
            "63 emoji is 126 units; a 64th would be 128 and overrun the buffer"
        );
        assert!(
            emoji.starts_with(&fitted),
            "truncation must drop a suffix, never rewrite what it keeps"
        );
    }

    #[test]
    fn the_badge_never_empties_the_menu_bar_title_and_hides_the_item() {
        for count in [0, 1, 999, 1_000] {
            assert!(
                !menu_bar_title("R", count).is_empty(),
                "an empty title is a zero-width, unclickable gap in the menu bar"
            );
        }
    }

    #[test]
    fn the_menu_bar_title_keeps_the_label_and_appends_the_count() {
        assert_eq!(menu_bar_title("R", 0), "R");
        assert_eq!(menu_bar_title("R", 7), "R 7");
        assert_eq!(menu_bar_title("R", 100), "R 100");
        assert_eq!(menu_bar_title("R", 1_000), "R 999+");
    }

    #[test]
    fn a_recorded_tray_shows_exactly_what_it_was_asked_to() {
        let mut tray = InMemoryTray::new();

        assert!(tray.set_tooltip("Riseonly").is_supported());
        assert!(tray.set_badge(4).is_supported());
        assert!(tray.set_visible(false).is_supported());

        assert_eq!(tray.tooltip(), "Riseonly");
        assert_eq!(tray.badge(), Some("4"));
        assert!(!tray.is_visible());
    }

    #[test]
    fn clearing_the_badge_removes_it_rather_than_showing_a_zero() {
        let mut tray = InMemoryTray::new();
        tray.set_badge(4);
        tray.set_badge(0);

        assert_eq!(tray.badge(), None);
    }

    #[test]
    fn a_tray_with_no_host_refuses_every_call_instead_of_pretending() {
        let mut tray = InMemoryTray::with_availability(TrayAvailability::NoHost);

        assert!(!tray.set_tooltip("Riseonly").is_supported());
        assert!(!tray.set_badge(4).is_supported());
        assert!(!tray.set_visible(false).is_supported());

        assert_eq!(
            tray.badge(),
            None,
            "a call that reported Unsupported must not have recorded a change"
        );
        assert_eq!(tray.availability(), TrayAvailability::NoHost);
    }

    #[test]
    fn availability_reports_honestly_for_this_host() {
        assert_eq!(availability(), availability_for(HostOs::current()));
        assert_eq!(availability_for(HostOs::MacOs), TrayAvailability::Available);
        assert_eq!(availability_for(HostOs::Windows), TrayAvailability::Available);
        assert_eq!(availability_for(HostOs::Linux), TrayAvailability::Available);
    }

    #[test]
    fn a_status_notifier_item_owns_the_name_the_spec_spells_out() {
        assert_eq!(
            status_notifier_bus_name(4242, 1),
            "org.kde.StatusNotifierItem-4242-1"
        );
        assert!(STATUS_NOTIFIER_ITEM_PATH.starts_with('/'));
        assert!(STATUS_NOTIFIER_NO_MENU_PATH.starts_with('/'));
    }

    #[test]
    fn a_hidden_tray_is_passive_rather_than_gone() {
        assert_eq!(status_notifier_status(true), "Active");
        assert_eq!(status_notifier_status(false), "Passive");
    }

    #[test]
    fn the_item_registers_again_whenever_a_watcher_appears() {
        use StatusNotifierWatcherEvent::*;
        assert!(status_notifier_step(WatcherAppeared).register_now);
        assert_eq!(status_notifier_step(RegisterFailed).registered, Some(false));
        assert_eq!(status_notifier_availability(false), TrayAvailability::NoHost);
        assert_eq!(status_notifier_owner_event(true), WatcherAppeared);
        let step = status_notifier_step(status_notifier_owner_event(true));
        assert!(step.register_now);
        assert_eq!(step.registered, None);
        assert_eq!(status_notifier_step(Registered).registered, Some(true));
        assert_eq!(status_notifier_availability(true), TrayAvailability::Available);
        let step = status_notifier_step(status_notifier_owner_event(false));
        assert_eq!(step.registered, Some(false));
        assert!(!step.register_now);
    }

    #[test]
    fn a_config_has_no_menu_until_one_is_given() {
        assert_eq!(TrayConfig::new("R", "Riseonly").menu_items, None);
    }

    #[test]
    fn an_ampersand_in_a_label_stays_visible_in_a_win32_menu() {
        assert_eq!(win32_menu_text("Open"), "Open");
        assert_eq!(win32_menu_text("Q&A & more"), "Q&&A && more");
    }

    /// a branch that overrides none of the optional methods
    struct BareTray;

    impl Tray for BareTray {
        fn set_tooltip(&mut self, _text: &str) -> PlatformSupport {
            PlatformSupport::Performed
        }

        fn set_badge(&mut self, _count: u32) -> PlatformSupport {
            PlatformSupport::Performed
        }

        fn set_visible(&mut self, _visible: bool) -> PlatformSupport {
            PlatformSupport::Performed
        }

        fn availability(&self) -> TrayAvailability {
            TrayAvailability::Available
        }
    }

    #[test]
    fn a_tray_without_click_reporting_says_so_instead_of_dropping_clicks() {
        let refused = BareTray.set_event_handler(Box::new(|_| {}));
        assert!(matches!(refused, Err(TrayError::Unsupported(_))));
    }

    #[test]
    fn a_branch_without_a_menu_says_so_instead_of_dropping_it() {
        let items = vec![TrayMenuItem::plain(1, "Open")];
        assert_eq!(BareTray.set_menu(items), PlatformSupport::Unsupported);
    }

    #[test]
    fn a_menu_id_must_be_nonzero_in_32_bits_and_unique() {
        let plain = |id| TrayMenuItem::plain(id, "x");
        assert!(tray_menu_is_valid(&[]));
        assert!(tray_menu_is_valid(&[plain(1), plain(2), plain(i32::MAX as u32)]));
        assert!(!tray_menu_is_valid(&[plain(0)]), "zero is a dismissed menu and the dbusmenu root");
        assert!(!tray_menu_is_valid(&[plain(i32::MAX as u32 + 1)]));
        assert!(!tray_menu_is_valid(&[plain(1), plain(2), plain(1)]));
    }

    #[test]
    fn a_popup_command_is_the_picked_id_and_zero_picks_nothing() {
        assert_eq!(tray_menu_pick(0), None);
        assert_eq!(tray_menu_pick(7), Some(TrayEvent::Menu(7)));
    }

    #[test]
    fn a_check_item_carries_its_state_and_a_plain_one_has_none() {
        assert_eq!(TrayMenuItem::plain(1, "Open").checked, None);
        assert_eq!(TrayMenuItem::check(2, "Run at login", true).checked, Some(true));
        assert_eq!(TrayMenuItem::check(2, "Run at login", false).checked, Some(false));
    }

    #[test]
    fn the_status_button_shows_only_the_count_beside_its_icon() {
        assert_eq!(status_button_title(true, "R", 0), "");
        assert_eq!(status_button_title(true, "R", 7), "7");
        assert_eq!(status_button_title(true, "R", 1_000), "999+");
        assert_eq!(status_button_title(false, "R", 0), "R", "no icon and no title is an unclickable gap");
        assert_eq!(status_button_title(false, "R", 7), "R 7");
    }

    #[test]
    fn a_config_carries_its_menu_items() {
        let items = vec![TrayMenuItem::plain(1, "Open")];
        let config = TrayConfig::new("R", "Riseonly").with_menu_items(items.clone());
        assert_eq!(config.menu_items, Some(items));
    }

    #[test]
    fn a_recorded_tray_keeps_its_menu_and_plays_events_to_the_handler() {
        let mut tray = InMemoryTray::new();
        assert!(!tray.send(TrayEvent::Activate), "no handler, nothing to deliver to");

        let items = vec![TrayMenuItem::plain(1, "Open"), TrayMenuItem::check(2, "Run at login", true)];
        assert!(tray.set_menu(items.clone()).is_supported());
        assert_eq!(tray.menu(), items.as_slice());

        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = seen.clone();
        tray.set_event_handler(Box::new(move |event| sink.borrow_mut().push(event))).unwrap();
        assert!(tray.send(TrayEvent::Activate));
        assert!(tray.send(TrayEvent::Menu(2)));
        assert_eq!(*seen.borrow(), vec![TrayEvent::Activate, TrayEvent::Menu(2)]);
    }

    #[test]
    fn a_recorded_tray_refuses_an_invalid_menu_or_one_with_no_host() {
        let mut tray = InMemoryTray::new();
        assert!(!tray.set_menu(vec![TrayMenuItem::plain(0, "x")]).is_supported());
        assert!(tray.menu().is_empty());

        let mut hostless = InMemoryTray::with_availability(TrayAvailability::NoHost);
        assert!(!hostless.set_menu(vec![TrayMenuItem::plain(1, "x")]).is_supported());
        assert!(hostless.menu().is_empty());
    }

    #[test]
    fn an_in_process_tray_has_no_service_to_spawn() {
        let mut tray = InMemoryTray::new();
        assert!(tray.take_service().is_none());
    }

    #[test]
    fn the_running_host_names_a_mechanism_of_its_own() {
        assert!(
            !tray_mechanism(HostOs::current()).icon_api.is_empty(),
            "every host must name the call that places its icon"
        );
    }
}
