use gpui::{FontStyle, FontWeight, HighlightStyle, SharedString, hsla};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_range::NSRange;
use rise_platform::text_capabilities::monospaced_family;

use super::rich_text::{AttributedString, FontDesign, LineStyle, RichFont, RichText};

fn bold_highlight() -> HighlightStyle {
    HighlightStyle {
        font_weight: Some(FontWeight::BOLD),
        font_style: Some(FontStyle::Normal),
        ..HighlightStyle::default()
    }
}

#[test]
fn bold_by_utf16_range_after_emoji() {
    let mut text = AttributedString::new("Hi \u{1F44B} bold");
    assert!(text.update(NSRange::new(6, 4), |a| {
        a.font = Some(RichFont::system_weight(15.0, FontWeight::BOLD));
    }));
    assert_eq!(text.highlights(None), vec![(8..12, bold_highlight())]);
    assert_eq!(&text.characters()[8..12], "bold");
}

#[test]
fn link_by_utf16_range_after_zwj_emoji() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let mut text = AttributedString::new(format!("{family} see riseonly"));
    let Some(url) = Url::from_string("https://riseonly.net/tag") else {
        panic!("test url must parse");
    };
    let link_color = hsla(0.6, 0.8, 0.5, 1.0);
    let tint = hsla(0.1, 0.9, 0.5, 1.0);
    assert!(text.update(NSRange::new(13, 8), |a| {
        a.link = Some(url.clone());
        a.foreground_color = Some(link_color);
        a.underline_style = Some(LineStyle::single());
    }));

    assert_eq!(text.link_ranges(), vec![(23..31, url.clone())]);
    assert_eq!(&text.characters()[23..31], "riseonly");

    let untinted = text.highlights(None);
    assert_eq!(untinted.len(), 1);
    assert_eq!(untinted[0].0, 23..31);
    assert_eq!(untinted[0].1.color, Some(link_color));
    assert!(untinted[0].1.underline.is_some());

    let tinted = text.highlights(Some(tint));
    assert_eq!(tinted[0].1.color, Some(tint));
}

#[test]
fn range_cutting_a_surrogate_pair_changes_nothing() {
    let mut text = AttributedString::new("Hi \u{1F44B}");
    let before = text.clone();
    assert!(!text.update(NSRange::new(4, 1), |a| a.foreground_color = Some(hsla(0.0, 1.0, 0.5, 1.0))));
    assert!(!text.update(NSRange::new(3, 9), |a| a.foreground_color = Some(hsla(0.0, 1.0, 0.5, 1.0))));
    assert_eq!(text, before);
    assert!(text.highlights(None).is_empty());
}

#[test]
fn replace_subrange_puts_bold_name_in_place_of_token() {
    let mut sentence = AttributedString::new("A gift for \u{FFFC}!");
    let Some(range) = sentence.range_of("\u{FFFC}") else {
        panic!("token must be found");
    };
    assert_eq!(range, NSRange::new(11, 1));
    let mut name = AttributedString::new("Ann \u{1F600}");
    name.update_all(|a| a.font = Some(RichFont::system_weight(15.0, FontWeight::SEMIBOLD)));
    assert!(sentence.replace_subrange(range, name));

    assert_eq!(sentence.characters(), "A gift for Ann \u{1F600}!");
    let highlights = sentence.highlights(None);
    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].0, 11..19);
    assert_eq!(highlights[0].1.font_weight, Some(FontWeight::SEMIBOLD));
}

#[test]
fn equal_neighbours_merge_and_append_keeps_runs() {
    let mut text = AttributedString::new("ab");
    let red = hsla(0.0, 1.0, 0.5, 1.0);
    assert!(text.update(NSRange::new(0, 1), |a| a.foreground_color = Some(red)));
    assert!(text.update(NSRange::new(1, 1), |a| a.foreground_color = Some(red)));
    assert_eq!(text.runs().count(), 1);

    let mut code = AttributedString::new("x");
    code.update_all(|a| a.font = Some(RichFont::system_design(14.0, FontDesign::Monospaced)));
    text += code;
    assert_eq!(text.characters(), "abx");
    assert_eq!(text.runs().count(), 2);
    assert_eq!(text.monospaced_ranges(), vec![2..3]);
}

#[test]
fn italic_bold_and_strikethrough_map_to_gpui_styles() {
    let mut text = AttributedString::new("one two");
    assert!(text.update(NSRange::new(0, 3), |a| {
        a.font = Some(RichFont::system_weight(15.0, FontWeight::BOLD).italic());
    }));
    assert!(text.update_byte_range(4..7, |a| a.strikethrough_style = Some(LineStyle::single())));
    assert!(!text.update_byte_range(5..9, |a| a.strikethrough_style = None));

    let highlights = text.highlights(None);
    assert_eq!(highlights.len(), 2);
    assert_eq!(highlights[0].0, 0..3);
    assert_eq!(highlights[0].1.font_style, Some(FontStyle::Italic));
    assert_eq!(highlights[0].1.font_weight, Some(FontWeight::BOLD));
    assert_eq!(highlights[1].0, 4..7);
    assert!(highlights[1].1.strikethrough.is_some());
}

#[test]
fn monospaced_runs_take_platform_face_by_default_and_builder_overrides() {
    let mut text = AttributedString::new("run ls now");
    assert!(text.update(NSRange::new(4, 2), |a| {
        a.font = Some(RichFont::system_design(14.0, FontDesign::Monospaced));
    }));
    let default = RichText::new("code", text.clone());
    assert_eq!(
        default.font_family_overrides(),
        vec![(4..6, SharedString::new_static(monospaced_family()))]
    );
    let custom = RichText::new("code", text).monospaced_family("Fira Code");
    assert_eq!(custom.font_family_overrides(), vec![(4..6, SharedString::from("Fira Code"))]);
}
