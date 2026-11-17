use super::foundation_file_name::{
    file_name_length, host_file_name, is_refused_file_name_character, is_windows_device_name,
};
use super::foundation_file_url::PathFlavor;

#[test]
fn posix_refuses_only_separator_and_nul() {
    assert!(is_refused_file_name_character('/', PathFlavor::Posix));
    assert!(is_refused_file_name_character('\0', PathFlavor::Posix));
    for scalar in ['?', '*', '<', '>', '"', '|', '\\', ':', ' ', '.'] {
        assert!(!is_refused_file_name_character(scalar, PathFlavor::Posix), "{scalar:?}");
    }
}

#[test]
fn windows_refuses_reserved_punctuation_and_control() {
    for scalar in ['<', '>', ':', '"', '/', '\\', '|', '?', '*', '\0', '\u{1F}', '\t'] {
        assert!(is_refused_file_name_character(scalar, PathFlavor::Windows), "{scalar:?}");
    }
    for scalar in [' ', '.', '-', '_', '\u{444}', '\u{1F3B5}', '\u{7F}'] {
        assert!(!is_refused_file_name_character(scalar, PathFlavor::Windows), "{scalar:?}");
    }
}

#[test]
fn windows_device_names_match_case_insensitive_with_extension() {
    for name in ["CON", "con", "Nul.txt", "PRN.mp3", "aux", "COM1", "com9.m4a", "LPT0", "COM\u{B9}", "lpt\u{B3}.x", "NUL .txt", "con.tar.gz"] {
        assert!(is_windows_device_name(name), "{name:?}");
    }
    for name in ["CONSOLE", "COM10", "COMA", "LPT", "nul_", "Artist - CON.mp3", "xCON", ""] {
        assert!(!is_windows_device_name(name), "{name:?}");
    }
}

#[test]
fn posix_host_name_replaces_only_separator() {
    assert_eq!(host_file_name("AC/DC - What?.mp3", PathFlavor::Posix), "AC_DC - What?.mp3");
    assert_eq!(host_file_name("CON.mp3", PathFlavor::Posix), "CON.mp3");
    assert_eq!(host_file_name("end. ", PathFlavor::Posix), "end. ");
}

#[test]
fn windows_host_name_replaces_trims_and_guards_devices() {
    assert_eq!(host_file_name("AC/DC - What?.mp3", PathFlavor::Windows), "AC_DC - What_.mp3");
    assert_eq!(host_file_name("a<b>c:d\"e|f*g\\h", PathFlavor::Windows), "a_b_c_d_e_f_g_h");
    assert_eq!(host_file_name("Title. . ", PathFlavor::Windows), "Title");
    assert_eq!(host_file_name("CON.mp3", PathFlavor::Windows), "_CON.mp3");
    assert_eq!(host_file_name("nul", PathFlavor::Windows), "_nul");
    assert_eq!(host_file_name("\u{444}\u{1F3B5}.m4a", PathFlavor::Windows), "\u{444}\u{1F3B5}.m4a");
}

#[test]
fn windows_host_name_of_only_dots_and_spaces_is_empty() {
    assert_eq!(host_file_name(" . ..", PathFlavor::Windows), "");
    assert_eq!(host_file_name("", PathFlavor::Windows), "");
}

#[test]
fn length_counts_bytes_on_posix_and_utf16_units_on_windows() {
    assert_eq!(file_name_length("ab", PathFlavor::Posix), 2);
    assert_eq!(file_name_length("\u{444}", PathFlavor::Posix), 2);
    assert_eq!(file_name_length("\u{444}", PathFlavor::Windows), 1);
    assert_eq!(file_name_length("\u{1F3B5}", PathFlavor::Posix), 4);
    assert_eq!(file_name_length("\u{1F3B5}", PathFlavor::Windows), 2);
}
