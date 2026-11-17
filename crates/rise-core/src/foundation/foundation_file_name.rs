use super::foundation_file_url::PathFlavor;

pub const FILE_NAME_REPLACEMENT: char = '_';

const WINDOWS_DEVICE_NAMES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
const WINDOWS_NUMBERED_DEVICE_NAMES: [&str; 2] = ["COM", "LPT"];

/// true if this char is not allowed in a file name on that flavor
pub fn is_refused_file_name_character(scalar: char, flavor: PathFlavor) -> bool {
    match flavor {
        PathFlavor::Posix => matches!(scalar, '/' | '\0'),
        PathFlavor::Windows => {
            matches!(scalar, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || u32::from(scalar) < 0x20
        }
    }
}

fn is_windows_device_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if WINDOWS_DEVICE_NAMES.contains(&upper.as_str()) {
        return true;
    }
    WINDOWS_NUMBERED_DEVICE_NAMES.iter().any(|prefix| {
        upper.strip_prefix(prefix).is_some_and(|rest| {
            let mut chars = rest.chars();
            let digit = chars.next();
            chars.next().is_none()
                && digit.is_some_and(|digit| digit.is_ascii_digit() || matches!(digit, '\u{B9}' | '\u{B2}' | '\u{B3}'))
        })
    })
}

/// true if the name is a windows device (eg nul or CON.mp3 or com1 .txt)
pub fn is_windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    is_windows_device_stem(stem.trim_end_matches(' '))
}

/// name turned into a valid file name for the flavor, bad chars become _
/// on windows trailing spaces and dots go and device names get a leading _
/// empty if nothing is left so the caller picks its own fallback
pub fn host_file_name(name: &str, flavor: PathFlavor) -> String {
    let replaced: String = name
        .chars()
        .map(|scalar| if is_refused_file_name_character(scalar, flavor) { FILE_NAME_REPLACEMENT } else { scalar })
        .collect();
    match flavor {
        PathFlavor::Posix => replaced,
        PathFlavor::Windows => {
            let kept = replaced.trim_end_matches([' ', '.']);
            if kept.is_empty() {
                return String::new();
            }
            if is_windows_device_name(kept) {
                return format!("{FILE_NAME_REPLACEMENT}{kept}");
            }
            kept.to_owned()
        }
    }
}

/// length the file system counts against its 255 limit
/// bytes on ext4 and apfs, utf-16 units on ntfs
pub fn file_name_length(name: &str, flavor: PathFlavor) -> usize {
    match flavor {
        PathFlavor::Posix => name.len(),
        PathFlavor::Windows => name.encode_utf16().count(),
    }
}
