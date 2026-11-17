use std::path::Path;

const CLASSES_KEY: &str = r"Software\Classes";

pub const OPEN_WITH_PROG_ID: &str = "Riseonly.File";

const APP_NAME: &str = "Riseonly";

pub const OPEN_WITH_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "tif", "tiff",
    "mp4", "mov", "m4v", "webm", "mkv", "avi",
    "mp3", "m4a", "aac", "ogg", "oga", "opus", "wav", "flac",
    "pdf", "txt", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "zip", "rar", "7z",
];

/// one string value under HKEY_CURRENT_USER, name None is the key default value
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AssociationValue {
    pub subkey: String,
    pub name: Option<String>,
    pub value: String,
}

impl AssociationValue {
    fn default_of(subkey: String, value: impl Into<String>) -> Self {
        Self {
            subkey,
            name: None,
            value: value.into(),
        }
    }

    fn named(subkey: String, name: &str) -> Self {
        Self {
            subkey,
            name: Some(name.to_owned()),
            value: String::new(),
        }
    }
}

/// builds the open command with both the exe and %1 quoted, a path with spaces would split in argv
pub fn open_command(executable: &Path) -> String {
    format!("\"{}\" \"%1\"", executable.display())
}

/// true when a registered command starts this executable
/// windows paths ignore case and the rest of the command can be spelled any way
pub fn command_starts_executable(command: &str, executable: &Path) -> bool {
    let command = command.trim_start_matches(' ');
    let program = match command.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next(),
        None => command.split(' ').next(),
    };
    program.is_some_and(|program| {
        program.to_lowercase() == executable.display().to_string().to_lowercase()
    })
}

fn class_values(class: &str, description: String, executable: &Path) -> Vec<AssociationValue> {
    let key = format!(r"{CLASSES_KEY}\{class}");
    vec![
        AssociationValue::default_of(key.clone(), description),
        AssociationValue::default_of(format!(r"{key}\DefaultIcon"), format!("{},0", executable.display())),
        AssociationValue::default_of(format!(r"{key}\shell\open\command"), open_command(executable)),
    ]
}

pub fn url_scheme_values(scheme: &str, executable: &Path) -> Vec<AssociationValue> {
    let mut values = class_values(scheme, format!("URL:{scheme} Protocol"), executable);
    values.push(AssociationValue::named(format!(r"{CLASSES_KEY}\{scheme}"), "URL Protocol"));
    values
}

/// one OpenWithProgids entry per extension then the class, extension default value stays untouched
/// so nothing opens with riseonly by itself
/// partial write leaves old values and next launch redoes all
pub fn open_with_values(executable: &Path) -> Vec<AssociationValue> {
    let mut values: Vec<AssociationValue> = OPEN_WITH_EXTENSIONS
        .iter()
        .map(|extension| {
            AssociationValue::named(format!(r"{CLASSES_KEY}\.{extension}\OpenWithProgids"), OPEN_WITH_PROG_ID)
        })
        .collect();
    values.extend(class_values(OPEN_WITH_PROG_ID, APP_NAME.to_owned(), executable));
    let command = values.pop();
    values.push(AssociationValue::default_of(open_with_extensions_key(), open_with_extensions_marker()));
    values.extend(command);
    values
}

/// key whose default value tells a launch the Open With class is already written
pub fn open_with_command_key() -> String {
    format!(r"{CLASSES_KEY}\{OPEN_WITH_PROG_ID}\shell\open\command")
}

/// key whose default value names the extensions the last complete write covered
pub fn open_with_extensions_key() -> String {
    format!(r"{CLASSES_KEY}\{OPEN_WITH_PROG_ID}\OpenWithExtensions")
}

/// the extension list itself so a new list is noticed with no version number
fn open_with_extensions_marker() -> String {
    OPEN_WITH_EXTENSIONS.join(" ")
}

/// written once per executable path and extension list
/// unchanged registration needs no write and no shell refresh
pub fn open_with_is_settled(
    current_command: Option<&str>,
    current_extensions: Option<&str>,
    executable: &Path,
) -> bool {
    current_command == Some(open_command(executable).as_str())
        && current_extensions == Some(open_with_extensions_marker().as_str())
}
