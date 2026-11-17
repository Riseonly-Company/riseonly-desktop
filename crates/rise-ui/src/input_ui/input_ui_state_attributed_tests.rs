use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Context, EntityInputHandler, TestAppContext, Window, WindowHandle, hsla};
use rise_core::foundation::ns_range::NSRange;
use rise_theme::AppTheme;

use crate::input_ui::*;
use crate::swiftui::rich_text::AttributeContainer;
use crate::input_ui::input_actions::{Backspace, Confirm, Undo};

#[derive(Clone, PartialEq, Debug)]
struct Mark(&'static str);

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

fn link() -> RunAttributes {
    RunAttributes::tagged(
        AttributeContainer {
            foreground_color: Some(hsla(0.6, 1.0, 0.5, 1.0)),
            ..AttributeContainer::default()
        },
        Mark("link"),
    )
}

fn install(state: &mut InputUiState, cx: &mut Context<InputUiState>) {
    state.set_attributed_text(
        [
            ("ab".to_owned(), RunAttributes::default()),
            ("cd".to_owned(), link()),
        ],
        RunAttributes::default(),
        cx,
    );
}

fn tags(state: &InputUiState) -> Vec<(String, Option<Mark>)> {
    state
        .typed_pieces::<Mark>()
        .into_iter()
        .map(|(piece, _, tag)| (piece.to_owned(), tag.cloned()))
        .collect()
}

#[gpui::test]
fn typing_inside_a_run_takes_its_attributes(cx: &mut TestAppContext) {
    let field = open(cx);
    edit(&field, cx, |state, window, cx| {
        install(state, cx);
        state.set_selection(Selection::cursor(3), cx);
        state.replace_text_in_range(None, "X", window, cx);
        assert_eq!(state.text(), "abcXd");
        assert_eq!(
            tags(state),
            vec![
                ("ab".to_owned(), None),
                ("cXd".to_owned(), Some(Mark("link"))),
            ]
        );
    });
}

#[gpui::test]
fn explicit_typing_attributes_last_until_the_caret_moves(cx: &mut TestAppContext) {
    let field = open(cx);
    edit(&field, cx, |state, window, cx| {
        install(state, cx);
        state.set_selection(Selection::cursor(1), cx);
        state.set_typing_attributes(link());
        state.replace_text_in_range(None, "Y", window, cx);
        assert_eq!(
            tags(state),
            vec![
                ("a".to_owned(), None),
                ("Y".to_owned(), Some(Mark("link"))),
                ("b".to_owned(), None),
                ("cd".to_owned(), Some(Mark("link"))),
            ]
        );
        state.set_selection(Selection::cursor(0), cx);
        assert_eq!(state.typing_attributes(), RunAttributes::default());
    });
}

#[gpui::test]
fn the_delegate_refuses_enter_and_hears_backspace_at_the_start(cx: &mut TestAppContext) {
    let field = open(cx);
    let asked: Rc<RefCell<Vec<(NSRange, String)>>> = Rc::default();
    let changes = Rc::new(RefCell::new(0_usize));
    edit(&field, cx, |state, window, cx| {
        let record = asked.clone();
        let counted = changes.clone();
        state.set_delegate(
            InputUiTextViewDelegate::default()
                .should_change_text_in(move |range, text, _cx| {
                    record.borrow_mut().push((range, text.to_owned()));
                    text != "\n" && !(text.is_empty() && range == NSRange::new(0, 0))
                })
                .did_change(move |_state, _cx| *counted.borrow_mut() += 1),
        );
        install(state, cx);
        assert_eq!(*changes.borrow(), 0, "installing text is not a user edit");

        state.set_selection(Selection::cursor(2), cx);
        state.confirm(&Confirm, window, cx);
        assert_eq!(state.text(), "abcd", "a refused Enter leaves the text alone");

        state.set_selection(Selection::cursor(0), cx);
        state.backspace(&Backspace, window, cx);
        assert_eq!(state.text(), "abcd");

        state.set_selection(Selection::cursor(4), cx);
        state.backspace(&Backspace, window, cx);
        assert_eq!(state.text(), "abc");
        assert_eq!(*changes.borrow(), 1);

        state.undo(&Undo, window, cx);
        assert_eq!(state.text(), "abcd");
        assert_eq!(*changes.borrow(), 2);
    });
    assert_eq!(
        *asked.borrow(),
        vec![
            (NSRange::new(2, 0), "\n".to_owned()),
            (NSRange::new(0, 0), String::new()),
            (NSRange::new(3, 1), String::new()),
        ]
    );
}

#[gpui::test]
fn the_buffer_is_not_reinstalled_mid_composition(cx: &mut TestAppContext) {
    let field = open(cx);
    edit(&field, cx, |state, window, cx| {
        install(state, cx);
        state.set_selection(Selection::cursor(4), cx);
        state.replace_and_mark_text_in_range(None, "ni", None, window, cx);
        assert!(state.is_composing());
        state.set_attributed_text(
            [("other".to_owned(), RunAttributes::default())],
            RunAttributes::default(),
            cx,
        );
        assert_eq!(state.text(), "abcdni");
        state.unmark_text(window, cx);
        assert!(!state.is_composing());
    });
}

#[gpui::test]
fn selected_range_speaks_utf16_and_survives_a_reinstall(cx: &mut TestAppContext) {
    let field = open(cx);
    edit(&field, cx, |state, _window, cx| {
        state.set_attributed_text(
            [("日本語".to_owned(), RunAttributes::default())],
            RunAttributes::default(),
            cx,
        );
        state.set_selected_range(NSRange::new(1, 1), cx);
        assert_eq!(state.selection().range(), 3..6);
        assert_eq!(state.selected_range(), NSRange::new(1, 1));

        state.set_attributed_text(
            [("日".to_owned(), RunAttributes::default())],
            RunAttributes::default(),
            cx,
        );
        assert_eq!(state.selected_range(), NSRange::new(1, 0));
    });
}
