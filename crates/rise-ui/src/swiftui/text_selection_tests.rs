use gpui::{Bounds, point, px, size};

use crate::swiftui::text_selection::{
    SELECTABLE_TEXT_MENU_COPY, SELECTABLE_TEXT_MENU_SELECT_ALL, SelectionGranularity, TextSelection,
    selectable_text_menu_items, selection_rects,
};

const WORD: SelectionGranularity = SelectionGranularity::Word;
const PARAGRAPH: SelectionGranularity = SelectionGranularity::Paragraph;

#[test]
fn click_count_picks_the_granularity() {
    assert_eq!(SelectionGranularity::for_click_count(1), SelectionGranularity::Character);
    assert_eq!(SelectionGranularity::for_click_count(2), SelectionGranularity::Word);
    assert_eq!(SelectionGranularity::for_click_count(3), SelectionGranularity::Paragraph);
    assert_eq!(SelectionGranularity::for_click_count(5), SelectionGranularity::Paragraph);
}

#[test]
fn press_and_drag_selects_between_the_two_points_either_way() {
    let text = "hello world";
    let mut selection = TextSelection::default();
    selection.press(text, 6, 1, false);
    assert!(selection.is_empty());
    assert!(selection.drag_to(text, 2));
    assert_eq!(selection.range(), 2..6);
    assert_eq!(selection.selected_text(text), Some("llo "));
    assert!(!selection.drag_to(text, 2), "the same point is no change and no redraw");
    selection.release();
    assert!(!selection.drag_to(text, 9), "after release the pointer no longer selects");
    assert_eq!(selection.range(), 2..6);
}

#[test]
fn shift_click_extends_the_existing_selection() {
    let text = "one two three";
    let mut selection = TextSelection::default();
    selection.press(text, 4, 1, false);
    selection.release();
    selection.press(text, 13, 1, true);
    assert_eq!(selection.selected_text(text), Some("two three"));
}

#[test]
fn double_click_selects_the_word_and_drags_by_words() {
    let text = "one two three";
    let mut selection = TextSelection::default();
    selection.press(text, 5, 2, false);
    assert_eq!(selection.selected_text(text), Some("two"));
    selection.drag_to(text, 9);
    assert_eq!(selection.selected_text(text), Some("two three"));
}

#[test]
fn triple_click_selects_the_line_without_its_break() {
    let text = "first line\nsecond line\nthird";
    let mut selection = TextSelection::default();
    selection.press(text, 14, 3, false);
    assert_eq!(selection.selected_text(text), Some("second line"));
    assert_eq!(PARAGRAPH.unit_at(text, 0), 0..10);
    assert_eq!(PARAGRAPH.unit_at(text, text.len()), 23..28);
}

#[test]
fn words_follow_character_classes() {
    let text = "hi, user_42!";
    assert_eq!(WORD.unit_at(text, 0), 0..2);
    assert_eq!(WORD.unit_at(text, 2), 2..3, "punctuation alone");
    assert_eq!(WORD.unit_at(text, 6), 4..11);
    assert_eq!(WORD.unit_at(text, text.len()), 11..12, "at the end: the character before");
    assert_eq!(WORD.unit_at("a  b", 2), 1..3, "a run of spaces");
    assert_eq!(WORD.unit_at("", 0), 0..0);
}

#[test]
fn words_handle_non_ascii() {
    let text = "privet\n\u{43c}\u{438}\u{440}";
    assert_eq!(WORD.unit_at(text, 2), 0..6);
    assert_eq!(WORD.unit_at(text, 8), 7..13);
}

#[test]
fn indices_inside_a_character_snap_back_to_its_start() {
    let text = "a\u{439}";
    assert_eq!(SelectionGranularity::Character.unit_at(text, 2), 1..1);
    assert_eq!(SelectionGranularity::Character.unit_at(text, 99), text.len()..text.len());
}

#[test]
fn right_click_keeps_a_selection_under_the_pointer() {
    let text = "one two three";
    let mut selection = TextSelection::default();
    selection.press(text, 0, 1, false);
    selection.drag_to(text, 7);
    selection.release();
    assert!(!selection.secondary_press(text, 5));
    assert_eq!(selection.selected_text(text), Some("one two"));
}

#[test]
fn right_click_elsewhere_selects_the_word_there() {
    let text = "one two three";
    let mut selection = TextSelection::default();
    assert!(selection.secondary_press(text, 10));
    assert_eq!(selection.selected_text(text), Some("three"));
    assert!(!selection.is_dragging(), "a right press never starts a drag");
    assert!(!selection.secondary_press(text, 9), "inside the same word: no change");
}

#[test]
fn menu_offers_copy_only_with_a_selection() {
    let [copy, select_all] = selectable_text_menu_items(false);
    assert_eq!(copy.id, SELECTABLE_TEXT_MENU_COPY);
    assert!(!copy.enabled);
    assert_eq!(select_all.id, SELECTABLE_TEXT_MENU_SELECT_ALL);
    assert!(select_all.enabled);
    assert!(selectable_text_menu_items(true)[0].enabled);
}

#[test]
fn select_all_and_clear() {
    let text = "copy me";
    let mut selection = TextSelection::default();
    selection.select_all(text);
    assert_eq!(selection.selected_text(text), Some("copy me"));
    assert!(selection.clear());
    assert!(!selection.clear(), "clearing nothing asks for no redraw");
    assert_eq!(selection.selected_text(text), None);
}

#[test]
fn a_shorter_text_drops_a_selection_it_no_longer_holds() {
    let mut selection = TextSelection::default();
    selection.select_all("a long message");
    selection.fit("a long message");
    assert_eq!(selection.range(), 0..14);
    selection.fit("short");
    assert!(selection.is_empty());
}

#[test]
fn one_row_gives_one_rectangle() {
    let bounds = Bounds::new(point(px(10.), px(20.)), size(px(200.), px(60.)));
    let rects = selection_rects(point(px(30.), px(20.)), point(px(80.), px(20.)), px(20.), bounds);
    assert_eq!(rects, vec![Bounds::from_corners(point(px(30.), px(20.)), point(px(80.), px(40.)))]);
    assert!(selection_rects(point(px(30.), px(20.)), point(px(30.), px(20.)), px(20.), bounds).is_empty());
}

#[test]
fn several_rows_give_tail_body_and_head() {
    let bounds = Bounds::new(point(px(10.), px(20.)), size(px(200.), px(60.)));
    let rects = selection_rects(point(px(50.), px(20.)), point(px(70.), px(60.)), px(20.), bounds);
    assert_eq!(
        rects,
        vec![
            Bounds::from_corners(point(px(50.), px(20.)), point(px(210.), px(40.))),
            Bounds::from_corners(point(px(10.), px(40.)), point(px(210.), px(60.))),
            Bounds::from_corners(point(px(10.), px(60.)), point(px(70.), px(80.))),
        ]
    );
}
