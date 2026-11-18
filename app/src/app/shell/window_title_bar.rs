use std::collections::HashMap;

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, CursorStyle, Entity, Global, HitboxBehavior, Hsla, IntoElement,
    KeyBinding, MouseButton, Pixels, Render, Subscription, WeakEntity, Window, WindowControlArea, WindowId, actions,
    canvas, div, prelude::*, px, svg, transparent_black,
};
use rise_platform::HostOs;
use rise_platform::gpui_shim::supports_native_menu_bar;
use rise_platform::materials::{WindowsBuild, windows_build};
use rise_platform::window_chrome::{
    self, CaptionButton, CaptionClick, ChromeStyle, ControlLayout, ControlSide, DecorationMode,
    SNAP_LAYOUTS_BUILD, TrafficLightOrigin, WindowChrome,
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
        HostOs::Windows => false,
        HostOs::Linux => window_chrome::app_starts_window_move(host, granted),
    }
}

pub const fn controls_dock_in_rail(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs)
}

/// true if the strip itself handles double click, the os keeps it only on a titlebar it owns
pub const fn strip_handles_double_click(host: HostOs) -> bool {
    match host {
        HostOs::MacOs => window_chrome::app_owns_titlebar_drag(host),
        HostOs::Windows | HostOs::Linux => true,
    }
}

/// icon font windows draws its own caption glyphs with
pub fn windows_caption_font(build: Option<WindowsBuild>) -> &'static str {
    match build {
        Some(build) if build.is_at_least(SNAP_LAYOUTS_BUILD) => "Segoe Fluent Icons",
        _ => "Segoe MDL2 Assets",
    }
}

/// windows caption glyph, maximize shows restore while maximized
pub const fn windows_caption_glyph(button: CaptionButton, maximized: bool) -> &'static str {
    match button {
        CaptionButton::Minimize => "\u{e921}",
        CaptionButton::Maximize if maximized => "\u{e923}",
        CaptionButton::Maximize => "\u{e922}",
        CaptionButton::Close => "\u{e8bb}",
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

/// caption buttons fitted into the top band of the rail pane
/// desktop sizes if they fit then tighter gaps then narrower buttons
pub fn rail_control_layout(layout: ControlLayout, buttons: usize, rail_width: f32) -> ControlLayout {
    if buttons == 0 {
        return layout;
    }
    let count = buttons as f32;
    let natural = count * layout.button_width + (count - 1.0) * layout.gap + 2.0 * layout.edge_padding;
    if natural <= rail_width {
        return layout;
    }
    let slack = rail_width - count * layout.button_width;
    if slack > 0.0 {
        let space = slack / (count + 1.0);
        return ControlLayout {
            gap: layout.gap.min(space),
            edge_padding: layout.edge_padding.min(space),
            ..layout
        };
    }
    ControlLayout {
        button_width: rail_width / count,
        gap: 0.0,
        edge_padding: 0.0,
        ..layout
    }
}

const WINDOWS_CLOSE_HOVER: u32 = 0xC42B1C;
const WINDOWS_CAPTION_GLYPH_SIZE: f32 = 10.0;

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
    should_move: bool,
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
            should_move: false,
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
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down(MouseButton::Left, cx.listener(|bar, _, _, _| bar.should_move = true))
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
        rail_width: Option<f32>,
    ) -> Option<AnyElement> {
        if window.is_fullscreen() {
            return None;
        }
        let buttons =
            window_chrome::caption_buttons(chrome, window_chrome::available_controls(window));
        if buttons.is_empty() {
            return None;
        }

        let host = HostOs::current();
        let maximized = window.is_maximized();
        let in_rail = rail_width.is_some();
        let layout = match rail_width {
            Some(width) => rail_control_layout(chrome.controls, buttons.len(), width),
            None => chrome.controls,
        };
        let glyph_font = windows_caption_font(windows_build());

        let children = buttons.into_iter().map(|button| match host {
            HostOs::Windows => {
                Self::windows_button(button, &layout, maximized, glyph_font, theme, in_rail)
            }
            HostOs::MacOs | HostOs::Linux => {
                Self::painted_button(button, &layout, maximized, theme, on_close.cloned())
            }
        });

        Some(
            div()
                .id(CONTROLS_ID)
                .h_full()
                .flex()
                .flex_row()
                .when(matches!(host, HostOs::Windows) && !in_rail, |group| group.items_start())
                .when(!matches!(host, HostOs::Windows) || in_rail, |group| group.items_center())
                .px(px(layout.edge_padding))
                .gap(px(layout.gap))
                .children(children)
                .into_any_element(),
        )
    }

    fn windows_button(
        button: CaptionButton,
        layout: &window_chrome::ControlLayout,
        maximized: bool,
        glyph_font: &'static str,
        theme: &AppTheme,
        in_rail: bool,
    ) -> AnyElement {
        let close = matches!(button, CaptionButton::Close);
        let hover_fill: Hsla = if close {
            gpui::rgb(WINDOWS_CLOSE_HOVER).into()
        } else {
            theme.bg._300
        };
        let hover_glyph = if close {
            gpui::white()
        } else {
            theme.text.primary
        };

        div()
            .id(caption_id(button))
            .occlude()
            .w(px(layout.button_width))
            .h(px(layout.button_height))
            .flex()
            .items_center()
            .justify_center()
            .when(in_rail, |button| button.rounded_md())
            .font_family(glyph_font)
            .text_size(px(WINDOWS_CAPTION_GLYPH_SIZE))
            .text_color(theme.text.primary)
            .hover(move |style| style.bg(hover_fill).text_color(hover_glyph))
            .active(move |style| {
                style
                    .bg(hover_fill.opacity(0.8))
                    .text_color(hover_glyph.opacity(0.8))
            })
            .window_control_area(button.control_area())
            .child(windows_caption_glyph(button, maximized))
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
        let rail_width = self.controls_in_rail.then(|| f32::from(shell.rail_width));
        let controls = Self::controls(&chrome, &theme, window, self.on_close.as_ref(), rail_width);
        let (rail_controls, leading_controls, trailing_controls) = match (rail_width, side) {
            (Some(_), _) => (controls, None, None),
            (None, ControlSide::Leading) => (None, controls, None),
            (None, ControlSide::Trailing) => (None, None, controls),
        };
        let rail_controls = rail_controls.map(|controls| {
            div()
                .absolute()
                .top(shell.rail_outer_inset)
                .left(shell.rail_outer_inset)
                .w(shell.rail_width)
                .h(shell.window_drag_height)
                .flex()
                .items_center()
                .justify_center()
                .child(controls)
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
            .when(self.controls_in_rail, |strip| strip.pl(rail_reserve))
            .children(segments)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|bar, _, _, _| bar.should_move = false),
            )
            .on_mouse_down_out(cx.listener(|bar, _, _, _| bar.should_move = false))
            .on_mouse_move(cx.listener(|bar, _, window, _| {
                if !bar.should_move {
                    return;
                }
                bar.should_move = false;
                let granted = window_chrome::granted_decorations(window);
                if drag_moves_window(HostOs::current(), granted) {
                    window.start_window_move();
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
