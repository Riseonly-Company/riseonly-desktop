use crate::gpui_shim::{PlatformSupport, gpui_runs_on_wayland};
use crate::host_os::HostOs;
use crate::materials::{MacOsVersion, WindowsBuild};

/// who draws the strip along the top of the window
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChromeStyle {
    System,
    AppDrawn,
}

impl ChromeStyle {
    /// the shell own choice on every platform so the left rail reaches the top
    pub const fn preferred(host: HostOs) -> Self {
        match host {
            HostOs::MacOs | HostOs::Windows | HostOs::Linux => Self::AppDrawn,
        }
    }

    /// under a system titlebar the os already handles the gesture, forwarding would act twice
    pub const fn app_handles_titlebar_double_click(self) -> bool {
        matches!(self, Self::AppDrawn)
    }
}

/// which edge of the titlebar the window control buttons sit against
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ControlSide {
    Leading,
    Trailing,
}

pub const GNOME_DEFAULT_BUTTON_LAYOUT: &str = "appmenu:close";

impl ControlSide {
    /// which side wins, leading takes precedence and neither gives None
    pub const fn from_button_sides(has_leading: bool, has_trailing: bool) -> Option<Self> {
        if has_leading {
            Some(Self::Leading)
        } else if has_trailing {
            Some(Self::Trailing)
        } else {
            None
        }
    }

    /// parses a gnome style button-layout string, eg appmenu:close
    /// no colon means all trailing, None means no button named so fall back to window_control_side
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

/// the shape appkit gives a window own outer corner
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WindowCorner {
    pub radius: f32,
    pub continuous: bool,
}

const TAHOE_WINDOW_RADIUS: f32 = 26.5;

const LEGACY_WINDOW_RADIUS: f32 = 10.0;

/// corner macos gives a titled window by release, all the selectors that report it are private
pub const fn macos_window_corner(version: Option<MacOsVersion>) -> WindowCorner {
    if let Some(version) = version
        && version.is_at_least(MacOsVersion::LIQUID_GLASS)
    {
        return WindowCorner {
            radius: TAHOE_WINDOW_RADIUS,
            continuous: true,
        };
    }

    WindowCorner {
        radius: LEGACY_WINDOW_RADIUS,
        continuous: false,
    }
}

/// whether the app shapes the window corner itself, macos only since gpui rounding misses metal
pub const fn window_corner(host: HostOs, version: Option<MacOsVersion>) -> Option<WindowCorner> {
    match host {
        HostOs::MacOs => Some(macos_window_corner(version)),
        HostOs::Windows | HostOs::Linux => None,
    }
}

/// corner this process applies to its windows, incl the one run tuning override
pub fn current_window_corner() -> Option<WindowCorner> {
    let mut corner = window_corner(HostOs::current(), crate::materials::macos_version())?;

    if let Some(radius) = tuned_corner_radius(std::env::var(CORNER_RADIUS_VAR).ok().as_deref()) {
        corner.radius = radius;
    }

    Some(corner)
}

const WINDOWS_11_WINDOW_RADIUS: f32 = 8.0;

/// radius of the window own outer corner on this desktop, drawn by the os or by the app (csd)
/// linux uses libadwaita 12 in either mode since gnome and kde round about as much
pub const fn host_window_radius(
    host: HostOs,
    macos: Option<MacOsVersion>,
    windows: Option<WindowsBuild>,
) -> f32 {
    match host {
        HostOs::MacOs => macos_window_corner(macos).radius,
        HostOs::Windows => match windows {
            Some(build) if build.is_at_least(SNAP_LAYOUTS_BUILD) => WINDOWS_11_WINDOW_RADIUS,
            _ => 0.0,
        },
        HostOs::Linux => CLIENT_SIDE_CORNER_RADIUS,
    }
}

/// corner of a pane laid gap inside a window rounded at window_radius
/// macos keeps arcs concentric with the big window corner, floored at floor
/// windows and linux give each box the window own radius
pub fn pane_corner_radius(host: HostOs, window_radius: f32, gap: f32, floor: f32) -> f32 {
    match host {
        HostOs::MacOs => (window_radius - gap).max(floor),
        HostOs::Windows | HostOs::Linux => window_radius.max(0.0),
    }
}

/// pane_corner_radius for this process incl the macos one run tuning
pub fn current_pane_corner_radius(gap: f32, floor: f32) -> f32 {
    let host = HostOs::current();
    let window_radius = match current_window_corner() {
        Some(corner) => corner.radius,
        None => host_window_radius(host, crate::materials::macos_version(), crate::materials::windows_build()),
    };
    pane_corner_radius(host, window_radius, gap, floor)
}

/// macos leads and windows trails, linux is a fallback until desktop_control_side answers
pub const fn window_control_side(host: HostOs) -> ControlSide {
    match host {
        HostOs::MacOs => ControlSide::Leading,
        HostOs::Windows | HostOs::Linux => ControlSide::Trailing,
    }
}

/// whether the frame around the window belongs to the compositor or to the app
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DecorationMode {
    ServerSide,
    ClientSide,
}

const CLIENT_SIDE_RESIZE_BORDER: f32 = 10.0;

const CLIENT_SIDE_CORNER_RADIUS: f32 = 12.0;

impl DecorationMode {
    /// under csd the resize border, shadow and corners are painted and hit tested by the app
    pub const fn app_draws_window_frame(self) -> bool {
        matches!(self, Self::ClientSide)
    }

    /// logical pixels reserved outside the visible window for edge resizing
    pub const fn resize_border(self) -> f32 {
        match self {
            Self::ServerSide => 0.0,
            Self::ClientSide => CLIENT_SIDE_RESIZE_BORDER,
        }
    }

    /// logical pixels of corner rounding the app has to paint itself
    pub const fn corner_radius(self) -> f32 {
        match self {
            Self::ServerSide => 0.0,
            Self::ClientSide => CLIENT_SIDE_CORNER_RADIUS,
        }
    }

    /// what gpui says the window is configured as
    /// the edge tiling state it also carries is dropped here
    pub const fn from_gpui(decorations: gpui::Decorations) -> Self {
        match decorations {
            gpui::Decorations::Server => Self::ServerSide,
            gpui::Decorations::Client { .. } => Self::ClientSide,
        }
    }

    /// what gpui is asked for, at creation and through Window request_decorations
    pub const fn into_gpui(self) -> gpui::WindowDecorations {
        match self {
            Self::ServerSide => gpui::WindowDecorations::Server,
            Self::ClientSide => gpui::WindowDecorations::Client,
        }
    }
}

/// what to ask gpui for, only linux can be handed the frame
pub const fn requested_decorations(host: HostOs, style: ChromeStyle) -> DecorationMode {
    match host {
        HostOs::MacOs | HostOs::Windows => DecorationMode::ServerSide,
        HostOs::Linux => match style {
            ChromeStyle::System => DecorationMode::ServerSide,
            ChromeStyle::AppDrawn => DecorationMode::ClientSide,
        },
    }
}

/// what a window opens asking for, client side on wayland and the server frame on x11
/// window follows the answer either way
pub const fn opening_decorations(requested: DecorationMode, wayland_session: bool) -> DecorationMode {
    match requested {
        DecorationMode::ClientSide if wayland_session => DecorationMode::ClientSide,
        DecorationMode::ClientSide | DecorationMode::ServerSide => DecorationMode::ServerSide,
    }
}

/// the window_decorations option for a window with this chrome
/// left unset gpui asks for the server frame, which a wayland compositor with xdg-decoration grants
pub fn shell_window_decorations(chrome: &WindowChrome) -> gpui::WindowDecorations {
    opening_decorations(chrome.decorations, gpui_runs_on_wayland()).into_gpui()
}

/// the style in force once the platform answered, on linux the granted mode is the style
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

/// height of one platform titlebar in logical pixels
pub const fn titlebar_height(host: HostOs) -> f32 {
    match host {
        HostOs::MacOs => 28.0,
        HostOs::Windows => 32.0,
        HostOs::Linux => 37.0,
    }
}

/// how far a screen keeps its own controls below the window top edge, given the shell strip height
/// macos and windows are zero, the lights sit over the rail
/// linux under csd puts buttons and a drag area in the strip, ssd titlebar is outside
pub const fn controls_band_height(host: HostOs, granted: DecorationMode, strip_height: f32) -> f32 {
    match host {
        HostOs::MacOs | HostOs::Windows => 0.0,
        HostOs::Linux => match granted {
            DecorationMode::ClientSide => strip_height,
            DecorationMode::ServerSide => 0.0,
        },
    }
}

/// where a full window cover starts and how far it keeps its controls below its top edge
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CoverTopInsets {
    pub frame_top: f32,
    pub safe_top: f32,
}

/// a cover cant hide the buttons the app draws, so where the strip holds them the cover starts below it
/// macos lights float over any layer so the cover stays and its controls move
/// full screen has neither
pub const fn cover_top_insets(
    host: HostOs,
    granted: DecorationMode,
    strip_height: f32,
    is_fullscreen: bool,
) -> CoverTopInsets {
    if is_fullscreen {
        return CoverTopInsets { frame_top: 0.0, safe_top: 0.0 };
    }
    match host {
        HostOs::MacOs | HostOs::Windows => CoverTopInsets { frame_top: 0.0, safe_top: strip_height },
        HostOs::Linux => CoverTopInsets { frame_top: controls_band_height(host, granted, strip_height), safe_top: 0.0 },
    }
}

pub const FULL_CONTROL_SET: usize = 3;

/// geometry of one platform window control buttons, in logical pixels
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ControlLayout {
    pub side: ControlSide,
    pub button_width: f32,
    pub button_height: f32,
    pub gap: f32,
    pub edge_padding: f32,
}

impl ControlLayout {
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

    /// room the controls need, measured from ControlLayout side
    pub fn reserved_inset(&self, buttons: usize) -> f32 {
        if buttons == 0 {
            return 0.0;
        }
        let count = buttons as f32;
        self.edge_padding * 2.0 + count * self.button_width + (count - 1.0) * self.gap
    }
}

/// traffic light placement in an app drawn titlebar, gpui reads the vertical value as padding
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TrafficLightOrigin {
    pub leading: f32,
    pub vertical_padding: f32,
}

impl TrafficLightOrigin {
    /// centres the group vertically in a strip of height
    pub fn centred_in(layout: &ControlLayout, height: f32) -> Self {
        Self {
            leading: layout.edge_padding,
            vertical_padding: ((height - layout.button_height) / 2.0).max(0.0),
        }
    }

    pub fn implied_titlebar_height(&self, layout: &ControlLayout) -> f32 {
        layout.button_height + self.vertical_padding * 2.0
    }
}

/// one platform whole chrome decision, holdable without knowing the platform
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WindowChrome {
    pub style: ChromeStyle,
    pub titlebar_height: f32,
    pub controls: ControlLayout,
    pub controls_drawn_by_app: bool,
    pub reserved_inset: f32,
    pub decorations: DecorationMode,
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

    /// the shell own choice for this platform
    pub fn preferred(host: HostOs) -> Self {
        Self::resolve(host, ChromeStyle::preferred(host))
    }

    /// re lays the strip at another height and keeps the traffic lights centred
    pub fn with_titlebar_height(mut self, height: f32) -> Self {
        self.titlebar_height = height;
        if self.traffic_lights.is_some() {
            let origin = TrafficLightOrigin::centred_in(&self.controls, height);
            self.traffic_lights = Some(origin);
        }
        self
    }

    /// padding the shell adds at the leading edge of its titlebar row
    pub fn leading_inset(&self) -> f32 {
        match self.controls.side {
            ControlSide::Leading => self.reserved_inset,
            ControlSide::Trailing => 0.0,
        }
    }

    /// padding the shell adds at the trailing edge of its titlebar row
    pub fn trailing_inset(&self) -> f32 {
        match self.controls.side {
            ControlSide::Leading => 0.0,
            ControlSide::Trailing => self.reserved_inset,
        }
    }
}

/// what the platform granted, as opposed to what WindowChrome asked for
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GrantedChrome {
    pub decorations: DecorationMode,
    pub traffic_lights: PlatformSupport,
    pub client_inset: PlatformSupport,
}

impl GrantedChrome {
    pub fn matches_request(&self, chrome: &WindowChrome) -> bool {
        self.decorations == chrome.decorations
    }
}

/// the titlebar half of gpui WindowOptions, must come from the same WindowChrome
pub fn titlebar_options(chrome: &WindowChrome, title: &str) -> gpui::TitlebarOptions {
    gpui::TitlebarOptions {
        title: Some(title.into()),
        appears_transparent: matches!(chrome.style, ChromeStyle::AppDrawn),
        traffic_light_position: chrome.traffic_lights.map(traffic_light_point),
    }
}

pub(super) fn traffic_light_point(origin: TrafficLightOrigin) -> gpui::Point<gpui::Pixels> {
    gpui::point(gpui::px(origin.leading), gpui::px(origin.vertical_padding))
}

/// applies chrome to an open window
/// on wayland the read back only settles on configure
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

pub fn round_window_corner(window: &gpui::Window) -> PlatformSupport {
    super::os::round_window_corner(window)
}

pub fn set_system_controls_appearance(
    window: &gpui::Window,
    appearance: Option<gpui::WindowAppearance>,
) -> PlatformSupport {
    super::os::set_system_controls_appearance(window, appearance)
}

#[cfg(feature = "dev-drive")]
pub fn conceal_window_for_dev_drive(window: &gpui::Window) -> PlatformSupport {
    super::os::conceal_window_for_dev_drive(window)
}

/// subscription of watch_windows_for_dev_drive, dropping it ends the watch
#[cfg(feature = "dev-drive")]
pub struct DevDriveWindowWatch {
    _registration: Box<dyn std::any::Any>,
}

#[cfg(feature = "dev-drive")]
pub fn watch_windows_for_dev_drive(
    wake: Box<dyn Fn() + Send + Sync>,
) -> Option<DevDriveWindowWatch> {
    super::os::watch_windows_for_dev_drive(wake)
        .map(|registration| DevDriveWindowWatch { _registration: registration })
}

#[cfg(feature = "dev-drive")]
pub fn keep_app_in_background_for_dev_drive() -> PlatformSupport {
    super::os::keep_app_in_background_for_dev_drive()
}

pub const CORNER_RADIUS_VAR: &str = "RISE_CORNER";

/// parses that variable, a radius outside 0 to 64 is refused and not applied
pub fn tuned_corner_radius(raw: Option<&str>) -> Option<f32> {
    let radius: f32 = raw?.trim().parse().ok()?;

    (radius.is_finite() && (0.0..=64.0).contains(&radius)).then_some(radius)
}

/// what the window is configured as right now, cheap enough per frame
pub fn granted_decorations(window: &gpui::Window) -> DecorationMode {
    DecorationMode::from_gpui(window.window_decorations())
}

fn place_traffic_lights(window: &gpui::Window, chrome: &WindowChrome) -> PlatformSupport {
    super::os::place_traffic_lights(window, chrome)
}

/// which window operations the platform actually offers
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

pub fn desktop_control_side(app: &gpui::App, host: HostOs) -> ControlSide {
    app.button_layout()
        .and_then(|layout| {
            let leading = layout.left.iter().any(Option::is_some);
            let trailing = layout.right.iter().any(Option::is_some);
            ControlSide::from_button_sides(leading, trailing)
        })
        .unwrap_or_else(|| window_control_side(host))
}

/// only gpui_macos implements the forward, elsewhere the call compiles and does nothing
pub const fn forwards_titlebar_double_click(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs => PlatformSupport::Performed,
        HostOs::Windows | HostOs::Linux => PlatformSupport::Unsupported,
    }
}

/// what a double click on empty space in the strip the shell draws turns into
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TitlebarDoubleClick {
    ForwardToOs,
    ToggleMaximize,
    Nothing,
}

/// double click action for a granted frame, resizable is Window is_resizable
pub const fn titlebar_double_click_action(
    host: HostOs,
    granted: DecorationMode,
    offered: AvailableControls,
    resizable: bool,
) -> TitlebarDoubleClick {
    match host {
        HostOs::MacOs => TitlebarDoubleClick::ForwardToOs,
        HostOs::Windows => {
            if offered.maximize && resizable {
                TitlebarDoubleClick::ToggleMaximize
            } else {
                TitlebarDoubleClick::Nothing
            }
        }
        HostOs::Linux => {
            if granted.app_draws_window_frame() && offered.maximize && resizable {
                TitlebarDoubleClick::ToggleMaximize
            } else {
                TitlebarDoubleClick::Nothing
            }
        }
    }
}

/// carries out a double click on empty titlebar space the way the desktop would
pub fn titlebar_double_click(window: &gpui::Window, chrome: &WindowChrome) -> PlatformSupport {
    if !chrome.style.app_handles_titlebar_double_click() {
        return PlatformSupport::Unsupported;
    }

    let action = titlebar_double_click_action(
        HostOs::current(),
        granted_decorations(window),
        available_controls(window),
        window.is_resizable(),
    );
    match action {
        TitlebarDoubleClick::ForwardToOs => window.titlebar_double_click(),
        TitlebarDoubleClick::ToggleMaximize => return super::os::toggle_maximize(window),
        TitlebarDoubleClick::Nothing => return PlatformSupport::Unsupported,
    }
    PlatformSupport::Performed
}

/// true where the shell strip takes titlebar drags and double clicks from the os
/// appkit only saw the top 28 points of a taller strip and let double clicks fall through
pub const fn app_owns_titlebar_drag(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs)
}

/// whether a drag on the strip has to go to the window system as an interactive move
pub const fn app_starts_window_move(host: HostOs, granted: DecorationMode) -> bool {
    match host {
        HostOs::MacOs => false,
        HostOs::Windows => true,
        HostOs::Linux => granted.app_draws_window_frame(),
    }
}

/// true where the strip is told to the os as caption area, only where no column runs under it
pub const fn strip_is_caption_area(host: HostOs) -> bool {
    !matches!(host, HostOs::Windows)
}

pub fn start_titlebar_move(window: &gpui::Window) -> PlatformSupport {
    let host = HostOs::current();
    if !app_starts_window_move(host, granted_decorations(window)) && !app_owns_titlebar_drag(host) {
        return PlatformSupport::Unsupported;
    }
    super::os::start_titlebar_move(window)
}

/// whether a secondary click on the strip opens the compositor window menu
pub const fn shows_window_menu(
    host: HostOs,
    granted: DecorationMode,
    offered: AvailableControls,
) -> bool {
    matches!(host, HostOs::Linux) && granted.app_draws_window_frame() && offered.window_menu
}

pub fn open_window_menu(
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
) -> PlatformSupport {
    let host = HostOs::current();
    if !shows_window_menu(host, granted_decorations(window), available_controls(window)) {
        return PlatformSupport::Unsupported;
    }

    window.show_window_menu(position);
    PlatformSupport::Performed
}

/// frame edge or corner under a point, in logical pixels from the window top left
/// border is the grab margin outside the window, sides the compositor tiled never answer
pub fn resize_edge_at(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    border: f32,
    tiling: gpui::Tiling,
) -> Option<gpui::ResizeEdge> {
    let grab = border * 1.5;
    let inside = |left: f32, top: f32, w: f32, h: f32| {
        x >= left && x < left + w && y >= top && y < top + h
    };

    if inside(grab, grab, width - grab * 2.0, height - grab * 2.0) {
        return None;
    }

    if !tiling.top && inside(0.0, 0.0, grab, grab) {
        return Some(gpui::ResizeEdge::TopLeft);
    }
    if !tiling.top && inside(width - grab, 0.0, grab, grab) {
        return Some(gpui::ResizeEdge::TopRight);
    }
    if !tiling.bottom && inside(0.0, height - grab, grab, grab) {
        return Some(gpui::ResizeEdge::BottomLeft);
    }
    if !tiling.bottom && inside(width - grab, height - grab, grab, grab) {
        return Some(gpui::ResizeEdge::BottomRight);
    }

    if !tiling.top && y < border {
        Some(gpui::ResizeEdge::Top)
    } else if !tiling.bottom && y > height - border {
        Some(gpui::ResizeEdge::Bottom)
    } else if !tiling.left && x < border {
        Some(gpui::ResizeEdge::Left)
    } else if !tiling.right && x > width - border {
        Some(gpui::ResizeEdge::Right)
    } else {
        None
    }
}

/// pointer shape the frame shows while hovering edge
pub const fn resize_cursor(edge: gpui::ResizeEdge) -> gpui::CursorStyle {
    match edge {
        gpui::ResizeEdge::Top | gpui::ResizeEdge::Bottom => gpui::CursorStyle::ResizeUpDown,
        gpui::ResizeEdge::Left | gpui::ResizeEdge::Right => gpui::CursorStyle::ResizeLeftRight,
        gpui::ResizeEdge::TopLeft | gpui::ResizeEdge::BottomRight => {
            gpui::CursorStyle::ResizeUpLeftDownRight
        }
        gpui::ResizeEdge::TopRight | gpui::ResizeEdge::BottomLeft => {
            gpui::CursorStyle::ResizeUpRightDownLeft
        }
    }
}

/// edge a pointer at position would resize, only where the app draws the frame
pub fn frame_edge_under(
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
) -> Option<gpui::ResizeEdge> {
    let gpui::Decorations::Client { tiling } = window.window_decorations() else {
        return None;
    };
    if !window.is_resizable() {
        return None;
    }

    let size = window.viewport_size();
    resize_edge_at(
        f32::from(position.x),
        f32::from(position.y),
        f32::from(size.width),
        f32::from(size.height),
        DecorationMode::ClientSide.resize_border(),
        tiling,
    )
}

pub fn start_frame_resize(
    window: &gpui::Window,
    position: gpui::Point<gpui::Pixels>,
) -> PlatformSupport {
    match frame_edge_under(window, position) {
        Some(edge) => {
            window.start_window_resize(edge);
            PlatformSupport::Performed
        }
        None => PlatformSupport::Unsupported,
    }
}

/// one window button the shell paints itself when controls_drawn_by_app
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CaptionButton {
    Minimize,
    Maximize,
    Close,
}

impl CaptionButton {
    /// the area the button element declares with window_control_area
    pub const fn control_area(self) -> gpui::WindowControlArea {
        match self {
            Self::Minimize => gpui::WindowControlArea::Min,
            Self::Maximize => gpui::WindowControlArea::Max,
            Self::Close => gpui::WindowControlArea::Close,
        }
    }
}

/// buttons the shell paints, left to right
/// empty when the os paints its own
pub fn caption_buttons(chrome: &WindowChrome, offered: AvailableControls) -> Vec<CaptionButton> {
    if !chrome.controls_drawn_by_app {
        return Vec::new();
    }

    let mut buttons = Vec::with_capacity(FULL_CONTROL_SET);
    if matches!(chrome.controls.side, ControlSide::Leading) {
        buttons.push(CaptionButton::Close);
    }
    if offered.minimize {
        buttons.push(CaptionButton::Minimize);
    }
    if offered.maximize {
        buttons.push(CaptionButton::Maximize);
    }
    if matches!(chrome.controls.side, ControlSide::Trailing) {
        buttons.push(CaptionButton::Close);
    }
    buttons
}

/// who carries out a click on a button the shell painted
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CaptionClick {
    Os,
    App,
}

/// windows routes the app buttons through its non client hit test, elsewhere the app acts
pub const fn caption_click(host: HostOs) -> CaptionClick {
    match host {
        HostOs::Windows => CaptionClick::Os,
        HostOs::MacOs | HostOs::Linux => CaptionClick::App,
    }
}

pub fn perform_caption_click(window: &mut gpui::Window, button: CaptionButton) -> PlatformSupport {
    if matches!(caption_click(HostOs::current()), CaptionClick::Os) {
        return PlatformSupport::Unsupported;
    }

    match button {
        CaptionButton::Minimize => window.minimize_window(),
        CaptionButton::Maximize => window.zoom_window(),
        CaptionButton::Close => window.remove_window(),
    }
    PlatformSupport::Performed
}

pub const SNAP_LAYOUTS_BUILD: WindowsBuild = WindowsBuild(22_000);

/// whether hovering the maximize button the shell paints opens the snap layouts flyout
pub const fn snap_layouts(host: HostOs, build: Option<WindowsBuild>) -> PlatformSupport {
    match (host, build) {
        (HostOs::Windows, Some(build)) if build.is_at_least(SNAP_LAYOUTS_BUILD) => {
            PlatformSupport::Performed
        }
        _ => PlatformSupport::Unsupported,
    }
}

pub fn current_snap_layouts() -> PlatformSupport {
    snap_layouts(HostOs::current(), crate::materials::windows_build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cover_leaves_the_controls_of_each_host_usable() {
        let mac = cover_top_insets(HostOs::MacOs, DecorationMode::ServerSide, 46.0, false);
        assert_eq!(mac, CoverTopInsets { frame_top: 0.0, safe_top: 46.0 });
        let windows = cover_top_insets(HostOs::Windows, DecorationMode::ServerSide, 46.0, false);
        assert_eq!(windows, CoverTopInsets { frame_top: 0.0, safe_top: 46.0 });
        let csd = cover_top_insets(HostOs::Linux, DecorationMode::ClientSide, 46.0, false);
        assert_eq!(csd, CoverTopInsets { frame_top: 46.0, safe_top: 0.0 });
        let ssd = cover_top_insets(HostOs::Linux, DecorationMode::ServerSide, 46.0, false);
        assert_eq!(ssd, CoverTopInsets { frame_top: 0.0, safe_top: 0.0 });
        let full = cover_top_insets(HostOs::MacOs, DecorationMode::ServerSide, 46.0, true);
        assert_eq!(full, CoverTopInsets { frame_top: 0.0, safe_top: 0.0 });
    }

    #[test]
    fn only_a_strip_with_buttons_in_it_pushes_screen_controls_down() {
        let strip = 46.0;
        for granted in [DecorationMode::ServerSide, DecorationMode::ClientSide] {
            assert_eq!(controls_band_height(HostOs::MacOs, granted, strip), 0.0);
            assert_eq!(controls_band_height(HostOs::Windows, granted, strip), 0.0);
        }
        assert_eq!(controls_band_height(HostOs::Linux, DecorationMode::ClientSide, strip), strip);
        assert_eq!(controls_band_height(HostOs::Linux, DecorationMode::ServerSide, strip), 0.0);
    }

    #[test]
    fn macos_26_uses_the_measured_continuous_corner() {
        let corner = macos_window_corner(Some(MacOsVersion::LIQUID_GLASS));

        assert_eq!(corner.radius, 26.5);
        assert!(corner.continuous);
    }

    #[test]
    fn the_tuning_variable_replaces_the_measured_radius() {
        assert_eq!(tuned_corner_radius(Some("22")), Some(22.0));
        assert_eq!(tuned_corner_radius(Some(" 17.5 ")), Some(17.5));
        assert_eq!(tuned_corner_radius(Some("0")), Some(0.0));
    }

    #[test]
    fn an_implausible_radius_is_refused_rather_than_applied() {
        for raw in [
            None,
            Some(""),
            Some("wide"),
            Some("-4"),
            Some("400"),
            Some("NaN"),
        ] {
            assert_eq!(tuned_corner_radius(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn the_releases_before_it_keep_the_smaller_circular_corner() {
        for version in [MacOsVersion::VIBRANCY, MacOsVersion::new(15, 6, 1)] {
            let corner = macos_window_corner(Some(version));

            assert_eq!(corner.radius, 10.0, "{version:?}");
            assert!(!corner.continuous, "{version:?}");
        }
    }

    #[test]
    fn an_unreadable_version_takes_the_smaller_corner_rather_than_the_larger() {
        assert_eq!(
            macos_window_corner(None),
            macos_window_corner(Some(MacOsVersion::VIBRANCY))
        );
    }

    #[test]
    fn only_macos_asks_the_app_to_shape_the_window_corner() {
        assert!(window_corner(HostOs::MacOs, Some(MacOsVersion::LIQUID_GLASS)).is_some());

        for host in [HostOs::Windows, HostOs::Linux] {
            assert!(
                window_corner(host, None).is_none(),
                "{host:?} rounds the window without us, and a second corner over \
                 the top of the first is a notch"
            );
        }
    }

    #[test]
    fn the_close_button_clears_the_corner_this_module_actually_holds() {
        let corner = macos_window_corner(Some(MacOsVersion::LIQUID_GLASS));
        let layout = ControlLayout::for_host(HostOs::MacOs);
        let inset = layout.edge_padding;

        let offset = (corner.radius - inset).max(0.0);
        assert!(
            offset * offset * 2.0 <= corner.radius * corner.radius,
            "at a {}pt radius the button at {inset}pt would ride on the rounding",
            corner.radius
        );
    }

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
    fn a_window_opens_client_decorated_on_wayland_only() {
        let client = DecorationMode::ClientSide;
        let server = DecorationMode::ServerSide;
        assert_eq!(opening_decorations(client, true), client);
        assert_eq!(opening_decorations(client, false), server);
        assert_eq!(opening_decorations(server, true), server, "macOS and Windows never ask for it");
        assert_eq!(opening_decorations(server, false), server);
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

    fn all_offered() -> AvailableControls {
        AvailableControls {
            minimize: true,
            maximize: true,
            fullscreen: true,
            window_menu: true,
        }
    }

    #[test]
    fn windows_paints_minimise_maximise_close_from_left_to_right() {
        let chrome = WindowChrome::preferred(HostOs::Windows);
        assert_eq!(
            caption_buttons(&chrome, all_offered()),
            vec![CaptionButton::Minimize, CaptionButton::Maximize, CaptionButton::Close]
        );
    }

    #[test]
    fn the_painted_buttons_fit_the_room_the_chrome_reserved() {
        for host in [HostOs::Windows, HostOs::Linux] {
            let chrome = WindowChrome::preferred(host);
            let buttons = caption_buttons(&chrome, all_offered());
            let painted = chrome.controls.reserved_inset(buttons.len());
            assert_eq!(painted, chrome.reserved_inset, "{host:?}");
        }
    }

    #[test]
    fn a_leading_layout_puts_close_first() {
        let mut chrome = WindowChrome::preferred(HostOs::Linux);
        chrome.controls.side = ControlSide::Leading;
        assert_eq!(caption_buttons(&chrome, all_offered()).first(), Some(&CaptionButton::Close));
    }

    #[test]
    fn a_withheld_operation_loses_its_button_but_close_stays() {
        let chrome = WindowChrome::preferred(HostOs::Windows);
        let offered = AvailableControls {
            minimize: false,
            maximize: false,
            ..all_offered()
        };
        assert_eq!(caption_buttons(&chrome, offered), vec![CaptionButton::Close]);
    }

    #[test]
    fn nothing_is_painted_where_the_os_draws_the_buttons() {
        assert!(caption_buttons(&WindowChrome::preferred(HostOs::MacOs), all_offered()).is_empty());
        for host in HostOs::ALL {
            let system = WindowChrome::resolve(host, ChromeStyle::System);
            assert!(caption_buttons(&system, all_offered()).is_empty(), "{host:?}");
        }
    }

    #[test]
    fn every_button_maps_to_its_own_hit_test_area() {
        assert_eq!(CaptionButton::Minimize.control_area(), gpui::WindowControlArea::Min);
        assert_eq!(CaptionButton::Maximize.control_area(), gpui::WindowControlArea::Max);
        assert_eq!(CaptionButton::Close.control_area(), gpui::WindowControlArea::Close);
    }

    #[test]
    fn only_windows_carries_out_the_click_itself() {
        assert_eq!(caption_click(HostOs::Windows), CaptionClick::Os);
        assert_eq!(caption_click(HostOs::Linux), CaptionClick::App);
        assert_eq!(caption_click(HostOs::MacOs), CaptionClick::App);
    }

    #[test]
    fn snap_layouts_open_from_windows_11_only() {
        let windows = |build| snap_layouts(HostOs::Windows, build).is_supported();
        assert!(windows(Some(SNAP_LAYOUTS_BUILD)));
        assert!(windows(Some(WindowsBuild(26_100))));
        assert!(!windows(Some(WindowsBuild(19_045))));
        assert!(!windows(None));
        for host in [HostOs::MacOs, HostOs::Linux] {
            assert!(!snap_layouts(host, Some(WindowsBuild(26_100))).is_supported(), "{host:?}");
        }
    }

    #[test]
    fn a_linux_double_click_maximizes_only_where_we_draw_the_frame() {
        let offered = all_offered();
        assert_eq!(
            titlebar_double_click_action(HostOs::Linux, DecorationMode::ClientSide, offered, true),
            TitlebarDoubleClick::ToggleMaximize,
        );
        assert_eq!(
            titlebar_double_click_action(HostOs::Linux, DecorationMode::ServerSide, offered, true),
            TitlebarDoubleClick::Nothing,
        );
    }

    #[test]
    fn a_window_that_cannot_grow_ignores_the_double_click() {
        let mut offered = all_offered();
        let client = DecorationMode::ClientSide;
        let fixed = titlebar_double_click_action(HostOs::Linux, client, offered, false);
        assert_eq!(fixed, TitlebarDoubleClick::Nothing);

        offered.maximize = false;
        let no_maximize = titlebar_double_click_action(HostOs::Linux, client, offered, true);
        assert_eq!(no_maximize, TitlebarDoubleClick::Nothing);
    }

    #[test]
    fn macos_forwards_and_windows_toggles_maximize_from_the_app_strip() {
        let offered = all_offered();
        let server = DecorationMode::ServerSide;
        let mac = titlebar_double_click_action(HostOs::MacOs, server, offered, true);
        assert_eq!(mac, TitlebarDoubleClick::ForwardToOs);
        let windows = titlebar_double_click_action(HostOs::Windows, server, offered, true);
        assert_eq!(windows, TitlebarDoubleClick::ToggleMaximize);
        let fixed = titlebar_double_click_action(HostOs::Windows, server, offered, false);
        assert_eq!(fixed, TitlebarDoubleClick::Nothing);
    }

    #[test]
    fn only_a_client_side_linux_frame_moves_and_opens_the_menu_through_us() {
        let offered = all_offered();
        assert!(app_starts_window_move(HostOs::Linux, DecorationMode::ClientSide));
        assert!(shows_window_menu(HostOs::Linux, DecorationMode::ClientSide, offered));
        assert!(!app_starts_window_move(HostOs::Linux, DecorationMode::ServerSide));
        assert!(!shows_window_menu(HostOs::Linux, DecorationMode::ServerSide, offered));
        for host in [HostOs::MacOs, HostOs::Windows] {
            assert!(!shows_window_menu(host, DecorationMode::ClientSide, offered), "{host:?}");
        }
        assert!(!app_starts_window_move(HostOs::MacOs, DecorationMode::ClientSide));
        assert!(app_starts_window_move(HostOs::Windows, DecorationMode::ServerSide));
        assert!(!strip_is_caption_area(HostOs::Windows));
    }

    #[test]
    fn a_compositor_without_a_window_menu_gets_no_right_click_request() {
        let mut offered = all_offered();
        offered.window_menu = false;
        assert!(!shows_window_menu(HostOs::Linux, DecorationMode::ClientSide, offered));
    }

    #[test]
    fn the_content_area_never_resizes() {
        let edge = resize_edge_at(400.0, 300.0, 800.0, 600.0, 10.0, gpui::Tiling::default());
        assert_eq!(edge, None);
    }

    #[test]
    fn each_edge_and_corner_of_a_floating_window_answers() {
        let at = |x, y| resize_edge_at(x, y, 800.0, 600.0, 10.0, gpui::Tiling::default());
        assert_eq!(at(2.0, 2.0), Some(gpui::ResizeEdge::TopLeft));
        assert_eq!(at(798.0, 2.0), Some(gpui::ResizeEdge::TopRight));
        assert_eq!(at(2.0, 598.0), Some(gpui::ResizeEdge::BottomLeft));
        assert_eq!(at(798.0, 598.0), Some(gpui::ResizeEdge::BottomRight));
        assert_eq!(at(400.0, 4.0), Some(gpui::ResizeEdge::Top));
        assert_eq!(at(400.0, 596.0), Some(gpui::ResizeEdge::Bottom));
        assert_eq!(at(4.0, 300.0), Some(gpui::ResizeEdge::Left));
        assert_eq!(at(796.0, 300.0), Some(gpui::ResizeEdge::Right));
    }

    #[test]
    fn a_corner_grab_reaches_further_than_an_edge_grab() {
        let edge = resize_edge_at(12.0, 12.0, 800.0, 600.0, 10.0, gpui::Tiling::default());
        assert_eq!(edge, Some(gpui::ResizeEdge::TopLeft));
        let between = resize_edge_at(400.0, 12.0, 800.0, 600.0, 10.0, gpui::Tiling::default());
        assert_eq!(between, None);
    }

    #[test]
    fn a_tiled_side_is_not_offered_for_resizing() {
        let left_half = gpui::Tiling {
            top: true,
            left: true,
            right: false,
            bottom: true,
        };
        let at = |x, y| resize_edge_at(x, y, 800.0, 600.0, 10.0, left_half);
        assert_eq!(at(2.0, 2.0), None);
        assert_eq!(at(400.0, 4.0), None);
        assert_eq!(at(4.0, 300.0), None);
        assert_eq!(at(796.0, 300.0), Some(gpui::ResizeEdge::Right));
        assert_eq!(resize_edge_at(400.0, 300.0, 800.0, 600.0, 10.0, gpui::Tiling::tiled()), None);
    }

    #[test]
    fn opposite_corners_share_a_diagonal_cursor() {
        let falling = resize_cursor(gpui::ResizeEdge::TopLeft);
        assert_eq!(falling, resize_cursor(gpui::ResizeEdge::BottomRight));
        let rising = resize_cursor(gpui::ResizeEdge::TopRight);
        assert_eq!(rising, resize_cursor(gpui::ResizeEdge::BottomLeft));
        assert_ne!(falling, rising);
        assert_eq!(resize_cursor(gpui::ResizeEdge::Top), gpui::CursorStyle::ResizeUpDown);
        assert_eq!(resize_cursor(gpui::ResizeEdge::Left), gpui::CursorStyle::ResizeLeftRight);
    }
}

#[cfg(test)]
mod pane_corner_tests {
    use super::*;

    #[test]
    fn panes_follow_the_window_radius_of_each_desktop() {
        assert_eq!(host_window_radius(HostOs::Windows, None, Some(SNAP_LAYOUTS_BUILD)), 8.0);
        assert_eq!(host_window_radius(HostOs::Windows, None, Some(WindowsBuild(19_045))), 0.0);
        assert_eq!(host_window_radius(HostOs::Linux, None, None), 12.0);
        assert_eq!(pane_corner_radius(HostOs::Windows, 8.0, 6.0, 10.0), 8.0);
        assert_eq!(pane_corner_radius(HostOs::Linux, 12.0, 6.0, 10.0), 12.0);
    }

    #[test]
    fn mac_panes_stay_concentric_with_the_window_corner() {
        let tahoe = host_window_radius(HostOs::MacOs, Some(MacOsVersion::LIQUID_GLASS), None);
        assert_eq!(pane_corner_radius(HostOs::MacOs, tahoe, 6.0, 10.0), tahoe - 6.0);
        let legacy = host_window_radius(HostOs::MacOs, Some(MacOsVersion::VIBRANCY), None);
        assert_eq!(pane_corner_radius(HostOs::MacOs, legacy, 6.0, 10.0), 10.0);
    }
}
