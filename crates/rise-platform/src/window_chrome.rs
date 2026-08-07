//! The strip along the top of the window: whether the OS draws it or we do,
//! where its buttons sit, and who owns the frame around it.
//!
//! The product shell is Telegram-for-macOS shaped — a vertical rail down the
//! left edge that has to reach the top of the window — so the stock titlebar is
//! suppressed and the strip is ours. That one decision lands differently on each
//! OS, and none of the three is obvious:
//!
//! - macOS hides the titlebar but keeps drawing and hit-testing the traffic
//!   lights, which then float over our client area. We only get to move them.
//! - Windows hides the caption bar outright, buttons included, so the shell has
//!   to paint minimise, maximise and close itself.
//! - Linux has no titlebar to hide. Asking for an app-drawn one is asking the
//!   compositor for client-side decorations, which it may refuse, and which drag
//!   the resize border, the drop shadow and the rounded corners into our lap.
//!
//! So what we asked for and what we were given are two different values here,
//! and both are representable. Everything above the handful of gpui calls at the
//! bottom is a pure function of [`HostOs`].

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// Who draws the strip along the top of the window.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChromeStyle {
    /// The OS draws its stock titlebar, above and outside our client area.
    System,
    /// The stock titlebar is suppressed and the shell lays the strip out itself.
    AppDrawn,
}

impl ChromeStyle {
    /// The shell's own choice, on every platform: the left rail is part of the
    /// window, and a stock titlebar would cut it off at the top.
    pub const fn preferred(host: HostOs) -> Self {
        match host {
            HostOs::MacOs | HostOs::Windows | HostOs::Linux => Self::AppDrawn,
        }
    }

    /// Whether the shell has to reproduce the double-click-the-titlebar gesture.
    ///
    /// Once the strip is our content nothing else will fire it, because the OS
    /// no longer sees a titlebar there. Forwarding it under a system titlebar
    /// instead would act on a gesture the OS has already handled.
    pub const fn app_handles_titlebar_double_click(self) -> bool {
        matches!(self, Self::AppDrawn)
    }
}

/// Which edge of the titlebar the window-control buttons sit against.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ControlSide {
    Leading,
    Trailing,
}

/// The `button-layout` GNOME ships in `org.gnome.desktop.wm.preferences`.
///
/// `appmenu` is not a window button, so what GNOME actually shows by default is
/// a lone close button on the trailing side; minimise and maximise appear only
/// once the user turns them on.
pub const GNOME_DEFAULT_BUTTON_LAYOUT: &str = "appmenu:close";

impl ControlSide {
    /// Which side wins when buttons are present on one, on the other, or on
    /// neither. Leading takes precedence: a desktop that puts anything on the
    /// left is a left-handed desktop, and whatever it also parks on the right is
    /// decoration we do not have to lead with.
    pub const fn from_button_sides(has_leading: bool, has_trailing: bool) -> Option<Self> {
        if has_leading {
            Some(Self::Leading)
        } else if has_trailing {
            Some(Self::Trailing)
        } else {
            None
        }
    }

    /// Parses a GNOME-style `button-layout` string, e.g. GNOME's own
    /// `appmenu:close` or elementary's `close:maximize`.
    ///
    /// gpui has this parser too, but it is compiled only on Linux and FreeBSD,
    /// so the policy cannot call it and could not be tested through it. The two
    /// rules that are easy to get wrong are reproduced here: a string with no
    /// colon is entirely the *trailing* side, and a token that is not a window
    /// button (`appmenu`, `icon`, `menu`, `spacer`) claims no side at all —
    /// which is exactly why GNOME's default reads as trailing and not leading.
    ///
    /// `None` means the string named no window button, and the caller should
    /// fall back to [`window_control_side`] rather than guess.
    pub fn from_desktop_layout(layout: &str) -> Option<Self> {
        fn has_button(side: &str) -> bool {
            side.split(',')
                .map(str::trim)
                .any(|name| matches!(name, "minimize" | "maximize" | "close"))
        }

        let (leading, trailing) = layout.split_once(':').unwrap_or(("", layout));
        Self::from_button_sides(has_button(leading), has_button(trailing))
    }
}

/// macOS puts the traffic lights on the leading edge and Windows puts
/// minimise/maximise/close on the trailing one; neither is configurable.
///
/// Linux is whatever the desktop environment says, so this is only the fallback
/// for when nothing has told us yet. It matches GNOME's shipped layout — see
/// [`GNOME_DEFAULT_BUTTON_LAYOUT`] — because guessing leading on a GNOME session
/// would stack our own buttons underneath the compositor's.
pub const fn window_control_side(host: HostOs) -> ControlSide {
    match host {
        HostOs::MacOs => ControlSide::Leading,
        HostOs::Windows | HostOs::Linux => ControlSide::Trailing,
    }
}

/// Whether the frame around the window belongs to the compositor or to us.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DecorationMode {
    /// The OS draws the border, the shadow and the corners outside our surface.
    ServerSide,
    /// Nothing outside our surface is drawn for us.
    ClientSide,
}

/// Width of the invisible margin outside the visible window that has to stay
/// grabbable for edge resizing under client-side decorations. GTK's own resize
/// handle is about this wide; conventional rather than measured.
const CLIENT_SIDE_RESIZE_BORDER: f32 = 10.0;

/// Corner radius the app paints under client-side decorations, matching
/// libadwaita's window rounding. Conventional rather than measured.
const CLIENT_SIDE_CORNER_RADIUS: f32 = 12.0;

impl DecorationMode {
    /// Under client-side decorations the compositor draws nothing outside our
    /// surface, so the resize border, the drop shadow and the rounded corners
    /// are all ours to paint *and* to hit-test. Under server-side they are the
    /// compositor's, and painting our own on top of them only produces a second
    /// rounding inside the first.
    pub const fn app_draws_window_frame(self) -> bool {
        matches!(self, Self::ClientSide)
    }

    /// Logical pixels to reserve outside the visible window for edge resizing.
    /// This is the value [`apply`] hands to `Window::set_client_inset`.
    pub const fn resize_border(self) -> f32 {
        match self {
            Self::ServerSide => 0.0,
            Self::ClientSide => CLIENT_SIDE_RESIZE_BORDER,
        }
    }

    /// Logical pixels of corner rounding the app has to paint itself.
    pub const fn corner_radius(self) -> f32 {
        match self {
            Self::ServerSide => 0.0,
            Self::ClientSide => CLIENT_SIDE_CORNER_RADIUS,
        }
    }

    /// What gpui reports the window is actually configured as. `Decorations`
    /// also carries the edge-tiling state, which only the corner painter cares
    /// about and is deliberately dropped here.
    pub const fn from_gpui(decorations: gpui::Decorations) -> Self {
        match decorations {
            gpui::Decorations::Server => Self::ServerSide,
            gpui::Decorations::Client { .. } => Self::ClientSide,
        }
    }

    /// What we ask gpui for, at window creation and again through
    /// `Window::request_decorations`.
    pub const fn into_gpui(self) -> gpui::WindowDecorations {
        match self {
            Self::ServerSide => gpui::WindowDecorations::Server,
            Self::ClientSide => gpui::WindowDecorations::Client,
        }
    }
}

/// macOS keeps the frame whatever we do to the titlebar: `appears_transparent`
/// makes the strip see-through, it does not hand us the window border, and
/// gpui's macOS backend never overrides `window_decorations`. Windows is the
/// same. Linux is the only platform where the question is open, and there the
/// compositor answers it — see [`effective_style`].
pub const fn requested_decorations(host: HostOs, style: ChromeStyle) -> DecorationMode {
    match host {
        HostOs::MacOs | HostOs::Windows => DecorationMode::ServerSide,
        HostOs::Linux => match style {
            ChromeStyle::System => DecorationMode::ServerSide,
            ChromeStyle::AppDrawn => DecorationMode::ClientSide,
        },
    }
}

/// The style actually in force once the platform has answered.
///
/// Only Linux can disagree, and it disagrees in both directions. A compositor
/// with no `xdg-decoration` support keeps drawing its own titlebar above our
/// surface, so an app-drawn strip would leave the user looking at two. A GNOME
/// Wayland session does the reverse and hands us client-side decorations whether
/// we asked or not, so there is no system titlebar left to fall back to. On
/// Linux the granted decoration mode *is* the style; the request only survives
/// where it was never in question.
pub const fn effective_style(
    host: HostOs,
    requested: ChromeStyle,
    granted: DecorationMode,
) -> ChromeStyle {
    match host {
        HostOs::MacOs | HostOs::Windows => requested,
        HostOs::Linux => match granted {
            DecorationMode::ServerSide => ChromeStyle::System,
            DecorationMode::ClientSide => ChromeStyle::AppDrawn,
        },
    }
}

/// Height of one platform's titlebar, in logical pixels.
///
/// - macOS 28: the standard AppKit titlebar of a regular-size `.titled` window.
/// - Windows 32: Microsoft's title bar guidance sizes the caption buttons
///   46x32 dip, so the caption strip is 32 tall.
/// - Linux 37: GNOME's Adwaita headerbar minimum height.
pub const fn titlebar_height(host: HostOs) -> f32 {
    match host {
        HostOs::MacOs => 28.0,
        HostOs::Windows => 32.0,
        HostOs::Linux => 37.0,
    }
}

/// The number of controls a platform shows unless the desktop says otherwise:
/// close, minimise and maximise — zoom, on macOS.
pub const FULL_CONTROL_SET: usize = 3;

/// Geometry of one platform's window-control buttons, in logical pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ControlLayout {
    pub side: ControlSide,
    pub button_width: f32,
    pub button_height: f32,
    /// Gap between two adjacent buttons. Zero on Windows, where the caption
    /// buttons are flush hit targets rather than separated glyphs.
    pub gap: f32,
    /// Space between the window edge and the outermost button, applied at both
    /// ends of the group so the strip does not look bolted to the frame.
    pub edge_padding: f32,
}

impl ControlLayout {
    /// The macOS numbers are the stock traffic lights — 14x14 frames 6 apart,
    /// the group starting 20 in from the leading edge — and the Linux ones are
    /// Adwaita's round 24pt titlebuttons with headerbar padding. Both sets are
    /// read off a standard window rather than taken from a published constant,
    /// so treat them as conventional. The Windows 46x32 is Microsoft's own
    /// documented caption-button size.
    pub const fn for_host(host: HostOs) -> Self {
        match host {
            HostOs::MacOs => Self {
                side: ControlSide::Leading,
                button_width: 14.0,
                button_height: 14.0,
                gap: 6.0,
                edge_padding: 20.0,
            },
            HostOs::Windows => Self {
                side: ControlSide::Trailing,
                button_width: 46.0,
                button_height: 32.0,
                gap: 0.0,
                edge_padding: 0.0,
            },
            HostOs::Linux => Self {
                side: ControlSide::Trailing,
                button_width: 24.0,
                button_height: 24.0,
                gap: 6.0,
                edge_padding: 6.0,
            },
        }
    }

    /// Room the controls need, measured from [`ControlLayout::side`].
    ///
    /// It takes a count because Linux is where the count varies: GNOME ships
    /// close alone and the user may add minimise and maximise, so a fixed
    /// three-button budget would leave a hole in the middle of the strip. On
    /// macOS and Windows the count is always [`FULL_CONTROL_SET`].
    pub fn reserved_inset(&self, buttons: usize) -> f32 {
        if buttons == 0 {
            return 0.0;
        }
        let count = buttons as f32;
        self.edge_padding * 2.0 + count * self.button_width + (count - 1.0) * self.gap
    }
}

/// Where the macOS traffic lights go inside a titlebar we drew ourselves.
///
/// gpui's macOS backend treats the vertical value as *symmetric* padding: it
/// resizes the AppKit titlebar container to `button height + 2 * padding` and
/// then places the buttons at that offset. So the buttons are centred in a strip
/// of a given height only when the padding is half of what is left over, and
/// reusing a value picked for a 28pt bar in a taller one silently leaves them
/// riding high.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TrafficLightOrigin {
    /// Inset from the window's leading edge to the close button's frame.
    pub leading: f32,
    /// Inset from both the top and the bottom of the strip.
    pub vertical_padding: f32,
}

impl TrafficLightOrigin {
    /// Centres the group vertically in a strip of `height`.
    pub fn centred_in(layout: &ControlLayout, height: f32) -> Self {
        Self {
            leading: layout.edge_padding,
            vertical_padding: ((height - layout.button_height) / 2.0).max(0.0),
        }
    }

    /// The strip height this origin implies, which is what AppKit will actually
    /// give the titlebar container.
    pub fn implied_titlebar_height(&self, layout: &ControlLayout) -> f32 {
        layout.button_height + self.vertical_padding * 2.0
    }
}

/// One platform's whole chrome decision, as a value the shell can hold without
/// ever learning which platform it is on.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WindowChrome {
    pub style: ChromeStyle,
    /// Under [`ChromeStyle::AppDrawn`] this is the row the shell lays out
    /// itself. Under [`ChromeStyle::System`] it is the OS's own strip, which
    /// sits above the client area and costs the shell's layout nothing.
    pub titlebar_height: f32,
    pub controls: ControlLayout,
    /// Whether the shell has to paint the buttons. False on macOS even with the
    /// titlebar suppressed, because AppKit goes on drawing and hit-testing the
    /// traffic lights over our content; painting our own there would double them.
    pub controls_drawn_by_app: bool,
    /// Logical pixels at `controls.side` that app content must not enter. Zero
    /// under a system titlebar, where the buttons are not in our area at all.
    pub reserved_inset: f32,
    pub decorations: DecorationMode,
    /// Only macOS, and only with an app-drawn titlebar: the OS buttons exist and
    /// have to be moved into our strip.
    pub traffic_lights: Option<TrafficLightOrigin>,
}

impl WindowChrome {
    pub fn resolve(host: HostOs, style: ChromeStyle) -> Self {
        let controls = ControlLayout::for_host(host);
        let height = titlebar_height(host);
        let app_drawn = matches!(style, ChromeStyle::AppDrawn);

        let reserved_inset = if app_drawn {
            controls.reserved_inset(FULL_CONTROL_SET)
        } else {
            0.0
        };

        let traffic_lights = if app_drawn && matches!(host, HostOs::MacOs) {
            Some(TrafficLightOrigin::centred_in(&controls, height))
        } else {
            None
        };

        Self {
            style,
            titlebar_height: height,
            controls,
            controls_drawn_by_app: app_drawn && !matches!(host, HostOs::MacOs),
            reserved_inset,
            decorations: requested_decorations(host, style),
            traffic_lights,
        }
    }

    /// The shell's own choice for this platform.
    pub fn preferred(host: HostOs) -> Self {
        Self::resolve(host, ChromeStyle::preferred(host))
    }

    /// Re-lays the strip at a different height, keeping the traffic lights
    /// centred in it. A taller strip is the reason this exists: the rail's first
    /// row sets the height, not the OS.
    pub fn with_titlebar_height(mut self, height: f32) -> Self {
        self.titlebar_height = height;
        if self.traffic_lights.is_some() {
            let origin = TrafficLightOrigin::centred_in(&self.controls, height);
            self.traffic_lights = Some(origin);
        }
        self
    }

    /// Padding the shell adds at the leading edge of its titlebar row.
    pub fn leading_inset(&self) -> f32 {
        match self.controls.side {
            ControlSide::Leading => self.reserved_inset,
            ControlSide::Trailing => 0.0,
        }
    }

    /// Padding the shell adds at the trailing edge of its titlebar row.
    pub fn trailing_inset(&self) -> f32 {
        match self.controls.side {
            ControlSide::Leading => 0.0,
            ControlSide::Trailing => self.reserved_inset,
        }
    }
}

/// What the platform granted, as distinct from what [`WindowChrome`] asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GrantedChrome {
    pub decorations: DecorationMode,
    /// Whether the traffic lights were moved into our strip. macOS only —
    /// `Window::set_traffic_light_position` does not exist elsewhere.
    pub traffic_lights: PlatformSupport,
    /// Whether the invisible resize margin was registered. Only reachable under
    /// client-side decorations: gpui's macOS and Windows backends leave
    /// `set_client_inset` at its empty default, so reporting Performed there
    /// would be a lie.
    pub client_inset: PlatformSupport,
}

impl GrantedChrome {
    pub fn matches_request(&self, chrome: &WindowChrome) -> bool {
        self.decorations == chrome.decorations
    }
}

/// The titlebar half of `gpui::WindowOptions`.
///
/// `appears_transparent` is the macOS and Windows switch only. Linux's
/// equivalent is `WindowOptions::window_decorations`, which takes
/// [`DecorationMode::into_gpui`]. Both have to be set from the same
/// [`WindowChrome`], or the two families of platform disagree about what was
/// even asked for.
pub fn titlebar_options(chrome: &WindowChrome, title: &str) -> gpui::TitlebarOptions {
    gpui::TitlebarOptions {
        title: Some(title.into()),
        appears_transparent: matches!(chrome.style, ChromeStyle::AppDrawn),
        traffic_light_position: chrome.traffic_lights.map(traffic_light_point),
    }
}

fn traffic_light_point(origin: TrafficLightOrigin) -> gpui::Point<gpui::Pixels> {
    gpui::point(gpui::px(origin.leading), gpui::px(origin.vertical_padding))
}

/// Applies `chrome` to an already-open window and reports what came back.
///
/// The read-back is not a formality, and it is not uniformly trustworthy. On X11
/// gpui downgrades a client-side request to server-side on the spot when no
/// compositor is running, so the answer here is true immediately. On Wayland it
/// is optimistic: the request is recorded as if granted, and the compositor only
/// confirms or contradicts it on the next `configure`. A shell that stores this
/// value and never looks again therefore paints a Wayland window wrong for as
/// long as it lives — re-read with [`granted_decorations`] instead of caching.
pub fn apply(window: &mut gpui::Window, chrome: &WindowChrome) -> GrantedChrome {
    window.request_decorations(chrome.decorations.into_gpui());

    let traffic_lights = place_traffic_lights(window, chrome);
    let decorations = granted_decorations(window);

    let client_inset = if decorations.app_draws_window_frame() {
        window.set_client_inset(gpui::px(decorations.resize_border()));
        PlatformSupport::Performed
    } else {
        PlatformSupport::Unsupported
    };

    GrantedChrome {
        decorations,
        traffic_lights,
        client_inset,
    }
}

/// What the window is configured as right now. Cheap; call it per frame rather
/// than trusting a value captured at window creation.
pub fn granted_decorations(window: &gpui::Window) -> DecorationMode {
    DecorationMode::from_gpui(window.window_decorations())
}

fn place_traffic_lights(window: &gpui::Window, chrome: &WindowChrome) -> PlatformSupport {
    #[cfg(target_os = "macos")]
    {
        match chrome.traffic_lights {
            Some(origin) => {
                window.set_traffic_light_position(traffic_light_point(origin));
                PlatformSupport::Performed
            }
            None => PlatformSupport::Unsupported,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, chrome);
        PlatformSupport::Unsupported
    }
}

/// Which window operations the platform actually offers.
///
/// An app-drawn titlebar that paints a maximise button on a platform that cannot
/// maximise hands the user a dead control, and there is no way to know that
/// without asking the open window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AvailableControls {
    pub minimize: bool,
    pub maximize: bool,
    pub fullscreen: bool,
    pub window_menu: bool,
}

pub fn available_controls(window: &gpui::Window) -> AvailableControls {
    let controls = window.window_controls();
    AvailableControls {
        minimize: controls.minimize,
        maximize: controls.maximize,
        fullscreen: controls.fullscreen,
        window_menu: controls.window_menu,
    }
}

/// The side the desktop environment puts its window buttons on.
///
/// gpui answers this only on Linux, where it comes from the XDG settings portal;
/// macOS and Windows answer `None` and fall through to [`window_control_side`].
/// Until the portal replies gpui reports its own built-in default, which is all
/// three buttons trailing — the same side as GNOME's, so a session that never
/// answers still lands somewhere the user recognises.
pub fn desktop_control_side(app: &gpui::App, host: HostOs) -> ControlSide {
    app.button_layout()
        .and_then(|layout| {
            let leading = layout.left.iter().any(Option::is_some);
            let trailing = layout.right.iter().any(Option::is_some);
            ControlSide::from_button_sides(leading, trailing)
        })
        .unwrap_or_else(|| window_control_side(host))
}

/// Whether forwarding a titlebar double click does anything on this host.
///
/// `Window::titlebar_double_click` is an empty default on gpui's `PlatformWindow`
/// trait and only `gpui_macos` overrides it, so on Windows and Linux the call
/// compiles, runs, and does nothing. Reporting success there would leave the
/// gesture silently dead — the caller has to know to handle it itself.
pub const fn forwards_titlebar_double_click(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs => PlatformSupport::Performed,
        HostOs::Windows | HostOs::Linux => PlatformSupport::Unsupported,
    }
}

/// Forwards a double click on empty titlebar space to the OS.
///
/// The gesture is a user preference rather than a fixed action — macOS reads
/// `AppleActionOnDoubleClick`, which can be zoom, minimise or nothing — and gpui
/// resolves it for us. Reports Unsupported under a system titlebar, where the OS
/// has already acted on the click and forwarding it would act twice, and on the
/// hosts where gpui has no implementation to forward to.
pub fn titlebar_double_click(window: &gpui::Window, chrome: &WindowChrome) -> PlatformSupport {
    if !chrome.style.app_handles_titlebar_double_click() {
        return PlatformSupport::Unsupported;
    }

    if !forwards_titlebar_double_click(HostOs::current()).is_supported() {
        return PlatformSupport::Unsupported;
    }

    window.titlebar_double_click();
    PlatformSupport::Performed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gpui's `titlebar_double_click` is an empty default on the platform trait
    /// and only `gpui_macos` overrides it. Claiming Performed on the other two
    /// would be the exact failure this crate exists to prevent: a call that
    /// compiles everywhere and quietly does nothing on two thirds of the targets.
    #[test]
    fn only_macos_can_hand_a_titlebar_double_click_back_to_the_os() {
        assert!(forwards_titlebar_double_click(HostOs::MacOs).is_supported());
        assert!(!forwards_titlebar_double_click(HostOs::Windows).is_supported());
        assert!(!forwards_titlebar_double_click(HostOs::Linux).is_supported());
    }

    #[test]
    fn a_system_titlebar_never_forwards_the_double_click_twice() {
        for host in HostOs::ALL {
            assert!(
                !ChromeStyle::System.app_handles_titlebar_double_click(),
                "the OS already acted on the click on {host:?}"
            );
        }
    }

    #[test]
    fn every_platform_draws_its_own_titlebar_by_default() {
        for host in HostOs::ALL {
            assert_eq!(
                ChromeStyle::preferred(host),
                ChromeStyle::AppDrawn,
                "the left rail must reach the top of the window on {host:?}"
            );
        }
    }

    #[test]
    fn the_controls_lead_on_macos_and_trail_on_windows_and_linux() {
        assert_eq!(window_control_side(HostOs::MacOs), ControlSide::Leading);
        assert_eq!(window_control_side(HostOs::Windows), ControlSide::Trailing);
        assert_eq!(window_control_side(HostOs::Linux), ControlSide::Trailing);
    }

    #[test]
    fn our_linux_fallback_is_the_side_gnome_actually_ships() {
        let gnome = ControlSide::from_desktop_layout(GNOME_DEFAULT_BUTTON_LAYOUT);
        assert_eq!(
            gnome,
            Some(window_control_side(HostOs::Linux)),
            "guessing the wrong side stacks our buttons under the compositor's"
        );
    }

    #[test]
    fn appmenu_is_not_a_window_button_and_claims_no_side() {
        let with_close = ControlSide::from_desktop_layout("appmenu:close");
        assert_eq!(with_close, Some(ControlSide::Trailing));
        assert_eq!(ControlSide::from_desktop_layout("appmenu:"), None);
    }

    #[test]
    fn a_desktop_that_puts_the_buttons_on_the_left_is_honoured() {
        let elementary = ControlSide::from_desktop_layout("close:maximize");
        assert_eq!(
            elementary,
            Some(ControlSide::Leading),
            "elementary puts close on the left and we have to follow"
        );

        let all_left = ControlSide::from_desktop_layout("close,minimize,maximize:");
        assert_eq!(all_left, Some(ControlSide::Leading));
    }

    #[test]
    fn a_layout_string_without_a_colon_is_read_as_the_trailing_side() {
        let bare = ControlSide::from_desktop_layout("minimize,maximize,close");
        assert_eq!(bare, Some(ControlSide::Trailing));
    }

    #[test]
    fn a_layout_naming_no_window_button_leaves_the_side_undecided() {
        for layout in ["", ":", "appmenu:spacer", "garbage", "icon:menu"] {
            assert_eq!(
                ControlSide::from_desktop_layout(layout),
                None,
                "{layout:?} places nothing, so the caller has to fall back"
            );
        }
    }

    #[test]
    fn whitespace_around_a_button_name_does_not_hide_it() {
        let padded = ControlSide::from_desktop_layout(" close , minimize : maximize ");
        assert_eq!(padded, Some(ControlSide::Leading));
    }

    #[test]
    fn an_app_drawn_titlebar_reserves_room_for_the_controls_everywhere() {
        for host in HostOs::ALL {
            let chrome = WindowChrome::resolve(host, ChromeStyle::AppDrawn);
            let buttons = chrome.controls.button_width * 3.0;
            assert!(
                chrome.reserved_inset >= buttons,
                "{host:?} would draw content under its own window buttons"
            );
        }
    }

    #[test]
    fn a_system_titlebar_reserves_nothing_inside_the_client_area() {
        for host in HostOs::ALL {
            let chrome = WindowChrome::resolve(host, ChromeStyle::System);
            assert_eq!(chrome.reserved_inset, 0.0, "{host:?}");
            assert_eq!(chrome.leading_inset(), 0.0, "{host:?}");
            assert_eq!(chrome.trailing_inset(), 0.0, "{host:?}");
        }
    }

    #[test]
    fn the_reserved_room_is_on_the_side_the_controls_are() {
        let mac = WindowChrome::resolve(HostOs::MacOs, ChromeStyle::AppDrawn);
        assert!(mac.leading_inset() > 0.0);
        assert_eq!(mac.trailing_inset(), 0.0);

        for host in [HostOs::Windows, HostOs::Linux] {
            let chrome = WindowChrome::resolve(host, ChromeStyle::AppDrawn);
            assert_eq!(chrome.leading_inset(), 0.0, "{host:?}");
            assert!(chrome.trailing_inset() > 0.0, "{host:?}");
        }
    }

    #[test]
    fn each_extra_control_costs_one_button_and_one_gap() {
        for host in HostOs::ALL {
            let layout = ControlLayout::for_host(host);
            assert_eq!(layout.reserved_inset(0), 0.0, "{host:?}");

            let step = layout.reserved_inset(3) - layout.reserved_inset(2);
            assert_eq!(
                step,
                layout.button_width + layout.gap,
                "a gnome session that enables minimise widens by one slot on {host:?}"
            );
        }
    }

    #[test]
    fn only_macos_keeps_drawing_the_window_buttons_for_us() {
        let mac = WindowChrome::resolve(HostOs::MacOs, ChromeStyle::AppDrawn);
        assert!(
            !mac.controls_drawn_by_app,
            "appkit still owns the traffic lights behind a transparent titlebar"
        );

        for host in [HostOs::Windows, HostOs::Linux] {
            let chrome = WindowChrome::resolve(host, ChromeStyle::AppDrawn);
            assert!(chrome.controls_drawn_by_app, "{host:?}");
        }
    }

    #[test]
    fn a_system_titlebar_never_asks_the_app_to_paint_buttons() {
        for host in HostOs::ALL {
            let chrome = WindowChrome::resolve(host, ChromeStyle::System);
            assert!(!chrome.controls_drawn_by_app, "{host:?}");
        }
    }

    #[test]
    fn macos_and_windows_keep_the_frame_even_with_the_titlebar_hidden() {
        for host in [HostOs::MacOs, HostOs::Windows] {
            for style in [ChromeStyle::System, ChromeStyle::AppDrawn] {
                assert_eq!(
                    requested_decorations(host, style),
                    DecorationMode::ServerSide,
                    "hiding the titlebar on {host:?} never hands us the frame"
                );
            }
        }
    }

    #[test]
    fn linux_is_the_only_platform_where_we_ask_for_the_frame() {
        let drawn = requested_decorations(HostOs::Linux, ChromeStyle::AppDrawn);
        assert_eq!(drawn, DecorationMode::ClientSide);

        let stock = requested_decorations(HostOs::Linux, ChromeStyle::System);
        assert_eq!(stock, DecorationMode::ServerSide);
    }

    #[test]
    fn only_client_side_decorations_make_the_border_and_the_corners_ours() {
        assert!(DecorationMode::ClientSide.app_draws_window_frame());
        assert!(DecorationMode::ClientSide.resize_border() > 0.0);
        assert!(DecorationMode::ClientSide.corner_radius() > 0.0);

        assert!(!DecorationMode::ServerSide.app_draws_window_frame());
        assert_eq!(
            DecorationMode::ServerSide.resize_border(),
            0.0,
            "reserving a grab margin the compositor owns steals its clicks"
        );
        assert_eq!(
            DecorationMode::ServerSide.corner_radius(),
            0.0,
            "our corners inside the compositor's round the window twice"
        );
    }

    #[test]
    fn a_compositor_that_refuses_client_side_puts_us_back_on_its_titlebar() {
        let refused = DecorationMode::ServerSide;
        let style = effective_style(HostOs::Linux, ChromeStyle::AppDrawn, refused);
        assert_eq!(
            style,
            ChromeStyle::System,
            "drawing our strip under the compositor's would give the user two"
        );
    }

    #[test]
    fn a_compositor_that_forces_client_side_makes_us_draw_the_titlebar() {
        let forced = DecorationMode::ClientSide;
        let style = effective_style(HostOs::Linux, ChromeStyle::System, forced);
        assert_eq!(
            style,
            ChromeStyle::AppDrawn,
            "gnome wayland leaves no system titlebar to fall back to"
        );
    }

    #[test]
    fn macos_and_windows_never_change_their_mind_about_the_titlebar() {
        for host in [HostOs::MacOs, HostOs::Windows] {
            for style in [ChromeStyle::System, ChromeStyle::AppDrawn] {
                assert_eq!(
                    effective_style(host, style, DecorationMode::ServerSide),
                    style,
                    "{host:?} always reports Server and it says nothing about the strip"
                );
            }
        }
    }

    #[test]
    fn only_macos_needs_its_window_buttons_repositioned() {
        let mac = WindowChrome::resolve(HostOs::MacOs, ChromeStyle::AppDrawn);
        assert!(mac.traffic_lights.is_some());

        let stock = WindowChrome::resolve(HostOs::MacOs, ChromeStyle::System);
        assert!(
            stock.traffic_lights.is_none(),
            "the system titlebar already holds them"
        );

        for host in [HostOs::Windows, HostOs::Linux] {
            let chrome = WindowChrome::resolve(host, ChromeStyle::AppDrawn);
            assert!(
                chrome.traffic_lights.is_none(),
                "{host:?} has no os-owned buttons floating over our content"
            );
        }
    }

    #[test]
    fn the_traffic_lights_stay_centred_in_whatever_strip_we_draw() {
        let base = WindowChrome::resolve(HostOs::MacOs, ChromeStyle::AppDrawn);
        for height in [28.0, 32.0, 44.0, 52.0] {
            let chrome = base.with_titlebar_height(height);
            let origin = chrome.traffic_lights.unwrap();
            assert_eq!(
                origin.implied_titlebar_height(&chrome.controls),
                height,
                "appkit sizes the container from the padding, so a stale one rides high"
            );
        }
    }

    #[test]
    fn raising_the_strip_invents_no_traffic_lights_on_other_platforms() {
        for host in [HostOs::Windows, HostOs::Linux] {
            let base = WindowChrome::resolve(host, ChromeStyle::AppDrawn);
            let chrome = base.with_titlebar_height(52.0);
            assert!(chrome.traffic_lights.is_none(), "{host:?}");
            assert_eq!(chrome.titlebar_height, 52.0, "{host:?}");
        }
    }

    #[test]
    fn nothing_is_laid_out_under_the_traffic_lights() {
        let chrome = WindowChrome::resolve(HostOs::MacOs, ChromeStyle::AppDrawn);
        let controls = chrome.controls;
        let origin = chrome.traffic_lights.unwrap();
        assert_eq!(origin.leading, controls.edge_padding);

        let group = controls.button_width * 3.0 + controls.gap * 2.0;
        assert!(
            chrome.leading_inset() >= origin.leading + group,
            "content would start under the zoom button"
        );
    }

    #[test]
    fn the_titlebar_heights_are_the_platform_ones() {
        assert_eq!(titlebar_height(HostOs::MacOs), 28.0);
        assert_eq!(titlebar_height(HostOs::Windows), 32.0);
        assert_eq!(titlebar_height(HostOs::Linux), 37.0);
    }

    #[test]
    fn a_windows_caption_button_is_the_size_microsoft_publishes() {
        let layout = ControlLayout::for_host(HostOs::Windows);
        assert_eq!(layout.button_width, 46.0);
        assert_eq!(layout.button_height, 32.0);
        assert_eq!(
            layout.button_height,
            titlebar_height(HostOs::Windows),
            "the caption strip is exactly one button tall"
        );
    }

    #[test]
    fn the_decoration_mode_survives_the_round_trip_through_gpui() {
        let server = DecorationMode::from_gpui(gpui::Decorations::Server);
        assert_eq!(server, DecorationMode::ServerSide);

        let client = gpui::Decorations::Client {
            tiling: gpui::Tiling::default(),
        };
        assert_eq!(
            DecorationMode::from_gpui(client),
            DecorationMode::ClientSide
        );

        let tiled = gpui::Decorations::Client {
            tiling: gpui::Tiling::tiled(),
        };
        assert_eq!(
            DecorationMode::from_gpui(tiled),
            DecorationMode::ClientSide,
            "a tiled window is still client-side decorated"
        );

        let asked = DecorationMode::ClientSide.into_gpui();
        assert_eq!(asked, gpui::WindowDecorations::Client);
        assert_eq!(
            DecorationMode::ServerSide.into_gpui(),
            gpui::WindowDecorations::Server
        );
    }

    #[test]
    fn the_stock_strip_is_hidden_only_when_we_replace_it() {
        for host in HostOs::ALL {
            let drawn = WindowChrome::resolve(host, ChromeStyle::AppDrawn);
            assert!(
                titlebar_options(&drawn, "R").appears_transparent,
                "{host:?}"
            );

            let stock = WindowChrome::resolve(host, ChromeStyle::System);
            let options = titlebar_options(&stock, "R");
            assert!(!options.appears_transparent, "{host:?}");
            assert!(options.traffic_light_position.is_none(), "{host:?}");
        }
    }

    #[test]
    fn the_titlebar_options_carry_a_traffic_light_origin_on_macos_only() {
        let mac = WindowChrome::resolve(HostOs::MacOs, ChromeStyle::AppDrawn);
        let options = titlebar_options(&mac, "Riseonly");
        let expected = gpui::point(gpui::px(20.0), gpui::px(7.0));
        assert_eq!(options.traffic_light_position, Some(expected));

        for host in [HostOs::Windows, HostOs::Linux] {
            let chrome = WindowChrome::resolve(host, ChromeStyle::AppDrawn);
            let options = titlebar_options(&chrome, "Riseonly");
            assert!(options.traffic_light_position.is_none(), "{host:?}");
        }
    }

    #[test]
    fn only_an_app_drawn_titlebar_forwards_its_own_double_click() {
        assert!(ChromeStyle::AppDrawn.app_handles_titlebar_double_click());
        assert!(
            !ChromeStyle::System.app_handles_titlebar_double_click(),
            "forwarding a click the os already acted on zooms twice"
        );
    }

    #[test]
    fn a_refused_request_is_not_mistaken_for_a_granted_one() {
        let chrome = WindowChrome::preferred(HostOs::Linux);

        let refused = GrantedChrome {
            decorations: DecorationMode::ServerSide,
            traffic_lights: PlatformSupport::Unsupported,
            client_inset: PlatformSupport::Unsupported,
        };
        assert!(!refused.matches_request(&chrome));

        let granted = GrantedChrome {
            decorations: DecorationMode::ClientSide,
            ..refused
        };
        assert!(granted.matches_request(&chrome));
    }

    #[test]
    fn the_preferred_chrome_is_the_app_drawn_one_on_every_platform() {
        for host in HostOs::ALL {
            assert_eq!(
                WindowChrome::preferred(host),
                WindowChrome::resolve(host, ChromeStyle::AppDrawn),
                "{host:?}"
            );
        }
    }
}
