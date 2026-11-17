use std::collections::HashSet;

use super::global_hotkey::{Hotkey, KeyCode, Modifiers};
use super::shortcut_routes::*;

fn chord(raw: &str) -> Hotkey {
    match Hotkey::parse(raw) {
        Ok(hotkey) => hotkey,
        Err(error) => panic!("{raw:?} should parse: {error}"),
    }
}

fn small_keymap() -> KeyboardMap {
    KeyboardMap::new(
        8,
        2,
        vec![
            0x0061, 0x0041,
            XK_ALT_L, 0,
            XK_SUPER_L, 0,
            XK_NUM_LOCK, 0,
            0x0062, 0x0042,
            0x0041, 0,
            XK_SUPER_R, 0,
            0xFFBE, 0,
        ],
    )
}

fn mapping(mod1: u8, mod2: u8, mod3: u8, mod4: u8) -> Vec<u8> {
    vec![0, 0, 0, mod1, mod2, mod3, mod4, 0]
}

#[test]
fn every_key_has_a_distinct_xkb_name() {
    let mut seen = HashSet::new();
    for key in KeyCode::all() {
        let name = xkb_key_name(key);
        assert!(!name.is_empty());
        assert!(seen.insert(name), "{name} named twice");
    }
}

#[test]
fn the_trigger_puts_modifiers_first_in_the_spec_spelling() {
    assert_eq!(preferred_trigger(chord("Ctrl+Shift+R")), "CTRL+SHIFT+r");
    assert_eq!(preferred_trigger(chord("Super+Alt+F5")), "ALT+LOGO+F5");
    assert_eq!(preferred_trigger(chord("Ctrl+PageDown")), "CTRL+Page_Down");
    assert_eq!(preferred_trigger(chord("Ctrl+-")), "CTRL+minus");
}

#[test]
fn a_portal_shortcut_is_named_by_its_action() {
    let shortcut = PortalShortcut::new("toggle_window", chord("Ctrl+Alt+R"));
    assert_eq!(shortcut.id, "toggle_window");
    assert_eq!(shortcut.preferred_trigger, "CTRL+ALT+r");
}

#[test]
fn a_keysym_resolves_to_the_lowest_keycode_of_the_first_column_holding_it() {
    let keymap = small_keymap();
    assert_eq!(keymap.keycode_for(0x0061), Some(8));
    assert_eq!(keymap.keycode_for(0xFFBE), Some(15));
    assert_eq!(
        keymap.keycode_for(0x0041),
        Some(13),
        "column 0 is searched across every keycode before column 1"
    );
    assert_eq!(keymap.keycode_for(0x0063), None);
    assert_eq!(keymap.keycode_for(0), None, "NoSymbol is never a key");
}

#[test]
fn a_keycode_outside_the_map_has_no_keysyms() {
    let keymap = small_keymap();
    assert_eq!(keymap.keysyms(8), &[0x0061, 0x0041]);
    assert!(keymap.keysyms(7).is_empty());
    assert!(keymap.keysyms(16).is_empty());
    assert!(KeyboardMap::default().keysyms(8).is_empty());
}

#[test]
fn the_stock_mapping_puts_alt_super_and_num_lock_where_xkb_does() {
    let layout = ModifierLayout::from_mapping(1, &mapping(9, 11, 0, 10), &small_keymap());
    assert_eq!(layout, ModifierLayout::CONVENTIONAL);
}

#[test]
fn a_moved_super_key_is_read_from_the_mapping_not_assumed() {
    let layout = ModifierLayout::from_mapping(1, &mapping(9, 11, 14, 0), &small_keymap());
    assert_eq!(layout.super_key, 0x0020, "Super on Mod3");
    assert_eq!(layout.mask(chord("Super+R").modifiers), 0x0020);
}

#[test]
fn an_unnamed_modifier_falls_back_to_the_stock_bit() {
    let layout = ModifierLayout::from_mapping(1, &mapping(0, 0, 0, 0), &small_keymap());
    assert_eq!(layout, ModifierLayout::CONVENTIONAL);
    assert_eq!(ModifierLayout::from_mapping(0, &[], &small_keymap()), ModifierLayout::CONVENTIONAL);
}

#[test]
fn the_mask_combines_the_layout_bits() {
    let layout = ModifierLayout::CONVENTIONAL;
    assert_eq!(layout.mask(Modifiers::NONE), 0);
    assert_eq!(layout.mask(chord("Ctrl+Shift+R").modifiers), CONTROL_MASK | SHIFT_MASK);
    assert_eq!(layout.mask(chord("Super+Alt+R").modifiers), 0x0040 | 0x0008);
}

#[test]
fn one_grab_per_lock_combination_so_caps_and_num_lock_do_not_hide_the_chord() {
    let layout = ModifierLayout::CONVENTIONAL;
    let base = CONTROL_MASK | SHIFT_MASK;
    let masks = layout.grab_masks(base);

    assert_eq!(masks.len(), 4);
    for extra in [0, LOCK_MASK, 0x0010, LOCK_MASK | 0x0010] {
        assert!(masks.contains(&(base | extra)), "missing {extra:#x}");
    }
}

#[test]
fn num_lock_sharing_a_chord_bit_is_never_ignored() {
    let layout = ModifierLayout {
        alt: 0x0008,
        super_key: 0x0010,
        num_lock: 0x0010,
    };
    assert_eq!(layout.ignored(), LOCK_MASK);
    assert_eq!(layout.grab_masks(CONTROL_MASK).len(), 2);
    assert!(!layout.matches(CONTROL_MASK | 0x0010, CONTROL_MASK));
}

#[test]
fn a_press_matches_through_locks_and_pointer_buttons_but_not_extra_modifiers() {
    let layout = ModifierLayout::CONVENTIONAL;
    let base = CONTROL_MASK;
    let button_one = 0x0100;

    assert!(layout.matches(base, base));
    assert!(layout.matches(base | LOCK_MASK | 0x0010, base));
    assert!(layout.matches(base | button_one, base));
    assert!(!layout.matches(base | SHIFT_MASK, base));
    assert!(!layout.matches(0, base));
}

#[test]
fn a_repeated_key_counts_once() {
    let mut filter = RepeatFilter::default();
    assert!(filter.pressed(8, 100));
    filter.released(8, 140);
    assert!(!filter.pressed(8, 140), "same timestamp as the release: auto-repeat");
    filter.released(8, 500);
    assert!(filter.pressed(8, 620), "a new press after a real release");
    filter.released(9, 700);
    assert!(filter.pressed(8, 700), "another key's release says nothing about this one");
}

#[test]
fn request_tokens_use_only_allowed_characters() {
    for token in [request_token(7), session_token(7)] {
        assert!(token.chars().all(|character| character.is_ascii_alphanumeric() || character == '_'));
    }
    assert_ne!(request_token(1), request_token(2));
}

#[test]
fn the_request_path_is_known_before_the_call() {
    assert_eq!(
        request_path(":1.42", "riseonly_hotkey_3"),
        "/org/freedesktop/portal/desktop/request/1_42/riseonly_hotkey_3"
    );
}

#[test]
fn response_codes_follow_the_request_spec() {
    assert_eq!(portal_answer(0), PortalAnswer::Accepted);
    assert_eq!(portal_answer(1), PortalAnswer::Cancelled);
    assert_eq!(portal_answer(2), PortalAnswer::Ended(2));
}

#[test]
fn only_the_live_session_and_a_wanted_shortcut_produce_a_press() {
    let wanted = [PortalShortcut::new("toggle_window", chord("Ctrl+Alt+R"))];
    let live = "/org/freedesktop/portal/desktop/session/1_42/riseonly_hotkeys_2";

    assert_eq!(
        activated_action(Some(live), live, "toggle_window", &wanted),
        Some("toggle_window".to_owned())
    );
    assert_eq!(activated_action(Some(live), "/old/session", "toggle_window", &wanted), None);
    assert_eq!(activated_action(None, live, "toggle_window", &wanted), None);
    assert_eq!(activated_action(Some(live), live, "released_action", &wanted), None);
}

#[test]
fn the_press_queue_keeps_the_newest_presses() {
    let mut queue = Vec::new();
    for index in 0..5 {
        push_bounded(&mut queue, format!("action_{index}"), 3);
    }
    assert_eq!(queue, ["action_2", "action_3", "action_4"]);

    let mut unused = Vec::new();
    push_bounded(&mut unused, "ignored".to_owned(), 0);
    assert!(unused.is_empty());
}
