use gpui::TruncateFrom;

use crate::main_text::{MainText, scaling_factor};
use crate::secondary_text::SecondaryText;
use crate::swiftui::minimum_scale_factor::{
    clamped_minimum_scale_factor, colored_runs, is_word_letter, scaled_line_height, single_line_scale, truncate_from,
};
use crate::swiftui::text::TruncationMode;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn a_line_that_fits_keeps_its_size() {
    assert_eq!(single_line_scale(80.0, 100.0, 0.7), 1.0);
    assert_eq!(single_line_scale(100.0, 100.0, 0.7), 1.0);
}

#[test]
fn a_wide_line_shrinks_to_the_offered_width() {
    assert!(close(single_line_scale(100.0, 80.0, 0.7), 0.8));
}

#[test]
fn shrinking_stops_at_the_minimum_factor() {
    assert!(close(single_line_scale(100.0, 40.0, 0.7), 0.7));
    assert!(close(single_line_scale(100.0, 0.0, 0.5), 0.5));
    assert!(close(single_line_scale(100.0, -10.0, 0.5), 0.5));
}

#[test]
fn an_empty_line_never_scales() {
    assert_eq!(single_line_scale(0.0, 0.0, 0.5), 1.0);
}

#[test]
fn the_factor_stays_within_swiftui_range() {
    assert_eq!(clamped_minimum_scale_factor(1.5), 1.0);
    assert!(close(clamped_minimum_scale_factor(0.0), 0.01));
    assert!(close(clamped_minimum_scale_factor(-1.0), 0.01));
    assert_eq!(clamped_minimum_scale_factor(f32::NAN), 1.0);
    assert!(close(clamped_minimum_scale_factor(0.55), 0.55));
}

#[test]
fn line_spacing_does_not_scale() {
    assert!(close(scaled_line_height(20.0, 0.0, 0.5), 10.0));
    assert!(close(scaled_line_height(26.0, 6.0, 0.5), 16.0));
    assert!(close(scaled_line_height(26.0, 6.0, 1.0), 26.0));
}

#[test]
fn truncation_modes_cut_where_swiftui_cuts() {
    assert_eq!(truncate_from(TruncationMode::Head), TruncateFrom::Start);
    assert_eq!(truncate_from(TruncationMode::Tail), TruncateFrom::End);
    assert_eq!(truncate_from(TruncationMode::Middle), TruncateFrom::Middle);
}

#[test]
fn a_factor_of_one_renders_the_plain_text() {
    assert_eq!(scaling_factor(None), None);
    assert_eq!(scaling_factor(Some(1.0)), None);
    assert_eq!(scaling_factor(Some(1.2)), None);
    assert_eq!(scaling_factor(Some(f32::NAN)), None);
    assert_eq!(scaling_factor(Some(0.7)), Some(0.7));
}

#[test]
fn main_text_carries_the_outer_modifier() {
    assert_eq!(MainText::new("Skip").effective_minimum_scale_factor(), None);
    assert_eq!(
        MainText::new("Skip")
            .number_of_lines(1)
            .minimum_scale_factor(0.7)
            .effective_minimum_scale_factor(),
        Some(0.7)
    );
}

#[test]
fn secondary_text_carries_the_outer_modifiers() {
    let text = SecondaryText::new("caption").line_limit(1).minimum_scale_factor(0.85);
    assert_eq!(text.effective_line_limit(), Some(1));
    assert_eq!(text.effective_minimum_scale_factor(), Some(0.85));
    assert_eq!(SecondaryText::new("caption").line_limit(0).effective_line_limit(), None);
}

#[test]
fn a_wrap_between_latin_or_cyrillic_letters_cuts_a_word() {
    assert!(is_word_letter('a'));
    assert!(is_word_letter('7'));
    assert!(is_word_letter('\u{0434}'));
    assert!(!is_word_letter(' '));
    assert!(!is_word_letter('-'));
    assert!(!is_word_letter('\u{4E2D}'));
}

#[test]
fn colored_runs_split_the_inherited_run_and_cover_the_text() {
    let base = gpui::TextRun {
        len: 6,
        font: gpui::font("Helvetica"),
        color: gpui::white(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let red = gpui::red();
    let runs = colored_runs(base, 6, &[(1..2, red), (4..9, red)]);
    let lens: Vec<usize> = runs.iter().map(|run| run.len).collect();
    assert_eq!(lens, vec![1, 1, 2, 2]);
    assert_eq!(runs[1].color, red);
    assert_eq!(runs[2].color, gpui::white());
    assert_eq!(runs[3].color, red);
}
