use crate::host_os::HostOs;

/// identifiers the os reports, already parsed
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DeviceLocale {
    pub languages: Vec<String>,
    pub region: Option<String>,
}

impl DeviceLocale {
    /// parses what the platform hands back (en-GB or ru_RU.UTF-8@euro) in preference order
    pub fn parse<S: AsRef<str>>(raw: &[S]) -> Self {
        let mut languages: Vec<String> = Vec::new();
        let mut region: Option<String> = None;

        for entry in raw {
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

/// which mechanism a platform uses to report the locale
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocaleSourceKind {
    AppKit,
    WindowsUiLanguages,
    PosixEnvironment,
}

pub const fn source_for(host: HostOs) -> LocaleSourceKind {
    match host {
        HostOs::MacOs => LocaleSourceKind::AppKit,
        HostOs::Windows => LocaleSourceKind::WindowsUiLanguages,
        HostOs::Linux => LocaleSourceKind::PosixEnvironment,
    }
}

/// locale of the machine, falls back to the posix environment when the native call is empty
pub fn current() -> DeviceLocale {
    resolve(
        source_for(HostOs::current()),
        &super::os::native_identifiers(),
        posix_identifiers,
    )
}

/// picks the locale from what the native mechanism reported
/// reads the plain environment only where the mechanism isnt the environment already
pub fn resolve(
    source: LocaleSourceKind,
    native: &[String],
    environment: impl FnOnce() -> Vec<String>,
) -> DeviceLocale {
    let parsed = DeviceLocale::parse(native);
    if !parsed.is_empty() || source == LocaleSourceKind::PosixEnvironment {
        return parsed;
    }
    DeviceLocale::parse(&environment())
}

/// week rules of the users region
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WeekRules {
    pub first_weekday: u8,
    pub minimum_days_in_first_week: u8,
}

/// week rules of the region, none when the platform reports nothing usable
pub fn week_rules() -> Option<WeekRules> {
    let (first_weekday, minimum_days) = super::os::native_week_rules()?;
    let valid = |value: usize| (1..=7).contains(&value);
    if !valid(first_weekday) || !valid(minimum_days) {
        return None;
    }
    Some(WeekRules {
        first_weekday: first_weekday as u8,
        minimum_days_in_first_week: minimum_days as u8,
    })
}

fn posix_identifiers() -> Vec<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .flat_map(|value| value.split(':').map(str::to_owned).collect::<Vec<_>>())
        .filter(|value| !value.is_empty())
        .collect()
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

    fn owned(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn an_empty_gettext_list_on_linux_is_final() {
        let locale = resolve(LocaleSourceKind::PosixEnvironment, &[], || owned(&["ru"]));
        assert!(
            locale.is_empty(),
            "LANG=C with LANGUAGE=ru runs gettext apps untranslated, the locale must not say ru"
        );
    }

    #[test]
    fn a_c_locale_list_on_linux_is_final_too() {
        let locale = resolve(LocaleSourceKind::PosixEnvironment, &owned(&["C.UTF-8"]), || {
            owned(&["ru"])
        });
        assert!(locale.is_empty());
    }

    #[test]
    fn the_gettext_list_on_linux_is_taken_as_is() {
        let locale = resolve(
            LocaleSourceKind::PosixEnvironment,
            &owned(&["ru", "en", "en_US.UTF-8"]),
            || owned(&["de_DE"]),
        );
        assert_eq!(locale.languages, vec!["ru", "en"]);
        assert_eq!(locale.region.as_deref(), Some("US"));
    }

    #[test]
    fn an_empty_native_list_elsewhere_falls_back_to_the_environment() {
        for source in [LocaleSourceKind::AppKit, LocaleSourceKind::WindowsUiLanguages] {
            let locale = resolve(source, &[], || owned(&["ru_RU.UTF-8"]));
            assert_eq!(locale.languages, vec!["ru"]);
            assert_eq!(locale.region.as_deref(), Some("RU"));
        }
    }

    #[test]
    fn a_native_list_elsewhere_wins_over_the_environment() {
        let locale = resolve(LocaleSourceKind::AppKit, &owned(&["en-GB"]), || {
            owned(&["ru_RU"])
        });
        assert_eq!(locale.languages, vec!["en"]);
    }

    #[test]
    fn reading_the_real_machine_never_panics() {
        let _ = current();
    }

    #[test]
    fn the_week_rules_of_the_machine_are_in_range_when_reported() {
        if let Some(rules) = week_rules() {
            assert!((1..=7).contains(&rules.first_weekday));
            assert!((1..=7).contains(&rules.minimum_days_in_first_week));
        }
    }
}
