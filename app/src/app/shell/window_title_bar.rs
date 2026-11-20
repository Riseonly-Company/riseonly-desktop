use std::collections::HashMap;

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, CursorStyle, Entity, Global, HitboxBehavior, Hsla, IntoElement,
    KeyBinding, MouseButton, Pixels, Render, Subscription, WeakEntity, Window, WindowControlArea, WindowId, actions,
    canvas, deferred, div, prelude::*, px, svg, transparent_black,
};
use rise_platform::HostOs;
use rise_platform::gpui_shim::supports_native_menu_bar;
use rise_platform::window_chrome::{
    self, CaptionButton, CaptionClick, ChromeStyle, ControlSide, DecorationMode, TrafficLightOrigin, WindowChrome,
};
use rise_theme::{AppTheme, ShellMetrics};
use rise_ui::IconUi;

use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::desktop_platform::app_menu_bar::{AppMenuBar, draws_in_window_menu_bar};

const TITLE_BAR_ID: &str = "shell.title_bar";
const MENU_SLOT_ID: &str = "shell.title_bar.menu";
const CONTROLS_ID: &str = "shell.title_bar.controls";
const FRAME_ID: &str = "shell.window_frame";

actions!(rise_title_bar, [ToggleAppMenuBar]);

pub const WINDOWS_MENU_BAR_KEYSTROKES: [&str; 2] = ["f10", "alt"];

/// none where the window draws no menu bar, the keys belong to the focused field there
pub fn menu_bar_keystrokes(host: HostOs, native_menu_bar: bool) -> &'static [&'static str] {
    if draws_in_window_menu_bar(host, native_menu_bar) { &WINDOWS_MENU_BAR_KEYSTROKES } else { &[] }
}

/// chrome every shell window opens with, app drawn on all three desktops
/// strip as tall as the band the columns leave empty, traffic lights where the rail expects them
pub fn shell_chrome(host: HostOs, metrics: &ShellMetrics) -> WindowChrome {
    let mut chrome = WindowChrome::resolve(host, ChromeStyle::preferred(host))
        .with_titlebar_height(f32::from(metrics.window_drag_height));

    if chrome.traffic_lights.is_some() {
        let origin = metrics.traffic_light_origin();
        chrome.traffic_lights = Some(TrafficLightOrigin {
            leading: f32::from(origin.x),
            vertical_padding: f32::from(origin.y),
        });
    }
    chrome
}

/// chrome in force once the platform answered, buttons on the desktops side
pub fn granted_chrome(
    host: HostOs,
    requested: &WindowChrome,
    granted: DecorationMode,
    desktop_side: ControlSide,
) -> WindowChrome {
    let style = window_chrome::effective_style(host, requested.style, granted);
    let mut chrome = WindowChrome::resolve(host, style);

    chrome.titlebar_height = requested.titlebar_height;
    chrome.decorations = granted;
    if style == requested.style {
        chrome.traffic_lights = requested.traffic_lights;
    }
    if chrome.controls_drawn_by_app {
        chrome.controls.side = desktop_side;
    }
    chrome
}

/// screens the root doesnt pad under the strip
/// the world map paints under it and the auth flow already keeps a band
pub fn draws_under_window_controls(route: &RootRoute) -> bool {
    matches!(
        route,
        RootRoute::MainTabs(MainTab::Maps)
            | RootRoute::WorldMap
            | RootRoute::MapJourneyComposer
            | RootRoute::SignIn
            | RootRoute::SignUp
    )
}

/// true if a left drag on empty strip space asks the window system to move the window
pub const fn drag_moves_window(host: HostOs, granted: DecorationMode) -> bool {
    match host {
        HostOs::MacOs => window_chrome::app_owns_titlebar_drag(host),
        HostOs::Windows | HostOs::Linux => window_chrome::app_starts_window_move(host, granted),
    }
}

pub const fn controls_dock_in_rail(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs | HostOs::Windows)
}

/// windows draws its buttons as lights in the top left corner of every window, rail or not
pub const fn lights_in_corner(host: HostOs) -> bool {
    matches!(host, HostOs::Windows)
}

/// true if the strip itself handles double click, the os keeps it only on a titlebar it owns
pub const fn strip_handles_double_click(host: HostOs) -> bool {
    match host {
        HostOs::MacOs => window_chrome::app_owns_titlebar_drag(host),
        HostOs::Windows | HostOs::Linux => true,
    }
}

/// lucide drawing the app painted linux button shows
pub const fn linux_caption_icon(button: CaptionButton, maximized: bool) -> &'static str {
    match button {
        CaptionButton::Minimize => "minus",
        CaptionButton::Maximize if maximized => "copy",
        CaptionButton::Maximize => "square",
        CaptionButton::Close => "x",
    }
}

const WINDOW_MOVE_THRESHOLD: f32 = 4.0;
const LIGHTS_GROUP: &str = "shell.title_bar.lights";
const LIGHT_SLOT: f32 = 14.0;
const LIGHT_GAP: f32 = 9.0;
const LIGHT_DIAMETER: f32 = 12.0;
const LIGHT_GLYPH: f32 = 8.0;
const LIGHT_CLOSE: u32 = 0xFF5F57;
const LIGHT_MINIMIZE: u32 = 0xFEBC2E;
const LIGHT_MAXIMIZE: u32 = 0x28C840;
const LIGHTS_LAYER_PRIORITY: usize = 2000;

/// fill of each light as on macos
pub const fn traffic_light_fill(button: CaptionButton) -> u32 {
    match button {
        CaptionButton::Close => LIGHT_CLOSE,
        CaptionButton::Minimize => LIGHT_MINIMIZE,
        CaptionButton::Maximize => LIGHT_MAXIMIZE,
    }
}

pub const TRAFFIC_LIGHT_ORDER: [CaptionButton; 3] = [CaptionButton::Close, CaptionButton::Minimize, CaptionButton::Maximize];

#[derive(Default)]
struct TitleBars(HashMap<WindowId, WeakEntity<WindowTitleBar>>);

impl Global for TitleBars {}

struct TitleBarKeysInstalled;

impl Global for TitleBarKeysInstalled {}

fn install_menu_bar_keys(cx: &mut App) {
    if cx.has_global::<TitleBarKeysInstalled>() {
        return;
    }
    cx.set_global(TitleBarKeysInstalled);

    let keystrokes =
        menu_bar_keystrokes(HostOs::current(), supports_native_menu_bar().is_supported());
    if keystrokes.is_empty() {
        return;
    }
    cx.bind_keys(
        keystrokes
            .iter()
            .map(|keystroke| KeyBinding::new(keystroke, ToggleAppMenuBar, None)),
    );
    cx.on_action(|_: &ToggleAppMenuBar, cx| toggle_menu_bar_in_active_window(cx));
}

fn toggle_menu_bar_in_active_window(cx: &mut App) {
    let Some(handle) = cx.active_window() else {
        return;
    };
    let Some(bar) = cx
        .try_global::<TitleBars>()
        .and_then(|bars| bars.0.get(&handle.window_id()))
        .and_then(WeakEntity::upgrade)
    else {
        return;
    };
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            bar.update(cx, |bar, cx| bar.toggle_menu_bar(window, cx));
        });
    });
}

pub type CloseAction = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct WindowTitleBar {
    menu_bar: Option<Entity<AppMenuBar>>,
    _menu_observation: Option<Subscription>,
    controls_theme: Option<AppTheme>,
    on_close: Option<CloseAction>,
    should_move: Option<gpui::Point<Pixels>>,
    controls_in_rail: bool,
    drag_holes: Vec<(Pixels, Pixels)>,
}

impl WindowTitleBar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let menu_bar = cx.new(AppMenuBar::new);
        Self::mounted(Some(menu_bar), window, cx)
    }

    pub fn without_app_menu(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::mounted(None, window, cx)
    }

    fn mounted(menu_bar: Option<Entity<AppMenuBar>>, window: &Window, cx: &mut Context<Self>) -> Self {
        install_menu_bar_keys(cx);

        let window_id = window.window_handle().window_id();
        let this = cx.weak_entity();
        let bars = cx.default_global::<TitleBars>();
        bars.0.retain(|_, bar| bar.upgrade().is_some());
        bars.0.insert(window_id, this);
        let menu_observation = menu_bar.as_ref().map(|menu| cx.observe(menu, |_, _, cx| cx.notify()));

        Self {
            menu_bar,
            _menu_observation: menu_observation,
            controls_theme: None,
            on_close: None,
            should_move: None,
            controls_in_rail: false,
            drag_holes: Vec::new(),
        }
    }

    /// for a window with its own closing rule
    /// the painted close button removes the window directly and on_window_should_close never hears of it
    pub fn on_close(mut self, on_close: CloseAction) -> Self {
        self.on_close = Some(on_close);
        self
    }

    pub fn toggle_menu_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu_bar) = &self.menu_bar else {
            return;
        };
        menu_bar.update(cx, |menu, cx| {
            if menu.open_section().is_some() {
                menu.close(window, cx);
            } else {
                menu.open_section_at(0, window, cx);
            }
        });
    }

    pub fn set_controls_theme(&mut self, theme: Option<AppTheme>, cx: &mut Context<Self>) {
        self.controls_theme = theme;
        cx.notify();
    }

    pub fn dock_controls_in_rail(&mut self, docked: bool, cx: &mut Context<Self>) {
        let docked = docked && controls_dock_in_rail(HostOs::current());
        if self.controls_in_rail == docked {
            return;
        }
        self.controls_in_rail = docked;
        cx.notify();
    }

    pub fn set_drag_holes(&mut self, holes: Vec<(Pixels, Pixels)>, cx: &mut Context<Self>) {
        if self.drag_holes == holes {
            return;
        }
        self.drag_holes = holes;
        cx.notify();
    }

    fn drag_segment(&self, index: usize, start: Pixels, end: Pixels, chrome: WindowChrome, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id(("window-drag-segment", index))
            .absolute()
            .top_0()
            .bottom_0()
            .left(start)
            .w(end - start)
            .when(window_chrome::strip_is_caption_area(HostOs::current()), |segment| {
                segment.window_control_area(WindowControlArea::Drag)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|bar, event: &gpui::MouseDownEvent, _, _| bar.should_move = Some(event.position)),
            )
            .on_click(move |event, window, _| {
                if event.click_count() == 2 && strip_handles_double_click(HostOs::current()) {
                    let _ = window_chrome::titlebar_double_click(window, &chrome);
                }
            })
            .on_mouse_down(MouseButton::Right, |event, window, _| {
                let _ = window_chrome::open_window_menu(window, event.position);
            })
    }

    fn chrome(window: &Window, cx: &App) -> WindowChrome {
        let host = HostOs::current();
        let requested = shell_chrome(host, &rise_ui::theme(cx).shell);
        granted_chrome(
            host,
            &requested,
            window_chrome::granted_decorations(window),
            window_chrome::desktop_control_side(cx, host),
        )
    }

    fn controls(
        chrome: &WindowChrome,
        theme: &AppTheme,
        window: &Window,
        on_close: Option<&CloseAction>,
    ) -> Option<AnyElement> {
        if window.is_fullscreen() {
            return None;
        }
        let buttons =
            window_chrome::caption_buttons(chrome, window_chrome::available_controls(window));
        if buttons.is_empty() {
            return None;
        }

        let maximized = window.is_maximized();
        if lights_in_corner(HostOs::current()) {
            return Some(Self::traffic_lights(&buttons, maximized, window.is_window_active(), theme));
        }
        let layout = chrome.controls;
        let children = buttons
            .into_iter()
            .map(|button| Self::painted_button(button, &layout, maximized, theme, on_close.cloned()));

        Some(
            div()
                .id(CONTROLS_ID)
                .h_full()
                .flex()
                .flex_row()
                .items_center()
                .px(px(layout.edge_padding))
                .gap(px(layout.gap))
                .children(children)
                .into_any_element(),
        )
    }

    fn traffic_lights(buttons: &[CaptionButton], maximized: bool, active: bool, theme: &AppTheme) -> AnyElement {
        let lights = TRAFFIC_LIGHT_ORDER
            .into_iter()
            .filter(|button| buttons.contains(button))
            .map(|button| Self::traffic_light(button, maximized, active, theme));
        div()
            .id(CONTROLS_ID)
            .group(LIGHTS_GROUP)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(LIGHT_GAP))
            .children(lights)
            .into_any_element()
    }

    fn traffic_light(button: CaptionButton, maximized: bool, active: bool, theme: &AppTheme) -> AnyElement {
        let lit: Hsla = gpui::rgb(traffic_light_fill(button)).into();
        let fill = if active { lit } else { theme.bg._400 };
        let icon = format!("{}{}.svg", IconUi::ASSET_PREFIX, linux_caption_icon(button, maximized));
        div()
            .id(caption_id(button))
            .occlude()
            .size(px(LIGHT_SLOT))
            .flex()
            .items_center()
            .justify_center()
            .window_control_area(button.control_area())
            .child(
                div()
                    .size(px(LIGHT_DIAMETER))
                    .rounded_full()
                    .bg(fill)
                    .group_hover(LIGHTS_GROUP, move |style| style.bg(lit))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        svg()
                            .path(icon)
                            .size(px(LIGHT_GLYPH))
                            .text_color(gpui::black().opacity(0.6))
                            .opacity(0.0)
                            .group_hover(LIGHTS_GROUP, |style| style.opacity(1.0)),
                    ),
            )
            .into_any_element()
    }

    fn painted_button(
        button: CaptionButton,
        layout: &window_chrome::ControlLayout,
        maximized: bool,
        theme: &AppTheme,
        on_close: Option<CloseAction>,
    ) -> AnyElement {
        let icon = format!(
            "{}{}.svg",
            IconUi::ASSET_PREFIX,
            linux_caption_icon(button, maximized)
        );
        let side = theme.icon.small;

        div()
            .id(caption_id(button))
            .occlude()
            .size(px(layout.button_width))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme.bg._200)
            .hover(|style| style.bg(theme.bg._300))
            .active(|style| style.bg(theme.bg._400))
            .window_control_area(button.control_area())
            .child(svg().path(icon).size(side).text_color(theme.text.primary))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                if matches!(
                    window_chrome::caption_click(HostOs::current()),
                    CaptionClick::App
                ) {
                    match on_close.as_ref().filter(|_| matches!(button, CaptionButton::Close)) {
                        Some(on_close) => on_close(window, cx),
                        None => {
                            let _ = window_chrome::perform_caption_click(window, button);
                        }
                    }
                }
            })
            .into_any_element()
    }
}

fn caption_id(button: CaptionButton) -> &'static str {
    match button {
        CaptionButton::Minimize => "shell.title_bar.minimize",
        CaptionButton::Maximize => "shell.title_bar.maximize",
        CaptionButton::Close => "shell.title_bar.close",
    }
}

impl Render for WindowTitleBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self
            .controls_theme
            .clone()
            .unwrap_or_else(|| rise_ui::theme(cx).clone());
        let chrome = Self::chrome(window, cx);
        let side = chrome.controls.side;

        let shell = theme.shell;
        let host = HostOs::current();
        let in_corner = self.controls_in_rail || lights_in_corner(host);
        let controls = Self::controls(&chrome, &theme, window, self.on_close.as_ref());
        let (rail_controls, leading_controls, trailing_controls) = match (in_corner, side) {
            (true, _) => (controls, None, None),
            (false, ControlSide::Leading) => (None, controls, None),
            (false, ControlSide::Trailing) => (None, None, controls),
        };
        let rail_controls = rail_controls.map(|controls| {
            let corner = div()
                .absolute()
                .top(shell.rail_outer_inset)
                .left(shell.rail_outer_inset)
                .w(shell.rail_width)
                .h(shell.window_drag_height)
                .flex()
                .items_center()
                .justify_center()
                .child(controls);
            if lights_in_corner(host) {
                deferred(corner).with_priority(LIGHTS_LAYER_PRIORITY).into_any_element()
            } else {
                corner.into_any_element()
            }
        });
        let rail_reserve = shell.rail_width + shell.rail_outer_inset * 2.0;

        let drawn_menu = self.menu_bar.as_ref().filter(|menu| menu.read(cx).is_drawn());
        let menu = drawn_menu.map(|menu| {
            let is_open = menu.read(cx).open_section().is_some();
            div()
                .id(MENU_SLOT_ID)
                .when(is_open, |slot| slot.occlude())
                .h_full()
                .flex()
                .items_center()
                .px(theme.spacing._200)
                .child(menu.clone())
        });

        let segments: Vec<AnyElement> = drag_segments(window.viewport_size().width, &self.drag_holes)
            .into_iter()
            .enumerate()
            .map(|(index, (start, end))| self.drag_segment(index, start, end, chrome, cx).into_any_element())
            .collect();
        div()
            .id(TITLE_BAR_ID)
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(chrome.titlebar_height))
            .flex()
            .flex_row()
            .items_center()
            .when(in_corner, |strip| strip.pl(rail_reserve))
            .children(segments)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|bar, _, _, _| bar.should_move = None),
            )
            .on_mouse_down_out(cx.listener(|bar, _, _, _| bar.should_move = None))
            .on_mouse_move(cx.listener(|bar, event: &gpui::MouseMoveEvent, window, _| {
                let Some(start) = bar.should_move else {
                    return;
                };
                let travel = event.position - start;
                if travel.x.abs().max(travel.y.abs()) < px(WINDOW_MOVE_THRESHOLD) {
                    return;
                }
                bar.should_move = None;
                let granted = window_chrome::granted_decorations(window);
                if drag_moves_window(HostOs::current(), granted) {
                    let _ = window_chrome::start_titlebar_move(window);
                }
            }))
            .children(leading_controls)
            .children(menu)
            .child(div().flex_1().h_full())
            .children(trailing_controls)
            .children(rail_controls)
    }
}

/// the strip minus the holes, as window x ranges that move the window
pub(crate) fn drag_segments(width: Pixels, holes: &[(Pixels, Pixels)]) -> Vec<(Pixels, Pixels)> {
    let mut holes: Vec<(Pixels, Pixels)> = holes
        .iter()
        .map(|(start, end)| ((*start).max(px(0.0)), (*end).min(width)))
        .filter(|(start, end)| start < end)
        .collect();
    holes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut segments = Vec::with_capacity(holes.len() + 1);
    let mut cursor = px(0.0);
    for (start, end) in holes {
        if start > cursor {
            segments.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    if cursor < width {
        segments.push((cursor, width));
    }
    segments
}

/// wraps the window content in the frame owed to the compositor under client side decorations
/// resize margin and edge cursors and border and shadow
pub fn window_frame(content: impl IntoElement, window: &mut Window, cx: &App) -> AnyElement {
    let decorations = window.window_decorations();
    let granted = DecorationMode::from_gpui(decorations);
    let border = px(granted.resize_border());
    window.set_client_inset(border);

    let gpui::Decorations::Client { tiling } = decorations else {
        return div().id(FRAME_ID).size_full().child(content).into_any_element();
    };

    let theme = rise_ui::theme(cx);
    let radius = px(granted.corner_radius());
    let resizable = window.is_resizable();

    let body = div()
        .size_full()
        .cursor(CursorStyle::Arrow)
        .border_color(theme.border._200)
        .when(!tiling.top, |body| body.border_t_1())
        .when(!tiling.bottom, |body| body.border_b_1())
        .when(!tiling.left, |body| body.border_l_1())
        .when(!tiling.right, |body| body.border_r_1())
        .when(!(tiling.top || tiling.left), |body| body.rounded_tl(radius))
        .when(!(tiling.top || tiling.right), |body| body.rounded_tr(radius))
        .when(!(tiling.bottom || tiling.left), |body| body.rounded_bl(radius))
        .when(!(tiling.bottom || tiling.right), |body| body.rounded_br(radius))
        .when(!tiling.is_tiled(), |body| {
            body.shadow(vec![
                gpui::BoxShadow::new(px(0.0), px(0.0), theme.material.scrim)
                    .blur_radius(border / 2.0),
            ])
        })
        .overflow_hidden()
        .on_mouse_move(|_, _, cx| cx.stop_propagation())
        .child(content);

    div()
        .id(FRAME_ID)
        .size_full()
        .bg(transparent_black())
        .when(!tiling.top, |frame| frame.pt(border))
        .when(!tiling.bottom, |frame| frame.pb(border))
        .when(!tiling.left, |frame| frame.pl(border))
        .when(!tiling.right, |frame| frame.pr(border))
        .when(resizable, |frame| {
            frame.on_mouse_down(MouseButton::Left, |event, window, _| {
                let _ = window_chrome::start_frame_resize(window, event.position);
            })
        })
        .child(body)
        .when(resizable, |frame| {
            frame.child(
                canvas(
                    |_, window, _| {
                        let whole = gpui::Bounds::new(gpui::Point::default(), window.viewport_size());
                        window.insert_hitbox(whole, HitboxBehavior::Normal)
                    },
                    |_, hitbox, window, _| {
                        let edge = window_chrome::frame_edge_under(window, window.mouse_position());
                        if let Some(edge) = edge {
                            window.set_cursor_style(window_chrome::resize_cursor(edge), &hitbox);
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
        })
        .into_any_element()
}
