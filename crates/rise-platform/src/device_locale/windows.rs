use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

pub(super) fn native_identifiers() -> Vec<String> {
    let mut count = 0u32;
    let mut length = 0u32;
    let ok = unsafe {
        GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut length).is_ok()
    };
    if !ok || length == 0 {
        return Vec::new();
    }

    let mut buffer = vec![0u16; length as usize];
    let ok = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(windows::core::PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )
        .is_ok()
    };
    if !ok {
        return Vec::new();
    }

    buffer
        .split(|unit| *unit == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

pub(super) fn native_week_rules() -> Option<(usize, usize)> {
    None
}
