use std::collections::HashSet;

use gpui::TestAppContext;
use rise_platform::HostOs;
use rise_platform::gpui_shim::full_screen_keystrokes_on;

use gpui::{Keystroke, Modifiers};

use super::app_menu_bar::{
    AppMenuBarKey, adjacent_section, menu_bar_key, mnemonic_candidates, mnemonic_of,
    section_for_mnemonic,
};
use super::app_menu::{
    APP_MENU, AppMenuCommand, AppMenuEntry, Quit, app_menu_key_bindings, app_menu_sections,
    ToggleFullScreen, app_menus, install_app_menu, tidy_separators,
};

fn commands_on(host: HostOs) -> Vec<AppMenuCommand> {
    app_menu_sections(host)
        .into_iter()
        .flat_map(|section| section.entries)
        .filter_map(|entry| match entry {
            AppMenuEntry::Command(command) => Some(command),
            AppMenuEntry::Separator => None,
        })
        .collect()
}

#[test]
fn every_command_id_is_unique_and_reads_back() {
    let mut seen = HashSet::new();
    for command in AppMenuCommand::ALL {
        assert!(seen.insert(command.id()), "{command:?} shares its id");
        assert_eq!(AppMenuCommand::from_id(command.id()), Some(command));
    }
    assert_eq!(AppMenuCommand::from_id("app_menu.unknown"), None);
}

#[test]
fn the_catalogue_names_every_command_exactly_once() {
    let mut listed = Vec::new();
    for spec in APP_MENU {
        for entry in spec.entries {
            if let AppMenuEntry::Command(command) = entry {
                listed.push(*command);
            }
        }
    }
    assert_eq!(listed.len(), AppMenuCommand::ALL.len());
    for command in AppMenuCommand::ALL {
        assert!(listed.contains(&command), "{command:?} is in no menu");
    }
}

#[test]
fn only_the_services_submenu_lacks_an_action() {
    for command in AppMenuCommand::ALL {
        assert_eq!(
            command.action().is_none(),
            command == AppMenuCommand::Services,
            "{command:?}"
        );
    }
}

#[test]
fn edit_commands_reach_the_first_responder() {
    for command in [
        AppMenuCommand::Undo,
        AppMenuCommand::Redo,
        AppMenuCommand::Cut,
        AppMenuCommand::Copy,
        AppMenuCommand::Paste,
        AppMenuCommand::SelectAll,
    ] {
        assert!(command.os_action().is_some(), "{command:?}");
    }
    assert!(AppMenuCommand::Quit.os_action().is_none());
}

#[test]
fn hiding_and_services_appear_only_on_macos() {
    let appkit_only = [
        AppMenuCommand::Services,
        AppMenuCommand::HideApp,
        AppMenuCommand::HideOthers,
        AppMenuCommand::ShowAll,
    ];

    let mac = commands_on(HostOs::MacOs);
    for command in appkit_only {
        assert!(mac.contains(&command), "{command:?} missing on macOS");
    }
    for host in [HostOs::Linux, HostOs::Windows] {
        let commands = commands_on(host);
        for command in appkit_only {
            assert!(!commands.contains(&command), "{command:?} offered on {host:?}");
        }
        assert!(commands.contains(&AppMenuCommand::Quit));
        assert!(commands.contains(&AppMenuCommand::Settings));
    }
}

#[test]
fn no_menu_starts_ends_or_doubles_a_separator() {
    for host in HostOs::ALL {
        for section in app_menu_sections(host) {
            let entries = &section.entries;
            assert_ne!(entries.first(), Some(&AppMenuEntry::Separator), "{host:?}");
            assert_ne!(entries.last(), Some(&AppMenuEntry::Separator), "{host:?}");
            for pair in entries.windows(2) {
                assert!(
                    pair != [AppMenuEntry::Separator, AppMenuEntry::Separator],
                    "{host:?} {}",
                    section.title_key
                );
            }
        }
    }
}

#[test]
fn tidy_separators_drops_edges_and_runs() {
    let s = AppMenuEntry::Separator;
    let quit = AppMenuEntry::Command(AppMenuCommand::Quit);
    let zoom = AppMenuEntry::Command(AppMenuCommand::Zoom);

    assert_eq!(tidy_separators([s, quit, s, s, zoom, s]), vec![quit, s, zoom]);
    assert_eq!(tidy_separators([s, s]), Vec::new());
}

#[test]
fn every_host_gets_the_same_menus() {
    for host in HostOs::ALL {
        assert_eq!(app_menu_sections(host).len(), APP_MENU.len(), "{host:?}");
        assert_eq!(app_menus(host).len(), APP_MENU.len(), "{host:?}");
    }
}

#[test]
fn hide_shortcuts_are_bound_only_on_macos() {
    let full_screen = |host| full_screen_keystrokes_on(host).len();
    let mac = app_menu_key_bindings(HostOs::MacOs).len() - full_screen(HostOs::MacOs);
    for host in [HostOs::Linux, HostOs::Windows] {
        assert_eq!(app_menu_key_bindings(host).len() - full_screen(host) + 2, mac, "{host:?}");
    }
}

#[test]
fn full_screen_shortcut_follows_the_host() {
    let spelled = |binding: &gpui::KeyBinding| {
        let keys: Vec<String> = binding.keystrokes().iter().map(|key| key.unparse()).collect();
        keys.join(" ")
    };
    let has = |host, keystrokes: &str| {
        let wanted = spelled(&gpui::KeyBinding::new(keystrokes, ToggleFullScreen, None));
        app_menu_key_bindings(host).iter().any(|binding| spelled(binding) == wanted)
    };
    assert!(has(HostOs::MacOs, "ctrl-cmd-f"));
    assert!(!has(HostOs::MacOs, "f11"));
    for host in [HostOs::Linux, HostOs::Windows] {
        assert!(has(host, "f11"), "{host:?}");
    }
}

#[gpui::test]
fn install_binds_quit_and_is_idempotent(cx: &mut TestAppContext) {
    cx.update(|cx| {
        install_app_menu(cx);
        install_app_menu(cx);

        let keymap = cx.key_bindings();
        assert_eq!(keymap.borrow().bindings_for_action(&Quit).count(), 1);
    });
}

fn keystroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
    Keystroke {
        modifiers,
        key: key.to_owned(),
        key_char: key_char.map(str::to_owned),
    }
}

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Modifiers::default()
    }
}

#[test]
fn alt_with_a_letter_is_a_mnemonic_whether_or_not_a_menu_is_open() {
    let alt_f = keystroke("f", None, alt());
    assert_eq!(menu_bar_key(&alt_f, false), Some(AppMenuBarKey::Mnemonic));
    assert_eq!(menu_bar_key(&alt_f, true), Some(AppMenuBarKey::Mnemonic));

    let lone_alt = keystroke("alt", None, alt());
    assert_eq!(menu_bar_key(&lone_alt, false), None);

    let ctrl_alt_f = keystroke(
        "f",
        None,
        Modifiers {
            control: true,
            ..alt()
        },
    );
    assert_eq!(menu_bar_key(&ctrl_alt_f, false), None);
}

#[test]
fn arrows_walk_between_menus_only_while_one_is_open() {
    let left = keystroke("left", None, Modifiers::none());
    let right = keystroke("right", None, Modifiers::none());
    assert_eq!(menu_bar_key(&left, true), Some(AppMenuBarKey::Previous));
    assert_eq!(menu_bar_key(&right, true), Some(AppMenuBarKey::Next));
    assert_eq!(menu_bar_key(&left, false), None);
    assert_eq!(menu_bar_key(&keystroke("down", None, Modifiers::none()), true), None);

    let shift_right = keystroke(
        "right",
        None,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(menu_bar_key(&shift_right, true), None);
}

#[test]
fn adjacent_menus_wrap_at_both_ends() {
    assert_eq!(adjacent_section(0, 5, AppMenuBarKey::Previous), 4);
    assert_eq!(adjacent_section(4, 5, AppMenuBarKey::Next), 0);
    assert_eq!(adjacent_section(2, 5, AppMenuBarKey::Next), 3);
    assert_eq!(adjacent_section(0, 0, AppMenuBarKey::Next), 0);
}

#[test]
fn a_mnemonic_is_the_first_letter_of_the_title_in_any_script() {
    assert_eq!(mnemonic_of("File"), Some('f'));
    assert_eq!(mnemonic_of("\u{0424}\u{0430}\u{0439}\u{043b}"), Some('\u{0444}'));
    assert_eq!(mnemonic_of("  &Edit"), Some('e'));
    assert_eq!(mnemonic_of(""), None);

    let cyrillic_layout = keystroke("a", Some("\u{0444}"), alt());
    assert_eq!(mnemonic_candidates(&cyrillic_layout), vec!['a', '\u{0444}']);
}

#[test]
fn titles_sharing_a_letter_take_turns() {
    let mnemonics = [Some('r'), Some('f'), Some('e'), Some('f'), None];
    assert_eq!(section_for_mnemonic(&mnemonics, &['f'], None), Some(1));
    assert_eq!(section_for_mnemonic(&mnemonics, &['f'], Some(1)), Some(3));
    assert_eq!(section_for_mnemonic(&mnemonics, &['f'], Some(3)), Some(1));
    assert_eq!(section_for_mnemonic(&mnemonics, &['x'], None), None);
    assert_eq!(section_for_mnemonic(&[], &['f'], None), None);
}
