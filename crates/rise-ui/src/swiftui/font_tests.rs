use gpui::{AbsoluteLength, DefiniteLength, Div, FontStyle, FontWeight, Styled, div, px};
use rise_platform::text_capabilities::monospaced_family;
use rise_theme::FONT_FAMILY;

use crate::swiftui::font::{
    Font, FontDesign, NATURAL_LINE_HEIGHT, SwiftUiFont, TABULAR_FIGURES, TextStyle,
};

const SHIPPED_REGULAR: &[u8] = include_bytes!("../../../../assets/fonts/Inter-Regular.ttf");

fn table_offset(face: &[u8], tag: &[u8; 4]) -> Option<usize> {
    let count = u16::from_be_bytes([*face.get(4)?, *face.get(5)?]) as usize;
    (0..count).find_map(|index| {
        let record = face.get(12 + index * 16..28 + index * 16)?;
        (&record[0..4] == tag).then(|| {
            u32::from_be_bytes([record[8], record[9], record[10], record[11]]) as usize
        })
    })
}

fn be_u16(face: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*face.get(at)?, *face.get(at + 1)?]))
}

fn styled(font: Font) -> Div {
    div().swiftui_font(font)
}

#[test]
fn text_styles_match_the_large_dynamic_type_table() {
    let table = [
        (Font::large_title(), 34.0, FontWeight::NORMAL),
        (Font::title(), 28.0, FontWeight::NORMAL),
        (Font::title2(), 22.0, FontWeight::NORMAL),
        (Font::title3(), 20.0, FontWeight::NORMAL),
        (Font::headline(), 17.0, FontWeight::SEMIBOLD),
        (Font::body(), 17.0, FontWeight::NORMAL),
        (Font::callout(), 16.0, FontWeight::NORMAL),
        (Font::subheadline(), 15.0, FontWeight::NORMAL),
        (Font::footnote(), 13.0, FontWeight::NORMAL),
        (Font::caption(), 12.0, FontWeight::NORMAL),
        (Font::caption2(), 11.0, FontWeight::NORMAL),
    ];
    for (font, size, weight) in table {
        assert_eq!(font.size, size, "a text style drew at the wrong size: {font:?}");
        assert_eq!(font.weight, weight, "a text style drew at the wrong weight: {font:?}");
        assert_eq!(font.design, FontDesign::Default);
    }
    assert_eq!(Font::text_style(TextStyle::Caption2), Font::caption2());
}

#[test]
fn modifiers_keep_the_size_and_change_only_their_part() {
    let caption = Font::caption().weight(FontWeight::SEMIBOLD);
    assert_eq!(caption.size, 12.0);
    assert_eq!(caption.weight, FontWeight::SEMIBOLD);

    let mono = Font::caption2().monospaced();
    assert_eq!(mono.size, 11.0);
    assert_eq!(mono.design, FontDesign::Monospaced);

    assert_eq!(Font::body().bold().weight, FontWeight::BOLD);
    assert_eq!(Font::body().italic().style(), FontStyle::Italic);
    assert_eq!(Font::body().style(), FontStyle::Normal);
    assert_eq!(
        Font::system(9.0, FontWeight::EXTRA_BOLD, FontDesign::Default),
        Font::system_weight(9.0, FontWeight::EXTRA_BOLD)
    );
    assert_eq!(Font::system_size(14.0).weight, FontWeight::NORMAL);
}

#[test]
fn only_monospaced_leaves_the_shipped_family() {
    for design in [FontDesign::Default, FontDesign::Rounded, FontDesign::Serif] {
        assert_eq!(
            Font::system(11.0, FontWeight::NORMAL, design).family().as_ref(),
            FONT_FAMILY,
            "a face that does not ship would measure differently on each OS"
        );
    }
    assert_eq!(
        Font::system(11.0, FontWeight::NORMAL, FontDesign::Monospaced).family().as_ref(),
        monospaced_family()
    );
}

#[test]
fn monospaced_digit_turns_on_tabular_figures_only_when_asked() {
    let tabular = Font::caption().monospaced_digit().features();
    assert_eq!(tabular.0.as_slice(), &[(TABULAR_FIGURES.to_string(), 1)]);
    assert!(Font::caption().features().0.is_empty());
}

#[test]
fn swiftui_font_cascades_every_part_to_the_text_inside() {
    let font = Font::caption().weight(FontWeight::SEMIBOLD).italic();
    let mut element = styled(font);
    let style = element.text_style();
    assert_eq!(style.font_family.as_ref().map(|family| family.as_ref()), Some(FONT_FAMILY));
    assert_eq!(style.font_size, Some(AbsoluteLength::Pixels(px(12.0))));
    assert_eq!(style.font_weight, Some(FontWeight::SEMIBOLD));
    assert_eq!(style.font_style, Some(FontStyle::Italic));
    assert_eq!(style.line_height, Some(DefiniteLength::Fraction(NATURAL_LINE_HEIGHT)));
}

#[test]
fn tracking_leaves_the_font_as_set() {
    let mut tracked = styled(Font::caption2()).tracking(0.5);
    let style = tracked.text_style();
    assert_eq!(style.font_size, Some(AbsoluteLength::Pixels(px(11.0))));
}

#[test]
fn natural_line_height_is_the_shipped_face_metrics() {
    let head = table_offset(SHIPPED_REGULAR, b"head").expect("the shipped face has a head table");
    let hhea = table_offset(SHIPPED_REGULAR, b"hhea").expect("the shipped face has an hhea table");
    let units_per_em = be_u16(SHIPPED_REGULAR, head + 18).expect("unitsPerEm") as f32;
    let ascender = be_u16(SHIPPED_REGULAR, hhea + 4).expect("ascender") as i16 as f32;
    let descender = be_u16(SHIPPED_REGULAR, hhea + 6).expect("descender") as i16 as f32;
    assert_eq!(
        NATURAL_LINE_HEIGHT,
        (ascender - descender) / units_per_em,
        "a new Inter build changed the line height SwiftUI lays text at"
    );
}
