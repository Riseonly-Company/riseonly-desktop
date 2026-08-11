pub mod app_language_catalog;
pub mod app_language_resolution;
pub mod app_plural_selector;
pub mod i18n;
pub mod relative_time;
pub mod shipped_languages;

pub use app_language_catalog::{
    ALIASES, ALL, AppLanguage, BASE_LANGUAGE, PluralRules, REVISION, alias_target, codes, language,
};
pub use app_language_resolution::{
    POPULAR_CODES, device_preferences, lookup_chain, normalize, ordered_for_display, resolve,
};
pub use app_plural_selector::PluralCategory;
pub use i18n::{
    Catalogue, Dictionary, DirectoryLocaleSource, EmptyLocaleSource, FormatArg, I18n,
    LOCALE_FILE_PREFIX, LocaleSource, format_named, format_printf, install, install_directory,
    prewarm, reload, shared, tr, tr_args, tr_named, tr_plural, tr_plural_arg, tr_plural_named,
};
pub use shipped_languages::{FALLBACK, SHIPPED, is_shipped, resolve_interface_language};
