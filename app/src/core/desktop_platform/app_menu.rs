use gpui::{
    Action, App, Global, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, Window, actions,
};
use rise_i18n::tr;
use rise_platform::HostOs;
use rise_platform::gpui_shim::{
    full_screen_keystrokes_on, hide_other_apps, hides_other_apps_on, unhide_other_apps,
};
use rise_ui::input_ui::input_actions;

use crate::core::desktop_platform::bug_report_trigger::{ReportBug, install_bug_report_trigger};
use crate::core::desktop_platform::rise_app_quit_drain::request_quit;
use crate::app::shell::shell_actions::{
    self, CloseWindow, GoBack, GoForward, NewWindow, ToggleCommandPalette,
};

actions!(
    rise_app_menu,
    [
        HideApp,
        HideOthers,
        Minimize,
        OpenSettings,
        Quit,
        ShowAll,
        ToggleFullScreen,
        Zoom,
    ]
);

/// one command the menu offers, id is what an in-window menu row carries back
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AppMenuCommand {
    Settings,
    Services,
    HideApp,
    HideOthers,
    ShowAll,
    Quit,
    NewWindow,
    CloseWindow,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    CommandPalette,
    Back,
    Forward,
    ToggleFullScreen,
    Minimize,
    Zoom,
    ReportBug,
}

impl AppMenuCommand {
    pub const ALL: [Self; 21] = [
        Self::Settings,
        Self::Services,
        Self::HideApp,
        Self::HideOthers,
        Self::ShowAll,
        Self::Quit,
        Self::NewWindow,
        Self::CloseWindow,
        Self::Undo,
        Self::Redo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::SelectAll,
        Self::CommandPalette,
        Self::Back,
        Self::Forward,
        Self::ToggleFullScreen,
        Self::Minimize,
        Self::Zoom,
        Self::ReportBug,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Settings => "app_menu.settings",
            Self::Services => "app_menu.services",
            Self::HideApp => "app_menu.hide",
            Self::HideOthers => "app_menu.hide_others",
            Self::ShowAll => "app_menu.show_all",
            Self::Quit => "app_menu.quit",
            Self::NewWindow => "app_menu.new_window",
            Self::CloseWindow => "app_menu.close_window",
            Self::Undo => "app_menu.undo",
            Self::Redo => "app_menu.redo",
            Self::Cut => "app_menu.cut",
            Self::Copy => "app_menu.copy",
            Self::Paste => "app_menu.paste",
            Self::SelectAll => "app_menu.select_all",
            Self::CommandPalette => "app_menu.command_palette",
            Self::Back => "app_menu.back",
            Self::Forward => "app_menu.forward",
            Self::ToggleFullScreen => "app_menu.toggle_full_screen",
            Self::Minimize => "app_menu.minimize",
            Self::Zoom => "app_menu.zoom",
            Self::ReportBug => "app_menu.report_bug",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.id() == id)
    }

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::Settings => "settings_page_title",
            Self::Services => "desktop_menu_services",
            Self::HideApp => "desktop_menu_hide",
            Self::HideOthers => "desktop_menu_hide_others",
            Self::ShowAll => "desktop_menu_show_all",
            Self::Quit => "desktop_menu_quit",
            Self::NewWindow => "desktop_menu_new_window",
            Self::CloseWindow => "desktop_menu_close_window",
            Self::Undo => "desktop_menu_undo",
            Self::Redo => "desktop_menu_redo",
            Self::Cut => "desktop_menu_cut",
            Self::Copy => "copy",
            Self::Paste => "desktop_menu_paste",
            Self::SelectAll => "desktop_menu_select_all",
            Self::CommandPalette => "desktop_menu_command_palette",
            Self::Back => "back",
            Self::Forward => "desktop_menu_forward",
            Self::ToggleFullScreen => "desktop_menu_toggle_full_screen",
            Self::Minimize => "desktop_menu_minimize",
            Self::Zoom => "desktop_menu_zoom",
            Self::ReportBug => "report_riseonly_title",
        }
    }

    /// none only for the services submenu, appkit fills that itself
    pub fn action(self) -> Option<Box<dyn Action>> {
        let action: Box<dyn Action> = match self {
            Self::Services => return None,
            Self::Settings => Box::new(OpenSettings),
            Self::HideApp => Box::new(HideApp),
            Self::HideOthers => Box::new(HideOthers),
            Self::ShowAll => Box::new(ShowAll),
            Self::Quit => Box::new(Quit),
            Self::NewWindow => Box::new(NewWindow),
            Self::CloseWindow => Box::new(CloseWindow),
            Self::Undo => Box::new(input_actions::Undo),
            Self::Redo => Box::new(input_actions::Redo),
            Self::Cut => Box::new(input_actions::Cut),
            Self::Copy => Box::new(input_actions::Copy),
            Self::Paste => Box::new(input_actions::Paste),
            Self::SelectAll => Box::new(input_actions::SelectAll),
            Self::CommandPalette => Box::new(ToggleCommandPalette),
            Self::Back => Box::new(GoBack),
            Self::Forward => Box::new(GoForward),
            Self::ToggleFullScreen => Box::new(ToggleFullScreen),
            Self::Minimize => Box::new(Minimize),
            Self::Zoom => Box::new(Zoom),
            Self::ReportBug => Box::new(ReportBug),
        };
        Some(action)
    }

    /// appkit routes these through the first responder so a native text field still gets them
    pub const fn os_action(self) -> Option<OsAction> {
        match self {
            Self::Undo => Some(OsAction::Undo),
            Self::Redo => Some(OsAction::Redo),
            Self::Cut => Some(OsAction::Cut),
            Self::Copy => Some(OsAction::Copy),
            Self::Paste => Some(OsAction::Paste),
            Self::SelectAll => Some(OsAction::SelectAll),
            _ => None,
        }
    }

    pub const fn is_offered_on(self, host: HostOs) -> bool {
        match self {
            Self::Services | Self::HideApp | Self::HideOthers | Self::ShowAll => {
                hides_other_apps_on(host)
            }
            _ => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppMenuEntry {
    Command(AppMenuCommand),
    Separator,
}

/// one top level menu of the catalogue
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AppMenuSectionSpec {
    pub title_key: &'static str,
    pub entries: &'static [AppMenuEntry],
}

/// one top level menu as a given desktop shows it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppMenuSection {
    pub title_key: &'static str,
    pub entries: Vec<AppMenuEntry>,
}

use AppMenuEntry::{Command as C, Separator as S};

pub const APP_MENU: [AppMenuSectionSpec; 6] = [
    AppMenuSectionSpec {
        title_key: "app_name",
        entries: &[
            C(AppMenuCommand::Settings),
            S,
            C(AppMenuCommand::Services),
            S,
            C(AppMenuCommand::HideApp),
            C(AppMenuCommand::HideOthers),
            C(AppMenuCommand::ShowAll),
            S,
            C(AppMenuCommand::Quit),
        ],
    },
    AppMenuSectionSpec {
        title_key: "desktop_menu_file",
        entries: &[
            C(AppMenuCommand::NewWindow),
            C(AppMenuCommand::CloseWindow),
        ],
    },
    AppMenuSectionSpec {
        title_key: "desktop_menu_edit",
        entries: &[
            C(AppMenuCommand::Undo),
            C(AppMenuCommand::Redo),
            S,
            C(AppMenuCommand::Cut),
            C(AppMenuCommand::Copy),
            C(AppMenuCommand::Paste),
            C(AppMenuCommand::SelectAll),
        ],
    },
    AppMenuSectionSpec {
        title_key: "desktop_menu_view",
        entries: &[
            C(AppMenuCommand::CommandPalette),
            S,
            C(AppMenuCommand::Back),
            C(AppMenuCommand::Forward),
            S,
            C(AppMenuCommand::ToggleFullScreen),
        ],
    },
    AppMenuSectionSpec {
        title_key: "desktop_menu_window",
        entries: &[C(AppMenuCommand::Minimize), C(AppMenuCommand::Zoom)],
    },
    AppMenuSectionSpec {
        title_key: "desktop_menu_help",
        entries: &[C(AppMenuCommand::ReportBug)],
    },
];

/// catalogue for host, missing commands removed then separators left at an edge or doubled
pub fn app_menu_sections(host: HostOs) -> Vec<AppMenuSection> {
    APP_MENU
        .iter()
        .map(|spec| AppMenuSection {
            title_key: spec.title_key,
            entries: tidy_separators(spec.entries.iter().copied().filter(|entry| match entry {
                AppMenuEntry::Command(command) => command.is_offered_on(host),
                AppMenuEntry::Separator => true,
            })),
        })
        .filter(|section| !section.entries.is_empty())
        .collect()
}

pub fn tidy_separators(entries: impl IntoIterator<Item = AppMenuEntry>) -> Vec<AppMenuEntry> {
    let mut tidy: Vec<AppMenuEntry> = Vec::new();
    for entry in entries {
        if entry == AppMenuEntry::Separator
            && tidy.last().is_none_or(|last| *last == AppMenuEntry::Separator)
        {
            continue;
        }
        tidy.push(entry);
    }
    if tidy.last() == Some(&AppMenuEntry::Separator) {
        tidy.pop();
    }
    tidy
}

/// gpui menus for host with labels in the active language
pub fn app_menus(host: HostOs) -> Vec<Menu> {
    app_menu_sections(host)
        .into_iter()
        .map(|section| {
            Menu::new(tr(section.title_key)).items(section.entries.into_iter().filter_map(menu_item))
        })
        .collect()
}

fn menu_item(entry: AppMenuEntry) -> Option<MenuItem> {
    let command = match entry {
        AppMenuEntry::Separator => return Some(MenuItem::separator()),
        AppMenuEntry::Command(command) => command,
    };
    let name = tr(command.label_key());

    if command == AppMenuCommand::Services {
        return Some(MenuItem::os_submenu(name, SystemMenuType::Services));
    }

    let action = command.action()?;
    Some(MenuItem::Action {
        name: name.into(),
        action,
        os_action: command.os_action(),
        checked: false,
        disabled: false,
    })
}

/// app wide shortcuts
/// edit and navigation keys stay in the input and shell key maps
pub fn app_menu_key_bindings(host: HostOs) -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-m", Minimize, None),
    ];
    bindings.extend(
        full_screen_keystrokes_on(host)
            .iter()
            .map(|keystrokes| KeyBinding::new(keystrokes, ToggleFullScreen, None)),
    );
    if hides_other_apps_on(host) {
        bindings.push(KeyBinding::new("secondary-h", HideApp, None));
        bindings.push(KeyBinding::new("alt-secondary-h", HideOthers, None));
    }
    bindings
}

struct AppMenuInstalled;

impl Global for AppMenuInstalled {}

/// safe to call twice, binds before set_menus because macos reads each shortcut at that moment
pub fn install_app_menu(cx: &mut App) {
    if cx.has_global::<AppMenuInstalled>() {
        return;
    }
    cx.set_global(AppMenuInstalled);

    let host = HostOs::current();
    input_actions::install_key_bindings(cx);
    shell_actions::install_key_bindings(cx);
    crate::modules::chat::components::chat::composer::chat_composer_text_view::install_key_bindings(cx);
    cx.bind_keys(app_menu_key_bindings(host));
    install_bug_report_trigger(cx);

    cx.on_action(|_: &Quit, cx| request_quit(cx));
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| {
        hide_other_apps(cx);
    });
    cx.on_action(|_: &ShowAll, cx| {
        unhide_other_apps(cx);
    });
    cx.on_action(|_: &Minimize, cx| with_active_window(cx, Window::minimize_window));
    cx.on_action(|_: &Zoom, cx| with_active_window(cx, Window::zoom_window));
    cx.on_action(|_: &ToggleFullScreen, cx| with_active_window(cx, Window::toggle_fullscreen));

    cx.set_menus(app_menus(host));
}

pub fn refresh_app_menu(cx: &App) {
    if cx.has_global::<AppMenuInstalled>() {
        cx.set_menus(app_menus(HostOs::current()));
    }
}

fn with_active_window(cx: &mut App, apply: fn(&Window)) {
    if let Some(handle) = cx.active_window() {
        handle.update(cx, |_, window, _| apply(window)).ok();
    }
}
