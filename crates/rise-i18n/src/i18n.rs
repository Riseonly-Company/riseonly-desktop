use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use gpui::App;
use rise_i18n_catalog::{LOCALE_FILE_PREFIX, MISSING_PLURAL_CATEGORY_FALLBACK, PluralCategory};

use crate::app_language_catalog::{AppLanguageCatalog, AppPluralRules};
use crate::app_plural_selector::AppPluralSelector;

pub type Dictionary = HashMap<String, String>;

/// where parsed locales come from
/// the desktop app installs its resource dir at launch and tests install a fixture
pub trait LocaleSource: Send + Sync {
    fn load(&self, code: &str) -> Option<Dictionary>;
}

#[derive(Debug, Clone)]
pub struct DirectoryLocaleSource {
    root: PathBuf,
}

impl DirectoryLocaleSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_for(&self, code: &str) -> PathBuf {
        self.root.join(format!("{LOCALE_FILE_PREFIX}{code}.json"))
    }
}

impl LocaleSource for DirectoryLocaleSource {
    fn load(&self, code: &str) -> Option<Dictionary> {
        let raw = fs::read_to_string(self.path_for(code)).ok()?;
        serde_json::from_str::<Dictionary>(&raw).ok()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyLocaleSource;

impl LocaleSource for EmptyLocaleSource {
    fn load(&self, _code: &str) -> Option<Dictionary> {
        None
    }
}

/// one argument for a format template
#[derive(Debug, Clone, PartialEq)]
pub enum FormatArg<'a> {
    Text(Cow<'a, str>),
    Int(i64),
    Float(f64),
}

impl FormatArg<'_> {
    fn as_catalog(&self) -> rise_i18n_catalog::FormatArg<'_> {
        match self {
            FormatArg::Text(text) => rise_i18n_catalog::FormatArg::Text(text.as_ref()),
            FormatArg::Int(value) => rise_i18n_catalog::FormatArg::Int(*value),
            FormatArg::Float(value) => rise_i18n_catalog::FormatArg::Float(*value),
        }
    }
}

impl<'a> From<&'a str> for FormatArg<'a> {
    fn from(value: &'a str) -> Self {
        Self::Text(Cow::Borrowed(value))
    }
}

impl<'a> From<&'a String> for FormatArg<'a> {
    fn from(value: &'a String) -> Self {
        Self::Text(Cow::Borrowed(value.as_str()))
    }
}

impl From<String> for FormatArg<'_> {
    fn from(value: String) -> Self {
        Self::Text(Cow::Owned(value))
    }
}

macro_rules! from_int {
    ($($ty:ty),*) => {
        $(impl From<$ty> for FormatArg<'_> {
            fn from(value: $ty) -> Self {
                Self::Int(i64::from(value))
            }
        })*
    };
}

from_int!(i8, i16, i32, i64, u8, u16, u32);

impl From<usize> for FormatArg<'_> {
    fn from(value: usize) -> Self {
        Self::Int(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<f64> for FormatArg<'_> {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

/// printf style format, supports %@ %s %d %f and %n$ and %%
pub fn format_printf(template: &str, args: &[FormatArg<'_>]) -> String {
    let converted: Vec<rise_i18n_catalog::FormatArg<'_>> =
        args.iter().map(FormatArg::as_catalog).collect();
    rise_i18n_catalog::format_printf(template, &converted)
}

/// braced format, each {name} is replaced once from the key value list
pub fn format_named(template: &str, args: &[(&str, &str)]) -> String {
    rise_i18n_catalog::format_braced(template, args)
}

pub struct I18n {
    code: &'static str,
    plural: AppPluralRules,
    dict: Arc<Dictionary>,
    fallbacks: Vec<Arc<Dictionary>>,
    pub decimal_separator: &'static str,
}

const BRANDS: [(&str, &str, &str); 1] = [("{{maps}}", "brand_maps", "Maps")];

struct Runtime {
    source: Arc<dyn LocaleSource>,
    shared: Arc<I18n>,
}

fn runtime() -> &'static RwLock<Runtime> {
    static RUNTIME: OnceLock<RwLock<Runtime>> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        let source: Arc<dyn LocaleSource> = Arc::new(EmptyLocaleSource);
        let shared = Arc::new(I18n::load(&source, cache_generation(), AppLanguageCatalog::BASE_LANGUAGE));
        RwLock::new(Runtime { source, shared })
    })
}

/// parsed dictionaries of one locale source
#[derive(Default)]
struct DictionaryCache {
    generation: u64,
    entries: HashMap<String, Arc<Dictionary>>,
}

/// parsed dictionaries are cached
/// switching language rebuilds the scene and would re-read the same ~150 KB json
fn cache() -> &'static RwLock<DictionaryCache> {
    static CACHE: OnceLock<RwLock<DictionaryCache>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(DictionaryCache::default()))
}

fn cache_generation() -> u64 {
    cache().read().unwrap_or_else(PoisonError::into_inner).generation
}

impl I18n {
    fn new(
        code: &'static str,
        plural: AppPluralRules,
        dict: Arc<Dictionary>,
        fallbacks: Vec<Arc<Dictionary>>,
    ) -> Self {
        Self {
            code,
            plural,
            dict,
            fallbacks,
            decimal_separator: rise_format::decimal_separator(code),
        }
    }

    pub fn shared() -> Arc<I18n> {
        let guard = runtime().read().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(&guard.shared)
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn translate<'a>(&'a self, key: &'a str) -> &'a str {
        if let Some(value) = self.dict.get(key) {
            return value;
        }
        for fallback in &self.fallbacks {
            if let Some(value) = fallback.get(key) {
                return value;
            }
        }
        if cfg!(debug_assertions) {
            tracing::debug!("[i18n] missing key: {key} ({})", self.code);
        }
        key
    }

    /// plural family of the active language, for resolving a counted string off the main thread
    /// from templates captured up front
    pub fn plural_rules(&self) -> AppPluralRules {
        self.plural
    }

    pub fn plural_category(&self, n: i64) -> PluralCategory {
        AppPluralSelector::category(self.plural, n)
    }

    pub fn plural_variant(&self, n: i64) -> &'static str {
        self.plural_category(n).as_str()
    }

    /// falls back to the canon category when the language lacks the exact one
    /// every catalogue has it so a counted string is never a raw key
    fn plural_template(&self, key: &str, n: i64) -> String {
        let exact = format!("{key}_{}", self.plural_variant(n));
        let defined = self.dict.contains_key(&exact)
            || self.fallbacks.iter().any(|fallback| fallback.contains_key(&exact));
        if defined {
            return self.translate(&exact).to_owned();
        }
        let fallback_key = format!("{key}_{}", MISSING_PLURAL_CATEGORY_FALLBACK.as_str());
        self.translate(&fallback_key).to_owned()
    }

    pub fn translate_plural(&self, key: &str, n: i64) -> String {
        format_printf(&self.plural_template(key, n), &[FormatArg::Int(n)])
    }

    pub fn translate_plural_with(&self, key: &str, n: i64, arg: FormatArg<'_>) -> String {
        format_printf(&self.plural_template(key, n), &[arg])
    }

    pub fn translate_args(&self, key: &str, args: &[FormatArg<'_>]) -> String {
        format_printf(self.translate(key), args)
    }

    pub fn translate_named(&self, key: &str, args: &[(&str, &str)]) -> String {
        format_named(self.translate(key), args)
    }

    pub fn translate_plural_named(&self, key: &str, n: i64, args: &[(&str, &str)]) -> String {
        format_named(&self.plural_template(key, n), args)
    }

    pub fn reload(code: &str) {
        let (source, generation) = {
            let guard = runtime().read().unwrap_or_else(PoisonError::into_inner);
            (Arc::clone(&guard.source), cache_generation())
        };
        let loaded = Arc::new(I18n::load(&source, generation, code));
        runtime().write().unwrap_or_else(PoisonError::into_inner).shared = loaded;
    }

    pub fn install(source: Arc<dyn LocaleSource>) {
        let mut guard = runtime().write().unwrap_or_else(PoisonError::into_inner);
        let generation = {
            let mut cache = cache().write().unwrap_or_else(PoisonError::into_inner);
            cache.generation = cache.generation.wrapping_add(1);
            cache.entries.clear();
            cache.generation
        };
        let code = guard.shared.code;
        guard.shared = Arc::new(I18n::load(&source, generation, code));
        guard.source = source;
    }

    pub fn install_directory(root: impl Into<PathBuf>) {
        Self::install(Arc::new(DirectoryLocaleSource::new(root)));
    }

    fn dictionary(source: &Arc<dyn LocaleSource>, generation: u64, code: &str) -> Arc<Dictionary> {
        {
            let cache = cache().read().unwrap_or_else(PoisonError::into_inner);
            if cache.generation == generation
                && let Some(cached) = cache.entries.get(code)
            {
                return Arc::clone(cached);
            }
        }

        let Some(dictionary) = Self::load_dict(source, code) else {
            return Arc::new(Dictionary::new());
        };
        let loaded = Arc::new(dictionary);
        let mut cache = cache().write().unwrap_or_else(PoisonError::into_inner);
        if cache.generation == generation {
            cache.entries.insert(code.to_owned(), Arc::clone(&loaded));
        }
        loaded
    }

    fn load(source: &Arc<dyn LocaleSource>, generation: u64, code: &str) -> I18n {
        let resolved = AppLanguageCatalog::language(code)
            .or_else(|| AppLanguageCatalog::language(AppLanguageCatalog::BASE_LANGUAGE));
        let resolved_code = resolved.map_or(AppLanguageCatalog::BASE_LANGUAGE, |language| language.code);
        let chain = AppLanguageCatalog::lookup_chain(resolved_code);
        let primary = chain
            .first()
            .copied()
            .unwrap_or(AppLanguageCatalog::BASE_LANGUAGE);

        I18n::new(
            primary,
            resolved.map_or(AppPluralRules::OneOther, |language| language.plural),
            Self::dictionary(source, generation, primary),
            chain
                .iter()
                .skip(1)
                .map(|entry| Self::dictionary(source, generation, entry))
                .collect(),
        )
    }

    fn load_dict(source: &Arc<dyn LocaleSource>, code: &str) -> Option<Dictionary> {
        source.load(code).map(Self::brands_resolved)
    }

    fn brands_resolved(raw: Dictionary) -> Dictionary {
        let mut resolved = raw.clone();
        for (token, key, fallback) in BRANDS {
            let name = raw.get(key).map_or(fallback, String::as_str);
            for (entry_key, value) in &raw {
                if value.contains(token) {
                    let current = resolved.get(entry_key).unwrap_or(value);
                    let replaced = current.replace(token, name);
                    resolved.insert(entry_key.clone(), replaced);
                }
            }
        }
        resolved
    }

    pub fn prewarm(code: &str, cx: &App) {
        let chain = AppLanguageCatalog::lookup_chain(code);
        let (source, generation) = {
            let guard = runtime().read().unwrap_or_else(PoisonError::into_inner);
            (Arc::clone(&guard.source), cache_generation())
        };
        cx.background_executor()
            .spawn(async move {
                for entry in chain {
                    let _ = Self::dictionary(&source, generation, entry);
                }
            })
            .detach();
    }
}

pub fn tr(key: &str) -> String {
    I18n::shared().translate(key).to_owned()
}

pub fn tr_plural(key: &str, n: i64) -> String {
    I18n::shared().translate_plural(key, n)
}

pub fn tr_plural_with(key: &str, n: i64, arg: FormatArg<'_>) -> String {
    I18n::shared().translate_plural_with(key, n, arg)
}

pub fn tr_args(key: &str, args: &[FormatArg<'_>]) -> String {
    I18n::shared().translate_args(key, args)
}

pub fn tr_named(key: &str, args: &[(&str, &str)]) -> String {
    I18n::shared().translate_named(key, args)
}

pub fn tr_plural_named(key: &str, n: i64, args: &[(&str, &str)]) -> String {
    I18n::shared().translate_plural_named(key, n, args)
}
