//! What the machine says about where it is and what it reads.
//!
//! This exists because of one server rule: `signup_region` and `signup_language`
//! are written exactly once, at registration, and are frozen afterwards. If the
//! client sends nothing, the server infers the market from the connection — and
//! a VPN or a carrier's foreign exit node then freezes the wrong market into the
//! account permanently. So the device has to say for itself.
//!
//! Split the usual way: [`DeviceLocale::parse`] is pure and is exercised for
//! every platform's output shape from a Mac; the OS binding below it is one call
//! with no decisions in it.

use crate::host_os::HostOs;

/// The identifiers the OS reports, already parsed.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DeviceLocale {
    /// BCP-47 language tags, most preferred first, region subtags stripped.
    pub languages: Vec<String>,
    /// Uppercase ISO-3166 alpha-2, from the most preferred locale that carries
    /// one. `None` is honest and the server falls back; a guess is not.
    pub region: Option<String>,
}

impl DeviceLocale {
    /// Parses whatever the platform hands back.
    ///
    /// The three shapes differ and all three arrive here: macOS gives
    /// `["en-GB", "ru"]`, Windows gives `["en-GB", "ru-RU"]`, and a Linux
    /// `LANG` gives `ru_RU.UTF-8`. Normalising in one pure place is what lets
    /// the two untested platforms be tested anyway.
    pub fn parse<S: AsRef<str>>(raw: &[S]) -> Self {
        let mut languages: Vec<String> = Vec::new();
        let mut region: Option<String> = None;

        for entry in raw {
            // `ru_RU.UTF-8@euro` — strip the encoding and the modifier first, or
            // the region reads as "UTF-8".
            let cleaned = entry
                .as_ref()
                .split(['.', '@'])
                .next()
                .unwrap_or_default()
                .replace('_', "-");
            let mut parts = cleaned.split('-').filter(|part| !part.is_empty());

            let Some(language) = parts.next() else {
                continue;
            };
            // "C" and "POSIX" are the absence of a locale, not a language.
            if language.eq_ignore_ascii_case("C") || language.eq_ignore_ascii_case("POSIX") {
                continue;
            }

            let language = language.to_lowercase();
            if !languages.contains(&language) {
                languages.push(language);
            }

            if region.is_none()
                && let Some(candidate) = parts
                    .find(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
            {
                region = Some(candidate.to_uppercase());
            }
        }

        Self { region, languages }
    }

    pub fn is_empty(&self) -> bool {
        self.languages.is_empty() && self.region.is_none()
    }
}

/// Which mechanism a platform uses. Named so a caller can say what it read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocaleSourceKind {
    /// `NSLocale.preferredLanguages` plus `NSLocale.currentLocale`.
    AppKit,
    /// `GetUserPreferredUILanguages`.
    WindowsUiLanguages,
    /// `LC_ALL`, `LC_MESSAGES`, `LANG`, `LANGUAGE`, in that order.
    PosixEnvironment,
}

pub const fn source_for(host: HostOs) -> LocaleSourceKind {
    match host {
        HostOs::MacOs => LocaleSourceKind::AppKit,
        HostOs::Windows => LocaleSourceKind::WindowsUiLanguages,
        HostOs::Linux => LocaleSourceKind::PosixEnvironment,
    }
}

/// The current machine's locale.
///
/// Falls back to the POSIX environment on any platform whose native call returns
/// nothing, which is why a bundled `.app` launched from Finder — where `LANG` is
/// unset — still answers on macOS through AppKit, and a terminal launch answers
/// either way.
pub fn current() -> DeviceLocale {
    let native = native_identifiers();
    if !native.is_empty() {
        let parsed = DeviceLocale::parse(&native);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    DeviceLocale::parse(&posix_identifiers())
}

fn posix_identifiers() -> Vec<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .flat_map(|value| value.split(':').map(str::to_owned).collect::<Vec<_>>())
        .filter(|value| !value.is_empty())
        .collect()
}

#[cfg(target_os = "macos")]
fn native_identifiers() -> Vec<String> {
    use objc2_foundation::NSLocale;

    // `preferredLanguages` is the user's ORDERED list and is what the system
    // itself resolves against; `currentLocale` alone would lose the order and,
    // on a machine set to English with a Russian region, the second language.
    let mut identifiers: Vec<String> = NSLocale::preferredLanguages()
        .iter()
        .map(|language| language.to_string())
        .collect();

    // The region can differ from every language tag: "English (Kazakhstan)"
    // reports `en` with no subtag, and the region is the whole point here.
    let current = NSLocale::currentLocale();
    identifiers.push(current.localeIdentifier().to_string());
    identifiers
}

#[cfg(target_os = "windows")]
fn native_identifiers() -> Vec<String> {
    use windows::Win32::System::SystemInformation::GetUserPreferredUILanguages;
    use windows::Win32::System::SystemServices::MUI_LANGUAGE_NAME;

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

    // A double-null-terminated run of null-terminated strings.
    buffer
        .split(|unit| *unit == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

#[cfg(target_os = "linux")]
fn native_identifiers() -> Vec<String> {
    // The POSIX environment IS the native mechanism here, and `current` reads it
    // as the fallback for every platform, so this arm has nothing of its own to
    // add. Written out rather than left to a catch-all so that adding a real
    // mechanism — a portal, or AccountsService — is an edit here.
    Vec::new()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn native_identifiers() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_macos_style_list_keeps_its_order_and_finds_the_region() {
        let locale = DeviceLocale::parse(&["en-GB", "ru", "kk-KZ"]);
        assert_eq!(locale.languages, vec!["en", "ru", "kk"]);
        assert_eq!(locale.region.as_deref(), Some("GB"));
    }

    #[test]
    fn a_posix_lang_strips_its_encoding_and_modifier() {
        let locale = DeviceLocale::parse(&["ru_RU.UTF-8"]);
        assert_eq!(locale.languages, vec!["ru"]);
        assert_eq!(
            locale.region.as_deref(),
            Some("RU"),
            "reading past the dot would report the region as UTF"
        );

        assert_eq!(
            DeviceLocale::parse(&["de_DE@euro"]).region.as_deref(),
            Some("DE")
        );
    }

    #[test]
    fn a_windows_style_list_parses_the_same_way() {
        let locale = DeviceLocale::parse(&["en-GB", "ru-RU"]);
        assert_eq!(locale.languages, vec!["en", "ru"]);
        assert_eq!(locale.region.as_deref(), Some("GB"));
    }

    #[test]
    fn the_region_comes_from_the_first_identifier_that_has_one() {
        let locale = DeviceLocale::parse(&["en", "ru-KZ"]);
        assert_eq!(locale.languages, vec!["en", "ru"]);
        assert_eq!(locale.region.as_deref(), Some("KZ"));
    }

    #[test]
    fn the_absence_of_a_locale_is_not_a_language_called_c() {
        assert_eq!(DeviceLocale::parse(&["C"]), DeviceLocale::default());
        assert_eq!(DeviceLocale::parse(&["POSIX"]), DeviceLocale::default());
        assert!(DeviceLocale::parse(&["C.UTF-8"]).is_empty());
    }

    #[test]
    fn a_repeated_language_appears_once() {
        let locale = DeviceLocale::parse(&["en-GB", "en-US", "en"]);
        assert_eq!(locale.languages, vec!["en"]);
        assert_eq!(locale.region.as_deref(), Some("GB"));
    }

    #[test]
    fn a_three_letter_subtag_is_a_script_or_a_variant_and_not_a_region() {
        let locale = DeviceLocale::parse(&["zh-Hans-CN"]);
        assert_eq!(locale.languages, vec!["zh"]);
        assert_eq!(
            locale.region.as_deref(),
            Some("CN"),
            "Hans is a script; the two-letter subtag after it is the region"
        );
    }

    #[test]
    fn nothing_at_all_is_reported_as_nothing_rather_than_a_default_market() {
        let locale = DeviceLocale::parse::<String>(&[]);
        assert!(locale.is_empty());
        assert_eq!(locale.region, None);
    }

    #[test]
    fn every_platform_names_the_mechanism_it_would_use() {
        for host in HostOs::ALL {
            let _ = source_for(host);
        }
        assert_eq!(source_for(HostOs::MacOs), LocaleSourceKind::AppKit);
        assert_eq!(
            source_for(HostOs::Linux),
            LocaleSourceKind::PosixEnvironment
        );
    }

    #[test]
    fn reading_the_real_machine_never_panics() {
        let _ = current();
    }
}
