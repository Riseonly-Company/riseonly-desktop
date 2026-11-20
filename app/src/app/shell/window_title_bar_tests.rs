use rise_platform::HostOs;
use rise_platform::window_chrome::{
    AvailableControls, CaptionButton, ChromeStyle, ControlSide, DecorationMode, caption_buttons,
};
use rise_theme::ShellMetrics;

use crate::app::router::navigation_types::{MainTab, RootRoute};

use super::window_title_bar::{controls_dock_in_rail, drag_segments, draws_under_window_controls, lights_in_corner};
use super::window_title_bar::{
    TRAFFIC_LIGHT_ORDER, WINDOWS_MENU_BAR_KEYSTROKES, drag_moves_window, granted_chrome, linux_caption_icon,
    menu_bar_keystrokes, shell_chrome, strip_handles_double_click, traffic_light_fill,
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
fn the_strip_moves_the_window_where_the_app_owns_the_titlebar() {
    assert!(
        drag_moves_window(HostOs::MacOs, DecorationMode::ServerSide),
        "the shell strip owns the macOS titlebar drag"
    );
    assert!(
        drag_moves_window(HostOs::Windows, DecorationMode::ServerSide),
        "the Windows strip lies over the columns, so the app starts the move"
    );
    assert!(drag_moves_window(HostOs::Linux, DecorationMode::ClientSide));
    assert!(
        !drag_moves_window(HostOs::Linux, DecorationMode::ServerSide),
        "the compositor's own titlebar moves a server-decorated window"
    );
}

#[test]
fn every_strip_handles_its_own_double_click() {
    assert!(strip_handles_double_click(HostOs::MacOs));
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
    assert!(menu_bar_keystrokes(HostOs::Linux, false).is_empty());
}

#[test]
fn only_windows_opens_the_menu_on_a_lone_alt() {
    assert!(WINDOWS_MENU_BAR_KEYSTROKES.contains(&"alt"));
    assert!(WINDOWS_MENU_BAR_KEYSTROKES.contains(&"f10"));
}

#[test]
fn a_maximized_window_offers_restore_instead_of_maximize() {
    assert_eq!(linux_caption_icon(CaptionButton::Maximize, false), "square");
    assert_eq!(linux_caption_icon(CaptionButton::Maximize, true), "copy");
}

#[test]
fn the_windows_lights_read_close_minimize_maximize_in_the_macos_colours() {
    assert_eq!(
        TRAFFIC_LIGHT_ORDER,
        [CaptionButton::Close, CaptionButton::Minimize, CaptionButton::Maximize]
    );
    assert_eq!(traffic_light_fill(CaptionButton::Close), 0xFF5F57);
    assert_eq!(traffic_light_fill(CaptionButton::Minimize), 0xFEBC2E);
    assert_eq!(traffic_light_fill(CaptionButton::Maximize), 0x28C840);
}

#[test]
fn the_linux_button_drawings_ship_with_the_app() {
    let lucide = std::path::Path::new(env!("RISE_WORKSPACE_DIR"))
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

#[test]
fn macos_and_windows_keep_their_window_buttons_in_the_rail() {
    assert!(controls_dock_in_rail(HostOs::MacOs));
    assert!(controls_dock_in_rail(HostOs::Windows));
    assert!(!controls_dock_in_rail(HostOs::Linux));
    assert!(lights_in_corner(HostOs::Windows));
    assert!(!lights_in_corner(HostOs::MacOs));
    assert!(!lights_in_corner(HostOs::Linux));
}

#[test]
fn the_drag_strip_leaves_a_hole_for_each_grip() {
    let px = gpui::px;
    let holes = vec![(px(500.0), px(564.0)), (px(100.0), px(164.0)), (px(-10.0), px(20.0))];
    assert_eq!(
        drag_segments(px(800.0), &holes),
        vec![(px(20.0), px(100.0)), (px(164.0), px(500.0)), (px(564.0), px(800.0))]
    );
    assert_eq!(drag_segments(px(800.0), &[]), vec![(px(0.0), px(800.0))]);
}
