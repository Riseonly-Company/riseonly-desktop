use crate::modules::media_picker::editor::editor_color_picker_bridge::EditorColorHex;

#[test]
fn six_digits_survive_the_round_trip() {
    for digits in ["000000", "FFFFFF", "7F3AC2", "808080", "01FE10"] {
        let parsed = EditorColorHex::color_of(digits);
        assert_eq!(parsed.map(EditorColorHex::text_of).as_deref(), Some(digits));
    }
}

#[test]
fn lower_case_digits_are_a_colour_and_show_upper_case() {
    assert_eq!(EditorColorHex::color_of("7f3ac2").map(EditorColorHex::text_of).as_deref(), Some("7F3AC2"));
}

#[test]
fn anything_but_six_digits_is_no_colour() {
    for typed in ["", "ABC", "ABCDE", "ABCDEFA", "GGGGGG", "#ABCDE", "ABCD\u{0415}F"] {
        assert!(EditorColorHex::color_of(typed).is_none(), "{typed}");
    }
}

#[test]
fn the_field_keeps_six_hex_digits_only() {
    assert_eq!(EditorColorHex::kept_digits("#AABBCC"), "AABBCC");
    assert_eq!(EditorColorHex::kept_digits("aabbccdd"), "aabbcc");
    assert_eq!(EditorColorHex::kept_digits("1\u{0430} 2z3"), "123");
    assert_eq!(EditorColorHex::kept_digits(""), "");
}
