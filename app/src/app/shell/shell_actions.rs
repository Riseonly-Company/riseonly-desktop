use gpui::{App, Entity, Global, KeyBinding, Window, actions};
use rise_ui::swiftui::binding::Binding;

use crate::app::router::navigation_fullscreen_pop::{NavigationFullscreenPop, NavigationInteractivePopGuard};
use crate::app::router::navigation_store::NavigationStore;
use crate::app::router::navigation_types::MainTab;
use crate::core::desktop_platform::bug_report_trigger::{REPORT_BUG_KEYSTROKE, ReportBug};
use crate::core::widgets::navigations::main_tab_bar::MainTabBarItem;
use crate::core::widgets::navigations::main_tab_scroll_to_top::MainTabScrollToTop;

pub use rise_widgets::side_panel::side_panel_ui::ToggleSidePanel;

pub use crate::modules::rtc::pages::shell_window_actions_hooks::{CloseWindow, KEY_CONTEXT};

pub const PALETTE_KEY_CONTEXT: &str = "CommandPalette";

actions!(
    rise_shell,
    [
        ConfirmSelection,
        GoBack,
        GoForward,
        NewWindow,
        SelectNextItem,
        SelectPreviousItem,
        SelectSlot1,
        SelectSlot2,
        SelectSlot3,
        SelectSlot4,
        SelectSlot5,
        SelectSlot6,
        SelectSlot7,
        SelectSlot8,
        SelectSlot9,
        ToggleCommandPalette,
    ]
);

#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
actions!(rise_shell, [ToggleSaiDebugger]);

pub const SHORTCUT_SLOTS: u8 = 9;

/// tab behind cmd or ctrl plus slot, the bar buttons in order
/// a slot past the last button does nothing
pub fn shortcut_target(slot: u8) -> Option<MainTab> {
    if slot == 0 || slot > SHORTCUT_SLOTS {
        return None;
    }
    MainTabBarItem::all().get(usize::from(slot - 1)).map(|item| item.id)
}

/// tab picked from keyboard or palette, same as clicking its bar button
/// picking the open tab reselects it so list to top and feed refresh and riffs to first riff
pub fn select_main_tab(
    tab: MainTab,
    current_tab: &Binding<MainTab>,
    nav_store: &Entity<NavigationStore>,
    window: &mut Window,
    cx: &mut App,
) {
    let current = current_tab.get(cx);
    let picked = nav_store.update(cx, |nav_store, cx| nav_store.pick_tab(tab, cx));
    if tab == current {
        if !picked {
            MainTabScrollToTop::reselect(tab, window, cx);
        }
        return;
    }
    current_tab.set(tab, window, cx);
}

pub fn select_slot(
    slot: u8,
    current_tab: &Binding<MainTab>,
    nav_store: &Entity<NavigationStore>,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(tab) = shortcut_target(slot) {
        select_main_tab(tab, current_tab, nav_store, window, cx);
    }
}

pub fn go_back(window: &mut Window, cx: &mut App) -> bool {
    NavigationFullscreenPop::active_delegate(cx).is_some_and(|delegate| delegate.request_back(window, cx))
}

pub fn go_forward(window: &mut Window, cx: &mut App) -> bool {
    NavigationInteractivePopGuard::is_right_to_left(cx) && go_back(window, cx)
}

struct KeyBindingsInstalled;

impl Global for KeyBindingsInstalled {}

pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);

    rise_widgets::side_panel::side_panel_ui::install_key_bindings(cx);

    let shell = Some(KEY_CONTEXT);
    let palette = Some(PALETTE_KEY_CONTEXT);

    cx.bind_keys([
        KeyBinding::new(REPORT_BUG_KEYSTROKE, ReportBug, shell),
        KeyBinding::new("secondary-1", SelectSlot1, shell),
        KeyBinding::new("secondary-2", SelectSlot2, shell),
        KeyBinding::new("secondary-3", SelectSlot3, shell),
        KeyBinding::new("secondary-4", SelectSlot4, shell),
        KeyBinding::new("secondary-5", SelectSlot5, shell),
        KeyBinding::new("secondary-6", SelectSlot6, shell),
        KeyBinding::new("secondary-7", SelectSlot7, shell),
        KeyBinding::new("secondary-8", SelectSlot8, shell),
        KeyBinding::new("secondary-9", SelectSlot9, shell),
        KeyBinding::new("secondary-[", GoBack, shell),
        KeyBinding::new("secondary-]", GoForward, shell),
        KeyBinding::new("alt-left", GoBack, shell),
        KeyBinding::new("alt-right", GoForward, shell),
        KeyBinding::new("secondary-k", ToggleCommandPalette, shell),
        KeyBinding::new("secondary-shift-p", ToggleCommandPalette, shell),
        KeyBinding::new("secondary-n", NewWindow, shell),
        KeyBinding::new("secondary-w", CloseWindow, shell),
        KeyBinding::new("down", SelectNextItem, palette),
        KeyBinding::new("up", SelectPreviousItem, palette),
        KeyBinding::new("ctrl-n", SelectNextItem, palette),
        KeyBinding::new("ctrl-p", SelectPreviousItem, palette),
        KeyBinding::new("enter", ConfirmSelection, palette),
    ]);

    #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
    cx.bind_keys([KeyBinding::new("secondary-shift-d", ToggleSaiDebugger, shell)]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_slots_are_the_bar_buttons_in_their_order() {
        for (index, item) in MainTabBarItem::all().iter().enumerate() {
            let slot = u8::try_from(index + 1).expect("the bar fits in a byte");
            assert_eq!(shortcut_target(slot), Some(item.id), "Cmd+{slot} must reach the button {index} places down");
        }
    }

    #[test]
    fn a_slot_past_the_last_button_does_nothing_rather_than_guessing() {
        let first_free = u8::try_from(MainTabBarItem::all().len() + 1).expect("the bar fits in a byte");
        for slot in first_free..=SHORTCUT_SLOTS {
            assert_eq!(shortcut_target(slot), None);
        }
    }

    #[test]
    fn slots_outside_the_keyboard_row_are_rejected() {
        assert_eq!(shortcut_target(0), None);
        assert_eq!(shortcut_target(10), None);
        assert_eq!(shortcut_target(u8::MAX), None);
    }

    #[test]
    fn a_shortcut_never_addresses_two_things() {
        let mut seen = Vec::new();
        for slot in 1..=SHORTCUT_SLOTS {
            if let Some(tab) = shortcut_target(slot) {
                assert!(!seen.contains(&tab), "{tab:?} has two shortcuts");
                seen.push(tab);
            }
        }
    }
}
