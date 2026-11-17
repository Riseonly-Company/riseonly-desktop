pub const MESSAGES_VARIABLES: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

pub const LANGUAGE_VARIABLE: &str = "LANGUAGE";

/// locale identifiers of the environment lookup reads, most preferred first and in gettext order
/// empty when the environment names no messages locale
pub fn gettext_identifiers(lookup: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let set = |name: &str| lookup(name).filter(|value| !value.is_empty());
    let Some(messages) = MESSAGES_VARIABLES.iter().find_map(|&name| set(name)) else {
        return Vec::new();
    };
    if is_untranslated(&messages) {
        return Vec::new();
    }
    let mut identifiers: Vec<String> = set(LANGUAGE_VARIABLE)
        .map(|list| {
            list.split(':')
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if !identifiers.contains(&messages) {
        identifiers.push(messages);
    }
    identifiers
}

/// true when the locale name is C or POSIX, with or without an encoding (eg C.UTF-8)
pub fn is_untranslated(locale: &str) -> bool {
    let name = locale.split(['.', '@']).next().unwrap_or_default();
    name.eq_ignore_ascii_case("C") || name.eq_ignore_ascii_case("POSIX")
}
