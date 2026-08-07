//! The status-area icon and the unread badge.
//!
//! gpui has no tray on any of its three backends, so every line of this is ours,
//! including the parts that on other seams would be a one-line forward.
//!
//! The three OSes do not merely spell the same idea differently here. They
//! disagree about whether a number can be drawn at all, about whether the icon
//! needs a window to hang off, and about whether a status area exists in the
//! first place. Those disagreements are why most of this file is pure functions
//! of [`HostOs`]: the decisions are exercised on a Mac, and only the last call
//! into the OS is behind a `cfg`.

use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// The largest unread count shown as a number. Past it the badge collapses, so
/// a client that falls a week behind does not grow a five-digit menu bar item.
///
/// The overflow marker is derived from this rather than written out, because the
/// classic bug in this policy is the cap and the label drifting apart by one.
pub const BADGE_CAP: u32 = 99;

/// What the badge should read, or `None` when nothing should be drawn.
///
/// Zero is `None` and never `Some("0")`. A badge showing zero is worse than no
/// badge: it draws the eye in order to say that nothing happened.
pub fn badge_text(count: u32) -> Option<String> {
    match count {
        0 => None,
        1..=BADGE_CAP => Some(count.to_string()),
        _ => Some(format!("{BADGE_CAP}+")),
    }
}

/// The macOS Dock tile's `badgeLabel`, which follows exactly the same rule.
///
/// Only macOS has an OS-drawn numeric badge. Windows can put an overlay *icon*
/// on the taskbar button and the freedesktop desktops disagree among themselves,
/// so neither gets a label from here — [`tray_tooltip`] is where the count goes
/// on those.
pub fn dock_badge_label(host: HostOs, count: u32) -> Option<String> {
    match host {
        HostOs::MacOs => badge_text(count),
        HostOs::Windows | HostOs::Linux => None,
    }
}

/// The text on the macOS status item's button.
///
/// The item carries no icon asset yet, so this string is the whole of what the
/// user sees, and it must never come back empty: a status item with neither
/// image nor title is a zero-width gap in the menu bar that cannot be clicked,
/// and nothing then tells the user the app is running. The badge is therefore
/// appended to the label rather than replacing it, and the label must not be
/// empty to begin with.
pub fn menu_bar_title(label: &str, count: u32) -> String {
    match badge_text(count) {
        Some(badge) => format!("{label} {badge}"),
        None => label.to_owned(),
    }
}

/// The hover text, with the unread count folded in where the OS cannot draw one.
///
/// On macOS the count is already on the Dock tile and repeating it here is
/// noise. Everywhere else the tooltip is the only surface that can carry a
/// number at all, so leaving it out would lose the count entirely.
pub fn tray_tooltip(host: HostOs, base: &str, count: u32) -> String {
    let composed = match badge_text(count) {
        Some(badge) if tray_mechanism(host).numeric_badge_api.is_none() => {
            format!("{base} ({badge})")
        }
        _ => base.to_owned(),
    };

    truncate_utf16(&composed, tooltip_limit(host))
}

/// The tooltip cap, in UTF-16 code units rather than characters.
///
/// `NOTIFYICONDATAW::szTip` is `[u16; 128]` and the shell reads it until a NUL,
/// so 127 units are usable and the last slot belongs to the terminator. Counting
/// characters is the trap: one emoji is a single `char` and two units, so a
/// 127-character tooltip can be a 254-unit overrun of a fixed buffer.
pub const fn tooltip_limit(host: HostOs) -> Option<usize> {
    match host {
        HostOs::Windows => Some(127),
        HostOs::MacOs | HostOs::Linux => None,
    }
}

/// Cuts at a character boundary, never between the halves of a surrogate pair.
///
/// A `String` cannot hold a lone surrogate, so a cut made by counting code units
/// would have to either panic or corrupt. Stepping whole characters can do
/// neither.
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

/// How a status icon reaches the screen, per OS, as data.
///
/// This exists so all three answers can be asserted from a Mac. Two of the three
/// bindings below have never been compiled, and what is most likely to be wrong
/// about them is not their syntax but the shape of the mechanism: whether there
/// is a badge, whether a window is needed first, whether another process has to
/// be listening. That shape lives here, in one place, and it is tested.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TrayMechanism {
    /// The call that places the icon in the status area.
    pub icon_api: &'static str,
    /// The older protocol to try when `icon_api` finds nothing listening.
    pub fallback_icon_api: Option<&'static str>,
    /// The OS-drawn numeric badge, where one exists at all.
    pub numeric_badge_api: Option<&'static str>,
    /// Whether a window must already exist before the icon can.
    pub attaches_to_a_window: bool,
    /// How far the call has to travel.
    pub transport: TrayTransport,
    /// Whether the status area is guaranteed to be there.
    pub host_guarantee: TrayHostGuarantee,
}

/// How far the call travels, which is what decides whether it can fail for
/// reasons that have nothing to do with this process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayTransport {
    /// A direct call into a framework already linked into this process.
    InProcess,
    /// A message to another process on the session bus, which may not be there.
    DBus,
}

/// Whether the status area is something the OS owns, or something a separate
/// program in the user's session happens to provide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayHostGuarantee {
    /// The OS owns the status area. If the app runs, the icon has a home.
    Guaranteed,
    /// Only the running session can answer. On Linux the status area belongs to
    /// the desktop shell rather than to X11 or Wayland, so a bare session has
    /// none and no amount of build-time configuration changes that.
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

/// Whether this machine has anywhere to put a status icon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayAvailability {
    /// There is a status area and this build can reach it.
    Available,
    /// The OS has the concept but nothing in this session is hosting one: a bare
    /// X11 desktop with no `StatusNotifierHost` registered and no XEmbed tray
    /// either. The app is fine, it simply has no icon, and the user has to be
    /// told that rather than left wondering where it went.
    NoHost,
    /// This build has no binding for this platform's status area.
    Unsupported,
}

/// A native window handle, as a plain integer.
///
/// `HWND` on Windows and ignored elsewhere. Deliberately not the OS type: this
/// crate exists so nothing above it learns which OS it is on, and the caller is
/// only forwarding a number the window system already gave it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NativeWindowHandle(pub u64);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrayConfig {
    /// The macOS status item's button title. Must not be empty; see
    /// [`menu_bar_title`].
    pub label: String,
    /// The hover text, before any unread count is folded into it.
    pub tooltip: String,
    /// Required on Windows and nowhere else, per
    /// [`TrayMechanism::attaches_to_a_window`]: `Shell_NotifyIconW` posts its
    /// callback messages to a window and rejects an icon added with a null
    /// `hWnd`.
    pub window: Option<NativeWindowHandle>,
}

impl TrayConfig {
    pub fn new(label: impl Into<String>, tooltip: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            tooltip: tooltip.into(),
            window: None,
        }
    }

    pub fn with_window(mut self, window: NativeWindowHandle) -> Self {
        self.window = Some(window);
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

/// The status icon, for as long as this value lives.
///
/// Deliberately neither `Send` nor `Sync`, unlike `SecureStore`. The macOS
/// implementation holds AppKit objects and a main-thread token, and AppKit off
/// the main thread is undefined behaviour rather than a lint. Adding the bounds
/// would mean an `unsafe impl Send` that is simply false, so instead the tray
/// stays on the thread that opened it and the compiler enforces that — including
/// for the drop, which is where the icon is removed.
///
/// Every method says whether it did the thing. A tray that silently no-ops is
/// the worst outcome available here: the app goes on believing the user can see
/// the unread count, and stops trying to tell them any other way.
pub trait Tray {
    fn set_tooltip(&mut self, text: &str) -> PlatformSupport;
    fn set_badge(&mut self, count: u32) -> PlatformSupport;
    fn set_visible(&mut self, visible: bool) -> PlatformSupport;
    fn availability(&self) -> TrayAvailability;
}

/// Whether a status icon could be placed, answered without placing one.
///
/// A different question from whether [`open`] will succeed: on Windows the
/// status area is there but `open` still needs a window handle. A caller that
/// wants to offer the user an alternative before the first icon exists asks
/// this one.
pub fn availability() -> TrayAvailability {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        TrayAvailability::Available
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        TrayAvailability::Unsupported
    }
}

/// Places the icon and returns the handle that owns it.
///
/// The value must be kept alive for as long as the icon should exist, and
/// dropped on the thread that created it.
pub fn open(config: TrayConfig) -> Result<Box<dyn Tray>, TrayError> {
    #[cfg(target_os = "macos")]
    {
        macos::open(config)
    }
    #[cfg(target_os = "windows")]
    {
        windows_status_area::open(config)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        freedesktop::open(config)
    }
}

/// Records what it was asked to show instead of drawing it.
///
/// For tests above this crate, and for a case a caller has to handle anyway:
/// [`InMemoryTray::with_availability`] can be built with
/// [`TrayAvailability::NoHost`], which is the shape a Linux session without a
/// status notifier host presents. Code that has only ever run against a working
/// tray is code that has never taken that branch.
pub struct InMemoryTray {
    availability: TrayAvailability,
    tooltip: String,
    badge: Option<String>,
    visible: bool,
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
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSApplication, NSDockTile, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    };
    use objc2_foundation::NSString;

    use super::*;

    /// `objc2-app-kit` 0.3 generates every call used here as a safe `fn`, so
    /// there is no `unsafe` block to justify: the message sends are checked.
    ///
    /// The one invariant that really is unsafe to break — AppKit is main-thread
    /// only — is *not* carried by those signatures for `NSStatusBar`, because
    /// Apple's headers do not mark the class main-thread-only and the generator
    /// only mirrors the headers. The `MainThreadMarker` taken here is what
    /// carries it instead, and it is taken before the first AppKit call rather
    /// than after.
    pub fn open(config: TrayConfig) -> Result<Box<dyn Tray>, TrayError> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Err(TrayError::NotMainThread);
        };

        let bar = NSStatusBar::systemStatusBar();
        let item = bar.statusItemWithLength(NSVariableStatusItemLength);
        let dock = NSApplication::sharedApplication(mtm).dockTile();

        let tray = StatusItemTray {
            mtm,
            bar,
            item,
            dock,
            label: config.label,
            tooltip: config.tooltip,
            unread: 0,
        };

        tray.refresh();
        Ok(Box::new(tray))
    }

    struct StatusItemTray {
        mtm: MainThreadMarker,
        bar: Retained<NSStatusBar>,
        item: Retained<NSStatusItem>,
        dock: Retained<NSDockTile>,
        label: String,
        tooltip: String,
        unread: u32,
    }

    impl StatusItemTray {
        fn refresh(&self) -> PlatformSupport {
            let badge = dock_badge_label(HostOs::MacOs, self.unread);
            let label = badge.map(|text| NSString::from_str(&text));
            self.dock.setBadgeLabel(label.as_deref());

            // The Dock tile is the canonical macOS unread badge and exists as
            // soon as NSApplication does, so the count has already landed by
            // here even when the status item has no button to write to.
            let Some(button) = self.item.button(self.mtm) else {
                return PlatformSupport::Unsupported;
            };

            let title = menu_bar_title(&self.label, self.unread);
            button.setTitle(&NSString::from_str(&title));

            let tooltip = tray_tooltip(HostOs::MacOs, &self.tooltip, self.unread);
            button.setToolTip(Some(&NSString::from_str(&tooltip)));

            PlatformSupport::Performed
        }
    }

    impl Tray for StatusItemTray {
        fn set_tooltip(&mut self, text: &str) -> PlatformSupport {
            self.tooltip = text.to_owned();
            self.refresh()
        }

        fn set_badge(&mut self, count: u32) -> PlatformSupport {
            self.unread = count;
            self.refresh()
        }

        fn set_visible(&mut self, visible: bool) -> PlatformSupport {
            self.item.setVisible(visible);
            PlatformSupport::Performed
        }

        fn availability(&self) -> TrayAvailability {
            TrayAvailability::Available
        }
    }

    /// The status bar owns the item, not this handle. Dropping the `Retained`
    /// alone leaves the icon in the menu bar for the rest of the process, and a
    /// relaunch then shows two.
    impl Drop for StatusItemTray {
        fn drop(&mut self) {
            self.bar.removeStatusItem(&self.item);
        }
    }
}

/// Written blind. Compiled on no machine, run on no machine.
#[cfg(target_os = "windows")]
mod windows_status_area {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell as shell;
    use windows::Win32::UI::WindowsAndMessaging as win;

    use super::*;

    /// Identifies the icon within our window. There is one tray icon, so one id;
    /// it only has to stay the same between the `NIM_ADD` and the `NIM_DELETE`.
    const ICON_ID: u32 = 1;

    pub fn open(config: TrayConfig) -> Result<Box<dyn Tray>, TrayError> {
        let Some(handle) = config.window else {
            return Err(TrayError::NoWindowHandle);
        };

        // SAFETY: a null instance handle with a stock `IDI_*` ordinal is the
        // documented way to ask for a system icon and is valid in any process.
        // The icon that comes back is shared, which is why it is only stored
        // and never destroyed in `Drop`.
        let stock = unsafe { win::LoadIconW(None, win::IDI_APPLICATION) };
        let icon = match stock {
            Ok(icon) => icon,
            Err(error) => return Err(TrayError::Refused(error.to_string())),
        };

        let tray = NotifyIconTray {
            window: HWND(handle.0 as usize as *mut core::ffi::c_void),
            icon,
            tooltip: config.tooltip,
            unread: 0,
            visible: true,
        };

        if !tray.push(shell::NIM_ADD).is_supported() {
            let refusal = "Shell_NotifyIconW refused NIM_ADD".to_owned();
            return Err(TrayError::Refused(refusal));
        }

        Ok(Box::new(tray))
    }

    struct NotifyIconTray {
        window: HWND,
        icon: win::HICON,
        tooltip: String,
        unread: u32,
        visible: bool,
    }

    impl NotifyIconTray {
        fn push(&self, message: shell::NOTIFY_ICON_MESSAGE) -> PlatformSupport {
            let mut data = shell::NOTIFYICONDATAW {
                cbSize: size_of::<shell::NOTIFYICONDATAW>() as u32,
                hWnd: self.window,
                uID: ICON_ID,
                uFlags: shell::NIF_ICON | shell::NIF_TIP | shell::NIF_STATE,
                hIcon: self.icon,
                dwStateMask: shell::NIS_HIDDEN,
                dwState: if self.visible {
                    Default::default()
                } else {
                    shell::NIS_HIDDEN
                },
                ..Default::default()
            };

            // `szTip` is filled through a local and then assigned whole. On
            // 32-bit x86 the struct is `packed(1)`, where borrowing the field in
            // order to slice into it would be a reference to unaligned memory;
            // assigning the array is not. `take` leaves the last slot zeroed,
            // and the shell reads `szTip` until a NUL.
            let mut tip = [0u16; 128];
            let text = tray_tooltip(HostOs::Windows, &self.tooltip, self.unread);
            for (slot, unit) in tip.iter_mut().zip(text.encode_utf16().take(127)) {
                *slot = unit;
            }
            data.szTip = tip;

            // SAFETY: `data` is fully initialised, its `cbSize` is the size of
            // the struct this build compiled against, and it outlives the call —
            // the shell copies what it needs before returning.
            let placed = unsafe { shell::Shell_NotifyIconW(message, &data) };

            if placed.as_bool() {
                PlatformSupport::Performed
            } else {
                PlatformSupport::Unsupported
            }
        }
    }

    impl Tray for NotifyIconTray {
        fn set_tooltip(&mut self, text: &str) -> PlatformSupport {
            self.tooltip = text.to_owned();
            self.push(shell::NIM_MODIFY)
        }

        /// The notification area has no numeric badge, so the count goes into
        /// the tooltip. A drawn badge would be `ITaskbarList3::SetOverlayIcon`
        /// on the taskbar button, which is a different surface and wants a
        /// rendered `HICON` per digit — an asset problem rather than a binding
        /// problem, and not one to solve blind.
        fn set_badge(&mut self, count: u32) -> PlatformSupport {
            self.unread = count;
            self.push(shell::NIM_MODIFY)
        }

        fn set_visible(&mut self, visible: bool) -> PlatformSupport {
            self.visible = visible;
            self.push(shell::NIM_MODIFY)
        }

        fn availability(&self) -> TrayAvailability {
            TrayAvailability::Available
        }
    }

    /// An icon whose `NIM_DELETE` never arrives stays in the notification area
    /// as a ghost until the user happens to hover it, so this is not optional
    /// cleanup.
    impl Drop for NotifyIconTray {
        fn drop(&mut self) {
            let data = shell::NOTIFYICONDATAW {
                cbSize: size_of::<shell::NOTIFYICONDATAW>() as u32,
                hWnd: self.window,
                uID: ICON_ID,
                ..Default::default()
            };

            // SAFETY: same contract as `push`. `NIM_DELETE` reads only `hWnd`
            // and `uID`, both set here.
            let _ = unsafe { shell::Shell_NotifyIconW(shell::NIM_DELETE, &data) };
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod freedesktop {
    use super::{Tray, TrayConfig, TrayError};

    /// Why there is no code here rather than blind code.
    ///
    /// The mechanism is not in doubt. A freedesktop status icon is a
    /// `StatusNotifierItem`: the app exports the `org.kde.StatusNotifierItem`
    /// interface on the session bus, owns a bus name of the form
    /// `org.kde.StatusNotifierItem-<pid>-<n>`, and hands that name to
    /// `org.kde.StatusNotifierWatcher.RegisterStatusNotifierItem`. Tooltip and
    /// visibility are the `ToolTip` and `Status` properties, each with a
    /// `NewToolTip` or `NewStatus` signal to announce the change. There is no
    /// numeric badge anywhere in the specification.
    ///
    /// What is missing is somewhere to run it. That object server has to answer
    /// method calls for the whole life of the process, and this crate owns no
    /// executor. zbus is pinned here with `default-features = false` and only
    /// the `tokio` feature, which leaves `blocking-api` off, so `zbus::blocking`
    /// does not exist; and `rise-platform` does not depend on tokio, so it
    /// cannot start a reactor for the async API either. Giving this crate a
    /// runtime is a decision about the whole crate rather than about the tray,
    /// and making it quietly here would be the wrong place to make it.
    ///
    /// The older XEmbed system tray protocol is not a way around that. It is
    /// exactly the fallback that is absent on the desktops this would be for:
    /// GNOME dropped it, and a bare X11 session with no panel never had it.
    pub fn open(_config: TrayConfig) -> Result<Box<dyn Tray>, TrayError> {
        let reason = "org.kde.StatusNotifierItem needs an executor this crate lacks";
        Err(TrayError::Unsupported(reason))
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
        assert_eq!(badge_text(BADGE_CAP), Some("99".to_owned()));
    }

    #[test]
    fn one_past_the_cap_collapses_to_the_overflow_marker() {
        assert_eq!(badge_text(BADGE_CAP + 1), Some("99+".to_owned()));
        assert_eq!(badge_text(u32::MAX), Some("99+".to_owned()));
    }

    #[test]
    fn the_dock_badge_follows_the_same_rule_as_the_tray_badge() {
        for count in [0, 1, 98, 99, 100, 1_000, u32::MAX] {
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
        for count in [0, 1, 99, 100] {
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
        assert_eq!(menu_bar_title("R", 100), "R 99+");
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
        let expected = if cfg!(any(target_os = "macos", target_os = "windows")) {
            TrayAvailability::Available
        } else {
            TrayAvailability::Unsupported
        };

        assert_eq!(availability(), expected);
    }

    #[test]
    fn the_running_host_names_a_mechanism_of_its_own() {
        assert!(
            !tray_mechanism(HostOs::current()).icon_api.is_empty(),
            "every host must name the call that places its icon"
        );
    }

    /// Skipped rather than asserted if the harness ever runs this on the main
    /// thread, in the same spirit as the keyring probes. The guarantee is that
    /// the refusal comes *before* any AppKit call; creating a real status item
    /// from the test suite is exactly what must not happen.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_status_item_is_refused_off_the_main_thread_rather_than_touching_appkit() {
        if objc2::MainThreadMarker::new().is_some() {
            return;
        }

        assert_eq!(
            open(TrayConfig::new("R", "Riseonly")).err(),
            Some(TrayError::NotMainThread)
        );
    }
}
