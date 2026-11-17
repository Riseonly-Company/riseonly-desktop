use rise_platform::HostOs;
use rise_platform::materials::WindowsBuild;
use rise_platform::window_chrome::{
    AvailableControls, CaptionButton, ChromeStyle, ControlLayout, ControlSide, DecorationMode,
    caption_buttons,
};
use rise_theme::ShellMetrics;

use crate::app::router::navigation_types::{MainTab, RootRoute};

use super::window_title_bar::draws_under_window_controls;
use super::window_title_bar::{
    LINUX_MENU_BAR_KEYSTROKES, WINDOWS_MENU_BAR_KEYSTROKES, drag_moves_window, granted_chrome,
    linux_caption_icon, menu_bar_keystrokes, rail_control_layout, shell_chrome,
    strip_handles_double_click, windows_caption_font, windows_caption_glyph,
};

const ALL_HOSTS: [HostOs; 3] = [HostOs::MacOs, HostOs::Windows, HostOs::Linux];

fn every_control() -> AvailableControls {
    AvailableControls {
        minimize: true,
        maximize: true,
        fullscreen: true,
        window_menu: true,
    }
}

#[test]
fn only_screens_that_handle_the_band_themselves_escape_the_pane_padding() {
    for route in [
        RootRoute::MainTabs(MainTab::Maps),
        RootRoute::WorldMap,
        RootRoute::MapJourneyComposer,
        RootRoute::SignIn,
        RootRoute::SignUp,
    ] {
        assert!(draws_under_window_controls(&route));
    }
    for route in [
        RootRoute::Onboarding,
        RootRoute::MainTabs(MainTab::Shorts),
        RootRoute::MapFriends,
        RootRoute::MapSettings,
    ] {
        assert!(!draws_under_window_controls(&route));
    }
}

#[test]
fn every_desktop_opens_with_the_strip_the_columns_leave_empty() {
    let metrics = ShellMetrics::default();
    for host in ALL_HOSTS {
        let chrome = shell_chrome(host, &metrics);
        assert_eq!(chrome.style, ChromeStyle::AppDrawn, "{host:?}");
        assert_eq!(
            chrome.titlebar_height,
            f32::from(metrics.window_drag_height),
            "the strip must cover exactly the band no screen draws into on {host:?}"
        );
    }
}

#[test]
fn the_traffic_lights_stay_where_the_rail_expects_them() {
    let metrics = ShellMetrics::default();
    let origin = metrics.traffic_light_origin();
    let lights = shell_chrome(HostOs::MacOs, &metrics)
        .traffic_lights
        .expect("macOS keeps its own buttons inside the strip");

    assert_eq!(lights.leading, f32::from(origin.x));
    assert_eq!(lights.vertical_padding, f32::from(origin.y));
    assert!(shell_chrome(HostOs::Windows, &metrics).traffic_lights.is_none());
    assert!(shell_chrome(HostOs::Linux, &metrics).traffic_lights.is_none());
}

#[test]
fn windows_paints_its_buttons_on_the_right_with_close_last() {
    let metrics = ShellMetrics::default();
    let requested = shell_chrome(HostOs::Windows, &metrics);
    let chrome = granted_chrome(
        HostOs::Windows,
        &requested,
        DecorationMode::ServerSide,
        ControlSide::Trailing,
    );

    assert!(chrome.controls_drawn_by_app);
    assert_eq!(chrome.controls.side, ControlSide::Trailing);
    assert_eq!(
        caption_buttons(&chrome, every_control()),
        vec![
            CaptionButton::Minimize,
            CaptionButton::Maximize,
            CaptionButton::Close
        ]
    );
}

#[test]
fn macos_paints_no_buttons_of_its_own() {
    let metrics = ShellMetrics::default();
    let requested = shell_chrome(HostOs::MacOs, &metrics);
    let chrome = granted_chrome(
        HostOs::MacOs,
        &requested,
        DecorationMode::ServerSide,
        ControlSide::Leading,
    );

    assert!(caption_buttons(&chrome, every_control()).is_empty());
    assert_eq!(chrome.traffic_lights, requested.traffic_lights);
}

#[test]
fn a_linux_compositor_that_kept_its_frame_gets_no_second_set_of_buttons() {
    let metrics = ShellMetrics::default();
    let requested = shell_chrome(HostOs::Linux, &metrics);
    let chrome = granted_chrome(
        HostOs::Linux,
        &requested,
        DecorationMode::ServerSide,
        ControlSide::Trailing,
    );

    assert_eq!(chrome.style, ChromeStyle::System);
    assert!(caption_buttons(&chrome, every_control()).is_empty());
    assert_eq!(chrome.titlebar_height, requested.titlebar_height);
}

#[test]
fn linux_client_side_buttons_follow_the_desktop_side() {
    let metrics = ShellMetrics::default();
    let requested = shell_chrome(HostOs::Linux, &metrics);

    let trailing = granted_chrome(
        HostOs::Linux,
        &requested,
        DecorationMode::ClientSide,
        ControlSide::Trailing,
    );
    assert_eq!(
        caption_buttons(&trailing, every_control()).last(),
        Some(&CaptionButton::Close)
    );

    let leading = granted_chrome(
        HostOs::Linux,
        &requested,
        DecorationMode::ClientSide,
        ControlSide::Leading,
    );
    assert_eq!(leading.controls.side, ControlSide::Leading);
    assert_eq!(
        caption_buttons(&leading, every_control()).first(),
        Some(&CaptionButton::Close)
    );
}

#[test]
fn a_window_that_cannot_grow_shows_no_maximize_button() {
    let metrics = ShellMetrics::default();
    let requested = shell_chrome(HostOs::Linux, &metrics);
    let chrome = granted_chrome(
        HostOs::Linux,
        &requested,
        DecorationMode::ClientSide,
        ControlSide::Trailing,
    );
    let offered = AvailableControls {
        maximize: false,
        ..every_control()
    };

    assert_eq!(
        caption_buttons(&chrome, offered),
        vec![CaptionButton::Minimize, CaptionButton::Close]
    );
}

#[test]
fn only_a_client_decorated_linux_strip_starts_a_window_move() {
    assert!(
        !drag_moves_window(HostOs::MacOs, DecorationMode::ServerSide),
        "AppKit keeps its own titlebar drag"
    );
    assert!(
        !drag_moves_window(HostOs::Windows, DecorationMode::ServerSide),
        "Windows moves the window from HTCAPTION before gpui sees the press"
    );
    assert!(drag_moves_window(HostOs::Linux, DecorationMode::ClientSide));
    assert!(
        !drag_moves_window(HostOs::Linux, DecorationMode::ServerSide),
        "the compositor's own titlebar moves a server-decorated window"
    );
}

#[test]
fn macos_leaves_the_double_click_to_appkit() {
    assert!(!strip_handles_double_click(HostOs::MacOs));
    assert!(strip_handles_double_click(HostOs::Windows));
    assert!(strip_handles_double_click(HostOs::Linux));
}

#[test]
fn the_menu_keys_exist_only_without_a_system_menu_bar() {
    for host in ALL_HOSTS {
        assert!(menu_bar_keystrokes(host, true).is_empty(), "{host:?}");
    }
    assert!(menu_bar_keystrokes(HostOs::MacOs, false).is_empty());
    assert_eq!(
        menu_bar_keystrokes(HostOs::Windows, false),
        &WINDOWS_MENU_BAR_KEYSTROKES
    );
    assert_eq!(
        menu_bar_keystrokes(HostOs::Linux, false),
        &LINUX_MENU_BAR_KEYSTROKES
    );
}

#[test]
fn only_windows_opens_the_menu_on_a_lone_alt() {
    assert!(WINDOWS_MENU_BAR_KEYSTROKES.contains(&"alt"));
    assert!(WINDOWS_MENU_BAR_KEYSTROKES.contains(&"f10"));
    assert_eq!(LINUX_MENU_BAR_KEYSTROKES, ["f10"]);
}

#[test]
fn windows_10_draws_the_glyphs_from_the_older_icon_font() {
    assert_eq!(windows_caption_font(Some(WindowsBuild(22_621))), "Segoe Fluent Icons");
    assert_eq!(windows_caption_font(Some(WindowsBuild(19_045))), "Segoe MDL2 Assets");
    assert_eq!(windows_caption_font(None), "Segoe MDL2 Assets");
}

#[test]
fn a_maximized_window_offers_restore_instead_of_maximize() {
    assert_eq!(
        windows_caption_glyph(CaptionButton::Maximize, false),
        "\u{e922}"
    );
    assert_eq!(
        windows_caption_glyph(CaptionButton::Maximize, true),
        "\u{e923}"
    );
    assert_eq!(linux_caption_icon(CaptionButton::Maximize, false), "square");
    assert_eq!(linux_caption_icon(CaptionButton::Maximize, true), "copy");
}

#[test]
fn the_linux_button_drawings_ship_with_the_app() {
    let lucide = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("assets")
        .join("icons")
        .join("lucide");
    for maximized in [false, true] {
        for button in [
            CaptionButton::Minimize,
            CaptionButton::Maximize,
            CaptionButton::Close,
        ] {
            let name = format!("{}.svg", linux_caption_icon(button, maximized));
            assert!(lucide.join(&name).is_file(), "missing assets/icons/lucide/{name}");
        }
    }
}

fn caption_span(layout: &ControlLayout, buttons: usize) -> f32 {
    let count = buttons as f32;
    count * layout.button_width + (count - 1.0) * layout.gap + 2.0 * layout.edge_padding
}

#[test]
fn the_window_buttons_fit_inside_the_rail_pane_on_every_desktop() {
    let rail = f32::from(ShellMetrics::default().rail_width);
    for host in ALL_HOSTS {
        let desktop = ControlLayout::for_host(host);
        let fitted = rail_control_layout(desktop, 3, rail);
        assert!(
            caption_span(&fitted, 3) <= rail + f32::EPSILON,
            "{host:?}: the buttons spill out of the rail pane"
        );
        assert_eq!(fitted.button_height, desktop.button_height);
        assert!(fitted.button_width > 0.0);
    }
}

#[test]
fn buttons_that_already_fit_keep_the_desktop_sizes() {
    let desktop = ControlLayout::for_host(HostOs::Linux);
    let wide = caption_span(&desktop, 3) + 10.0;
    assert_eq!(rail_control_layout(desktop, 3, wide), desktop);
    assert_eq!(rail_control_layout(desktop, 0, 1.0), desktop);
}

#[test]
fn the_gaps_give_way_before_the_buttons_shrink() {
    let desktop = ControlLayout::for_host(HostOs::Linux);
    let tight = 3.0 * desktop.button_width + 8.0;
    let fitted = rail_control_layout(desktop, 3, tight);
    assert_eq!(fitted.button_width, desktop.button_width);
    assert!(fitted.gap < desktop.gap);

    let windows = rail_control_layout(ControlLayout::for_host(HostOs::Windows), 3, 90.0);
    assert_eq!(windows.button_width, 30.0);
    assert_eq!(windows.gap, 0.0);
}
