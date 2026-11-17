use gpui::{
    Bounds, Context, EntityInputHandler, Pixels, TestAppContext, Window, WindowHandle, point, px,
    size,
};
use rise_theme::AppTheme;

use crate::input_ui::input_ui::attachment_slot;
use crate::input_ui::shaped_input::{attachment_shifts, line_attachments};
use crate::input_ui::*;
use crate::swiftui::rich_text::AttributeContainer;

#[derive(Clone, PartialEq, Debug)]
struct Emoji(&'static str);

const OBJECT: &str = "\u{FFFC}";

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

fn emoji_bounds() -> Bounds<Pixels> {
    Bounds::new(point(px(0.0), px(-4.0)), size(px(20.0), px(20.0)))
}

fn emoji(attachment: &TextAttachment) -> RunAttributes {
    RunAttributes::tagged(AttributeContainer::default(), Emoji("smile"))
        .with_attachment(attachment.clone())
}

fn install(state: &mut InputUiState, attachment: &TextAttachment, cx: &mut Context<InputUiState>) {
    state.set_attributed_text(
        [
            ("a".to_owned(), RunAttributes::default()),
            (OBJECT.to_owned(), emoji(attachment)),
            ("b".to_owned(), RunAttributes::default()),
        ],
        RunAttributes::default(),
        cx,
    );
}

fn attachment_ranges(state: &InputUiState) -> Vec<(std::ops::Range<usize>, TextAttachmentId)> {
    state
        .attributed_runs()
        .map(|runs| {
            runs.attachments(state.text())
                .map(|(range, attachment)| (range, attachment.id()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn every_attachment_is_its_own_object() {
    let first = TextAttachment::new(None, emoji_bounds());
    let second = TextAttachment::new(None, emoji_bounds());
    assert_ne!(first.id(), second.id());
    assert_ne!(first, second);
    assert_eq!(first.clone(), first);
    assert_ne!(
        RunAttributes::default().with_attachment(first.clone()),
        RunAttributes::default().with_attachment(second),
        "two attachments side by side stay two runs"
    );
}

#[test]
fn the_attachment_advances_by_its_bounds_width() {
    let attachment = TextAttachment::new(None, emoji_bounds());
    let stops = [(0, px(0.0)), (1, px(8.0)), (4, px(14.0))];
    let shifts = attachment_shifts(&stops, px(22.0), &[(1, attachment.clone())]);
    assert_eq!(shifts, vec![(1, px(14.0))]);

    let shifts = attachment_shifts(&stops[..2], px(14.0), &[(1, attachment)]);
    assert_eq!(shifts, vec![(1, px(14.0))]);
}

#[test]
fn the_image_sits_on_the_baseline_by_its_bounds() {
    let slot = attachment_slot(px(30.0), px(100.0), emoji_bounds());
    assert_eq!(slot.origin, point(px(30.0), px(84.0)));
    assert_eq!(slot.size, size(px(20.0), px(20.0)));
    assert_eq!(slot.bottom(), px(104.0));
}

#[test]
fn only_object_characters_of_an_attachment_run_are_drawn_as_images() {
    let attachment = TextAttachment::new(None, emoji_bounds());
    let text = format!("a{OBJECT}{OBJECT}b");
    let display = DisplayText::new(&text, &gpui::SharedString::default(), false, false);
    let runs = AttributedRuns::from_pieces([
        (1, RunAttributes::default()),
        (OBJECT.len() * 2, RunAttributes::default().with_attachment(attachment.clone())),
        (1, RunAttributes::default()),
    ]);
    let placed: Vec<usize> = line_attachments(&display, Some(&runs), 0, &display.text)
        .into_iter()
        .map(|(offset, _)| offset)
        .collect();
    assert_eq!(placed, vec![1, 1 + OBJECT.len()]);

    let secure = DisplayText::new(&text, &gpui::SharedString::default(), true, false);
    assert!(line_attachments(&secure, Some(&runs), 0, &secure.text).is_empty());
}

#[gpui::test]
fn text_typed_after_an_attachment_keeps_the_tag_but_not_the_image(cx: &mut TestAppContext) {
    let field = open(cx);
    let attachment = TextAttachment::new(None, emoji_bounds());
    edit(&field, cx, |state, window, cx| {
        install(state, &attachment, cx);
        let after = 1 + OBJECT.len();
        state.set_selection(Selection::cursor(after), cx);
        assert!(state.typing_attributes().attachment.is_none());
        assert_eq!(state.typing_attributes().tag::<Emoji>(), Some(&Emoji("smile")));

        state.replace_text_in_range(None, "X", window, cx);
        assert_eq!(state.text(), format!("a{OBJECT}Xb"));
        let pieces: Vec<(String, bool, Option<Emoji>)> = state
            .attributed_pieces()
            .into_iter()
            .map(|(piece, attributes)| {
                (
                    piece.to_owned(),
                    attributes.attachment.is_some(),
                    attributes.tag::<Emoji>().cloned(),
                )
            })
            .collect();
        assert_eq!(
            pieces,
            vec![
                ("a".to_owned(), false, None),
                (OBJECT.to_owned(), true, Some(Emoji("smile"))),
                ("X".to_owned(), false, Some(Emoji("smile"))),
                ("b".to_owned(), false, None),
            ]
        );
    });
}

#[gpui::test]
fn explicit_typing_attributes_never_type_an_attachment(cx: &mut TestAppContext) {
    let field = open(cx);
    let attachment = TextAttachment::new(None, emoji_bounds());
    edit(&field, cx, |state, window, cx| {
        install(state, &attachment, cx);
        state.set_selection(Selection::cursor(0), cx);
        state.set_typing_attributes(emoji(&attachment));
        assert!(state.typing_attributes().attachment.is_none());
        state.replace_text_in_range(None, "Y", window, cx);
        assert_eq!(attachment_ranges(state), vec![(2..2 + OBJECT.len(), attachment.id())]);
    });
}

#[gpui::test]
fn edits_before_an_attachment_move_it_and_deleting_it_drops_it(cx: &mut TestAppContext) {
    let field = open(cx);
    let attachment = TextAttachment::new(None, emoji_bounds());
    edit(&field, cx, |state, window, cx| {
        install(state, &attachment, cx);
        assert_eq!(attachment_ranges(state), vec![(1..1 + OBJECT.len(), attachment.id())]);
        assert_eq!(state.tallest_attachment(), Some(px(20.0)));

        state.set_selection(Selection::cursor(0), cx);
        state.replace_text_in_range(None, "zz", window, cx);
        assert_eq!(attachment_ranges(state), vec![(3..3 + OBJECT.len(), attachment.id())]);

        state.replace_text_in_range(Some(3..4), "", window, cx);
        assert_eq!(state.text(), "zzab");
        assert!(attachment_ranges(state).is_empty());
        assert_eq!(state.tallest_attachment(), None);
    });
}

#[gpui::test]
fn copying_an_attachment_yields_the_object_character(cx: &mut TestAppContext) {
    let field = open(cx);
    let attachment = TextAttachment::new(None, emoji_bounds());
    edit(&field, cx, |state, window, cx| {
        install(state, &attachment, cx);
        let mut adjusted = None;
        assert_eq!(
            state.text_for_range(1..2, &mut adjusted, window, cx),
            Some(OBJECT.to_owned())
        );
    });
}
