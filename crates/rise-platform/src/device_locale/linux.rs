use super::posix_locale::gettext_identifiers;

pub(super) fn native_identifiers() -> Vec<String> {
    gettext_identifiers(|name| std::env::var(name).ok())
}

pub(super) fn native_week_rules() -> Option<(usize, usize)> {
    None
}
