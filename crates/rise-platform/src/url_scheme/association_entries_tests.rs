use std::path::Path;

use super::association_entries::*;

const EXE: &str = r"C:\Program Files\Riseonly\riseonly.exe";

fn exe() -> &'static Path {
    Path::new(EXE)
}

#[test]
fn the_open_command_quotes_the_program_and_the_argument() {
    assert_eq!(open_command(exe()), format!("\"{EXE}\" \"%1\""));
}

#[test]
fn a_command_is_ours_by_its_program_whatever_the_case_or_arguments() {
    assert!(command_starts_executable(&open_command(exe()), exe()));
    assert!(command_starts_executable(&format!("\"{}\" --x \"%1\"", EXE.to_uppercase()), exe()));
    assert!(!command_starts_executable(r#""C:\Other\riseonly.exe" "%1""#, exe()));
    assert!(!command_starts_executable("", exe()));
}

#[test]
fn an_unquoted_command_is_read_up_to_its_first_space() {
    let plain = Path::new(r"C:\Riseonly\riseonly.exe");
    assert!(command_starts_executable(r"C:\Riseonly\riseonly.exe %1", plain));
    assert!(!command_starts_executable(r"C:\Program Files\Riseonly\riseonly.exe %1", exe()));
}

#[test]
fn a_url_scheme_gets_the_protocol_marker_and_the_open_command() {
    let values = url_scheme_values("riseonly", exe());
    let key = r"Software\Classes\riseonly";
    assert!(values.iter().any(|value| {
        value.subkey == key && value.name.is_none() && value.value == "URL:riseonly Protocol"
    }));
    assert!(values.iter().any(|value| {
        value.subkey == key && value.name.as_deref() == Some("URL Protocol") && value.value.is_empty()
    }));
    assert!(values.iter().any(|value| {
        value.subkey == format!(r"{key}\shell\open\command") && value.value == open_command(exe())
    }));
}

#[test]
fn open_with_lists_every_extension_and_never_touches_a_default_association() {
    let values = open_with_values(exe());
    for extension in OPEN_WITH_EXTENSIONS {
        let key = format!(r"Software\Classes\.{extension}\OpenWithProgids");
        assert!(
            values.iter().any(|value| value.subkey == key && value.name.as_deref() == Some(OPEN_WITH_PROG_ID)),
            "{extension} is not offered"
        );
        assert!(
            !values.iter().any(|value| value.subkey == format!(r"Software\Classes\.{extension}")),
            "{extension}'s default association must stay the person's"
        );
    }
}

#[test]
fn open_with_writes_only_when_the_executable_or_the_list_changed() {
    let list = OPEN_WITH_EXTENSIONS.join(" ");
    assert!(open_with_is_settled(Some(&open_command(exe())), Some(&list), exe()));
    assert!(!open_with_is_settled(Some(r#""C:\old.exe" "%1""#), Some(&list), exe()));
    assert!(!open_with_is_settled(None, Some(&list), exe()));
    assert!(!open_with_is_settled(Some(&open_command(exe())), Some("jpg png"), exe()));
    assert!(!open_with_is_settled(Some(&open_command(exe())), None, exe()));
    assert_eq!(open_with_command_key(), r"Software\Classes\Riseonly.File\shell\open\command");
    assert_eq!(open_with_extensions_key(), r"Software\Classes\Riseonly.File\OpenWithExtensions");
}

#[test]
fn open_with_writes_the_settled_values_last_so_a_partial_write_is_repeated() {
    let values = open_with_values(exe());
    let [.., marker, command] = values.as_slice() else {
        panic!("the registration has values");
    };
    assert_eq!(command.subkey, open_with_command_key());
    assert_eq!(command.value, open_command(exe()));
    assert_eq!(marker.subkey, open_with_extensions_key());
    assert_eq!(marker.value, OPEN_WITH_EXTENSIONS.join(" "));

    for written in 0..values.len() {
        let read = |key: String| {
            values[..written]
                .iter()
                .find(|value| value.subkey == key && value.name.is_none())
                .map(|value| value.value.as_str())
        };
        assert!(!open_with_is_settled(read(open_with_command_key()), read(open_with_extensions_key()), exe()));
    }
}
