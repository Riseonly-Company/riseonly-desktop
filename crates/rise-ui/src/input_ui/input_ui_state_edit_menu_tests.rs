use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, Context, TestAppContext, Window, WindowHandle, hsla, point, px};
use rise_theme::AppTheme;

use crate::context_menu_ui::ContextMenuItem;
use crate::input_ui::*;

fn open(cx: &mut TestAppContext) -> WindowHandle<InputUiState> {
    cx.update(|cx| {
        if !cx.has_global::<AppTheme>() {
            crate::install_theme(AppTheme::dark(), cx);
        }
    });
    cx.add_window(|_window, cx| InputUiState::new(InputMode::MultiLine, cx))
}

fn edit<R>(
    handle: &WindowHandle<InputUiState>,
    cx: &mut TestAppContext,
    f: impl FnOnce(&mut InputUiState, &mut Window, &mut Context<InputUiState>) -> R,
) -> R {
    handle.update(cx, f).expect("window is open")
}

fn submenu() -> InputEditSubmenu {
    InputEditSubmenu {
        title: "format".to_owned(),
        icon: None,
        items: vec![ContextMenuItem::new(0, "bold", None, "bold", |_, _| {})],
    }
}

#[gpui::test]
fn an_empty_field_offers_only_paste(cx: &mut TestAppContext) {
    let field = open(cx);
    let commands = edit(&field, cx, |state, _, _| state.edit_menu_commands());
    assert_eq!(commands, vec![InputEditMenuCommand::Paste]);
}

#[gpui::test]
fn a_selection_brings_cut_copy_and_the_submenu_at_index_one(cx: &mut TestAppContext) {
    let field = open(cx);
    let commands = edit(&field, cx, |state, _, cx| {
        state.reset("hello", cx);
        state.set_edit_submenu(Some(submenu()), cx);
        state.set_selection(Selection::new(0, 5), cx);
        state.edit_menu_commands()
    });
    assert_eq!(
        commands,
        vec![
            InputEditMenuCommand::Cut,
            InputEditMenuCommand::Submenu,
            InputEditMenuCommand::Copy,
            InputEditMenuCommand::Paste,
            InputEditMenuCommand::SelectAll,
        ]
    );
}

#[gpui::test]
fn the_submenu_hides_without_a_selection(cx: &mut TestAppContext) {
    let field = open(cx);
    let commands = edit(&field, cx, |state, _, cx| {
        state.reset("hello", cx);
        state.set_edit_submenu(Some(submenu()), cx);
        state.edit_menu_commands()
    });
    assert!(!commands.contains(&InputEditMenuCommand::Submenu));
    assert!(!commands.contains(&InputEditMenuCommand::Copy));
}

#[gpui::test]
fn a_secure_field_never_offers_copy_or_cut(cx: &mut TestAppContext) {
    let field = open(cx);
    let commands = edit(&field, cx, |state, _, cx| {
        state.reset("secret", cx);
        state.set_secure(true, cx);
        state.set_selection(Selection::new(0, 6), cx);
        state.edit_menu_commands()
    });
    assert!(!commands.contains(&InputEditMenuCommand::Copy));
    assert!(!commands.contains(&InputEditMenuCommand::Cut));
}

#[gpui::test]
fn opening_and_closing_the_menu_reports_once_each(cx: &mut TestAppContext) {
    let field = open(cx);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let sink = seen.clone();
    edit(&field, cx, |state, window, cx| {
        state.set_on_edit_menu_presented_change(Some(Rc::new(move |shown: bool, _: &mut Window, _: &mut App| {
            sink.borrow_mut().push(shown);
        })));
        state.open_edit_menu(point(px(1.), px(1.)), window, cx);
        state.open_edit_menu(point(px(2.), px(2.)), window, cx);
        state.close_edit_menu(window, cx);
        state.close_edit_menu(window, cx);
    });
    cx.run_until_parked();
    assert_eq!(*seen.borrow(), vec![true, false]);
}

#[gpui::test]
fn the_submenu_row_opens_the_submenu_in_place(cx: &mut TestAppContext) {
    let field = open(cx);
    let presentation = edit(&field, cx, |state, window, cx| {
        state.set_edit_submenu(Some(submenu()), cx);
        state.open_edit_menu(point(px(1.), px(1.)), window, cx);
        state.perform_edit_menu_command(InputEditMenuCommand::Submenu, window, cx);
        state.edit_menu()
    });
    assert_eq!(presentation.map(|menu| menu.in_submenu), Some(true));
}

#[gpui::test]
fn scroll_enabled_decides_how_many_rows_layout_asks_for(cx: &mut TestAppContext) {
    let field = open(cx);
    let rows = edit(&field, cx, |state, _, cx| {
        state.reset("1\n2\n3\n4\n5", cx);
        state.set_max_visible_lines(3, cx);
        let capped = state.layout_rows();
        state.set_scroll_enabled(false, cx);
        let grown = state.layout_rows();
        state.set_scroll_enabled(true, cx);
        (capped, grown, state.layout_rows(), state.fills_parent_height())
    });
    assert_eq!(rows, (3, 5, 1, true));
}

#[gpui::test]
fn colour_and_size_overrides_fall_back_to_the_theme(cx: &mut TestAppContext) {
    let field = open(cx);
    let red = hsla(0., 1., 0.5, 1.);
    let values = edit(&field, cx, |state, _, cx| {
        state.set_text_color(Some(red), cx);
        state.set_caret_color(Some(red), cx);
        state.set_font_size(Some(px(20.)), cx);
        let set = (state.text_color(), state.caret_color(), state.font_size());
        state.set_text_color(None, cx);
        (set, state.text_color(), state.placeholder_color())
    });
    assert_eq!(values, ((Some(red), Some(red), Some(px(20.))), None, None));
}
