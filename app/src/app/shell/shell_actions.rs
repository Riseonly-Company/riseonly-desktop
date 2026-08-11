use gpui::{App, Global, KeyBinding, actions};
use rise_navigation::RootTab;

pub const KEY_CONTEXT: &str = "Shell";

pub const PALETTE_KEY_CONTEXT: &str = "CommandPalette";

actions!(
    rise_shell,
    [
        CloseWindow,
        ConfirmSelection,
        DismissOverlay,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShortcutTarget {
    Section(RootTab),
    Folder(usize),
}

pub const SHORTCUT_SLOTS: u8 = 9;

pub fn shortcut_target(slot: u8, folders: usize) -> Option<ShortcutTarget> {
    if slot == 0 || slot > SHORTCUT_SLOTS {
        return None;
    }

    let index = usize::from(slot - 1);
    match RootTab::ALL.get(index) {
        Some(tab) => Some(ShortcutTarget::Section(*tab)),
        None => {
            let folder = index - RootTab::ALL.len();
            (folder < folders).then_some(ShortcutTarget::Folder(folder))
        }
    }
}

struct KeyBindingsInstalled;

impl Global for KeyBindingsInstalled {}

pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<KeyBindingsInstalled>() {
        return;
    }
    cx.set_global(KeyBindingsInstalled);

    let shell = Some(KEY_CONTEXT);
    let palette = Some(PALETTE_KEY_CONTEXT);

    cx.bind_keys([
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
        KeyBinding::new("escape", DismissOverlay, shell),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_slots_are_the_sections_in_rail_order() {
        for (index, tab) in RootTab::ALL.iter().enumerate() {
            let slot = u8::try_from(index + 1).expect("the sections fit in a byte");
            assert_eq!(
                shortcut_target(slot, 0),
                Some(ShortcutTarget::Section(*tab)),
                "Cmd+{slot} must reach the section the rail draws {index} places down"
            );
        }
    }

    #[test]
    fn the_remaining_slots_address_folders_in_the_order_they_are_stacked() {
        let first_folder = u8::try_from(RootTab::ALL.len() + 1).unwrap();
        let folders = usize::from(SHORTCUT_SLOTS) - RootTab::ALL.len();

        assert_eq!(
            shortcut_target(first_folder, folders),
            Some(ShortcutTarget::Folder(0))
        );
        assert_eq!(
            shortcut_target(SHORTCUT_SLOTS, folders),
            Some(ShortcutTarget::Folder(folders - 1))
        );
    }

    #[test]
    fn a_slot_past_the_last_folder_does_nothing_rather_than_guessing() {
        let first_folder = u8::try_from(RootTab::ALL.len() + 1).unwrap();

        assert_eq!(shortcut_target(first_folder, 0), None);
        assert_eq!(shortcut_target(SHORTCUT_SLOTS, 0), None);
    }

    #[test]
    fn the_rail_now_leaves_room_for_two_folder_shortcuts() {
        assert_eq!(usize::from(SHORTCUT_SLOTS) - RootTab::ALL.len(), 2);
    }

    #[test]
    fn slots_outside_the_keyboard_row_are_rejected() {
        assert_eq!(shortcut_target(0, 9), None);
        assert_eq!(shortcut_target(10, 99), None);
        assert_eq!(shortcut_target(u8::MAX, 99), None);
    }

    #[test]
    fn a_shortcut_never_addresses_two_things() {
        let mut seen = Vec::new();
        for slot in 1..=SHORTCUT_SLOTS {
            if let Some(target) = shortcut_target(slot, 9) {
                assert!(!seen.contains(&target), "{target:?} has two shortcuts");
                seen.push(target);
            }
        }
        assert_eq!(seen.len(), usize::from(SHORTCUT_SLOTS));
    }
}
