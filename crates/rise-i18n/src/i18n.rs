//! Runtime string catalogue: lookup chain `chosen -> base of a regional variant -> en`.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::app_language_catalog::{BASE_LANGUAGE, PluralRules, language};
use crate::app_language_resolution::{lookup_chain, normalize};
use crate::app_plural_selector::PluralCategory;

pub type Dictionary = HashMap<String, String>;

pub const LOCALE_FILE_PREFIX: &str = "locale-";

/// Where parsed locales come from; the caller installs the directory or fixture.
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

/// Renders every key as itself: what a translate call gets before a source is installed.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyLocaleSource;

impl LocaleSource for EmptyLocaleSource {
    fn load(&self, _code: &str) -> Option<Dictionary> {
        None
    }
}

/// A locale source plus a cache of parsed dictionaries, one per language code.
pub struct Catalogue {
    source: Arc<dyn LocaleSource>,
    cache: RwLock<HashMap<String, Arc<Dictionary>>>,
}

impl Catalogue {
    pub fn new(source: Arc<dyn LocaleSource>) -> Self {
        Self {
            source,
            cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn directory(root: impl Into<PathBuf>) -> Self {
        Self::new(Arc::new(DirectoryLocaleSource::new(root)))
    }

    pub fn dictionary(&self, code: &str) -> Arc<Dictionary> {
        if let Some(cached) = self
            .cache
            .read()
            .expect("locale cache poisoned")
            .get(code)
            .cloned()
        {
            return cached;
        }

        let loaded = Arc::new(self.source.load(code).unwrap_or_default());
        self.cache
            .write()
            .expect("locale cache poisoned")
            .insert(code.to_owned(), Arc::clone(&loaded));
        loaded
    }

    /// Warm a language's dictionaries off the UI thread so the later `load` is a
    /// map lookup rather than disk I/O plus a parse.
    pub fn prewarm(&self, code: &str) {
        for entry in lookup_chain(code) {
            let _ = self.dictionary(entry);
        }
    }

    /// Load a language. The tag is normalised first, so `ru_RU` resolves to `ru`;
    /// an unrecognised tag falls back to [`BASE_LANGUAGE`].
    pub fn load(&self, code: &str) -> I18n {
        let resolved = normalize(code)
            .and_then(language)
            .or_else(|| language(BASE_LANGUAGE));
        let resolved_code = resolved.map_or(BASE_LANGUAGE, |entry| entry.code);
        let chain = lookup_chain(resolved_code);
        let primary = chain.first().copied().unwrap_or(BASE_LANGUAGE);

        I18n {
            code: primary,
            plural: resolved.map_or(PluralRules::OneOther, |entry| entry.plural),
            dict: self.dictionary(primary),
            fallbacks: chain
                .iter()
                .skip(1)
                .map(|entry| self.dictionary(entry))
                .collect(),
        }
    }
}

pub struct I18n {
    code: &'static str,
    plural: PluralRules,
    dict: Arc<Dictionary>,
    fallbacks: Vec<Arc<Dictionary>>,
}

impl I18n {
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn plural_rules(&self) -> PluralRules {
        self.plural
    }

    pub fn lookup(&self, key: &str) -> Option<&str> {
        if let Some(value) = self.dict.get(key) {
            return Some(value);
        }
        self.fallbacks
            .iter()
            .find_map(|fallback| fallback.get(key))
            .map(String::as_str)
    }

    /// A key nothing in the chain defines renders as itself, never as another language's copy.
    pub fn translate<'a>(&'a self, key: &'a str) -> &'a str {
        self.lookup(key).unwrap_or(key)
    }

    /// printf dialect: `%@ %s %d %f`, `%n$` indices, `%%`.
    pub fn translate_args(&self, key: &str, args: &[FormatArg<'_>]) -> String {
        format_printf(self.translate(key), args)
    }

    /// Braced dialect: `{name}`, substituted from a key-value list.
    pub fn translate_named(&self, key: &str, args: &[(&str, &str)]) -> String {
        format_named(self.translate(key), args)
    }

    pub fn plural_variant(&self, n: i64) -> PluralCategory {
        self.plural.category(n)
    }

    /// Resolve a counted key, falling back to the `_other` suffix when this
    /// language does not define the exact category.
    pub fn plural_template(&self, key: &str, n: i64) -> String {
        let exact = format!("{key}_{}", self.plural_variant(n).as_str());
        if let Some(value) = self.lookup(&exact) {
            return value.to_owned();
        }
        let other = format!("{key}_other");
        self.lookup(&other).unwrap_or(&other).to_owned()
    }

    pub fn translate_plural(&self, key: &str, n: i64) -> String {
        format_printf(&self.plural_template(key, n), &[FormatArg::Int(n)])
    }

    pub fn translate_plural_arg(&self, key: &str, n: i64, arg: FormatArg<'_>) -> String {
        format_printf(&self.plural_template(key, n), &[arg])
    }

    /// For plural-suffixed keys whose values use the braced dialect (`chat_thread_title_*`).
    pub fn translate_plural_named(&self, key: &str, n: i64, args: &[(&str, &str)]) -> String {
        format_named(&self.plural_template(key, n), args)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FormatArg<'a> {
    Text(Cow<'a, str>),
    Int(i64),
    Float(f64),
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

struct Spec {
    index: Option<usize>,
    left_justify: bool,
    zero_pad: bool,
    width: Option<usize>,
    precision: Option<usize>,
    conversion: u8,
    end: usize,
}

/// Caps what one specifier can allocate; `format!` also panics past `u16::MAX` float precision.
const MAX_FIELD_WIDTH: usize = 1024;

/// Parses `%[index$][flags][width][.precision]conversion`; `None` means emit verbatim.
fn parse_spec(bytes: &[u8], start: usize) -> Option<Spec> {
    let mut at = start + 1;
    if at >= bytes.len() {
        return None;
    }
    if bytes[at] == b'%' {
        return Some(Spec {
            index: None,
            left_justify: false,
            zero_pad: false,
            width: None,
            precision: None,
            conversion: b'%',
            end: at + 1,
        });
    }

    let digits_end = scan_digits(bytes, at);
    let index = if digits_end > at && bytes.get(digits_end) == Some(&b'$') {
        let parsed = parse_usize(bytes, at, digits_end)?;
        at = digits_end + 1;
        Some(parsed)
    } else {
        None
    };

    let mut left_justify = false;
    let mut zero_pad = false;
    while let Some(flag) = bytes.get(at) {
        match flag {
            b'-' => left_justify = true,
            b'0' => zero_pad = true,
            b'+' | b' ' | b'#' | b'\'' => {}
            _ => break,
        }
        at += 1;
    }

    let width_end = scan_digits(bytes, at);
    let width = if width_end > at {
        let parsed = parse_usize(bytes, at, width_end)?;
        if parsed > MAX_FIELD_WIDTH {
            return None;
        }
        at = width_end;
        Some(parsed)
    } else {
        None
    };

    let precision = if bytes.get(at) == Some(&b'.') {
        at += 1;
        let end = scan_digits(bytes, at);
        let parsed = if end > at {
            parse_usize(bytes, at, end)?
        } else {
            0
        };
        if parsed > MAX_FIELD_WIDTH {
            return None;
        }
        at = end;
        Some(parsed)
    } else {
        None
    };

    for modifier in [
        b"hh".as_slice(),
        b"ll",
        b"h",
        b"l",
        b"q",
        b"z",
        b"t",
        b"j",
        b"L",
    ] {
        if bytes[at..].starts_with(modifier) {
            at += modifier.len();
            break;
        }
    }

    let conversion = *bytes.get(at)?;
    if !matches!(
        conversion,
        b'@' | b's' | b'S' | b'd' | b'i' | b'u' | b'f' | b'F'
    ) {
        return None;
    }

    Some(Spec {
        index,
        left_justify,
        zero_pad,
        width,
        precision,
        conversion,
        end: at + 1,
    })
}

fn scan_digits(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    at
}

fn parse_usize(bytes: &[u8], from: usize, to: usize) -> Option<usize> {
    std::str::from_utf8(&bytes[from..to]).ok()?.parse().ok()
}

fn render_arg(arg: &FormatArg<'_>, spec: &Spec) -> String {
    match (spec.conversion, arg) {
        (b'f' | b'F', FormatArg::Float(value)) => {
            format!("{:.*}", spec.precision.unwrap_or(6), value)
        }
        (b'f' | b'F', FormatArg::Int(value)) => {
            format!("{:.*}", spec.precision.unwrap_or(6), *value as f64)
        }
        (b'd' | b'i' | b'u', FormatArg::Float(value)) => format!("{}", *value as i64),
        (_, FormatArg::Int(value)) => value.to_string(),
        (_, FormatArg::Float(value)) => value.to_string(),
        (_, FormatArg::Text(value)) => match spec.precision {
            Some(limit) if matches!(spec.conversion, b'@' | b's' | b'S') => {
                value.chars().take(limit).collect()
            }
            _ => value.as_ref().to_owned(),
        },
    }
}

fn pad(body: String, spec: &Spec) -> String {
    let Some(width) = spec.width else {
        return body;
    };
    let length = body.chars().count();
    if length >= width {
        return body;
    }
    let fill = width - length;
    if spec.left_justify {
        let mut out = body;
        out.extend(std::iter::repeat_n(' ', fill));
        return out;
    }
    let filler = if spec.zero_pad { '0' } else { ' ' };
    let mut out: String = std::iter::repeat_n(filler, fill).collect();
    out.push_str(&body);
    out
}

/// printf-style substitution.
///
/// Positional and `%n$` indexed specifiers, `%@ %s %d %f` and a literal `%%`. A
/// malformed specifier or a missing argument degrades to the specifier text
/// itself rather than panicking.
pub fn format_printf(template: &str, args: &[FormatArg<'_>]) -> String {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len() + 16);
    let mut literal_start = 0usize;
    let mut next_arg = 0usize;
    let mut at = 0usize;

    while at < bytes.len() {
        if bytes[at] != b'%' {
            at += 1;
            continue;
        }
        out.push_str(&template[literal_start..at]);

        match parse_spec(bytes, at) {
            None => {
                out.push('%');
                at += 1;
            }
            Some(spec) if spec.conversion == b'%' => {
                out.push('%');
                at = spec.end;
            }
            Some(spec) => {
                let index = match spec.index {
                    Some(0) => None,
                    Some(explicit) => Some(explicit - 1),
                    None => {
                        let position = next_arg;
                        next_arg += 1;
                        Some(position)
                    }
                };
                match index.and_then(|position| args.get(position)) {
                    Some(arg) => out.push_str(&pad(render_arg(arg, &spec), &spec)),
                    None => out.push_str(&template[at..spec.end]),
                }
                at = spec.end;
            }
        }
        literal_start = at;
    }

    out.push_str(&template[literal_start..]);
    out
}

/// Braced substitution: `{name}` from a key-value list.
///
/// One pass, so a substituted value that itself contains braces is never
/// re-substituted. An unknown placeholder is left intact.
pub fn format_named(template: &str, args: &[(&str, &str)]) -> String {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len() + 16);
    let mut literal_start = 0usize;
    let mut at = 0usize;

    while at < bytes.len() {
        if bytes[at] != b'{' {
            at += 1;
            continue;
        }
        let mut cursor = at + 1;
        while bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            cursor += 1;
        }
        if cursor == at + 1 || bytes.get(cursor) != Some(&b'}') {
            at += 1;
            continue;
        }

        let name = &template[at + 1..cursor];
        if let Some((_, value)) = args.iter().find(|(key, _)| *key == name) {
            out.push_str(&template[literal_start..at]);
            out.push_str(value);
            literal_start = cursor + 1;
        }
        at = cursor + 1;
    }

    out.push_str(&template[literal_start..]);
    out
}

struct Runtime {
    catalogue: Arc<Catalogue>,
    active: Arc<I18n>,
}

fn runtime() -> &'static RwLock<Runtime> {
    static RUNTIME: OnceLock<RwLock<Runtime>> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        let catalogue = Arc::new(Catalogue::new(Arc::new(EmptyLocaleSource)));
        let active = Arc::new(catalogue.load(BASE_LANGUAGE));
        RwLock::new(Runtime { catalogue, active })
    })
}

/// Point the process at a locale source, keeping whatever language is active.
pub fn install(source: Arc<dyn LocaleSource>) {
    let mut guard = runtime().write().expect("i18n runtime poisoned");
    let code = guard.active.code;
    let catalogue = Arc::new(Catalogue::new(source));
    guard.active = Arc::new(catalogue.load(code));
    guard.catalogue = catalogue;
}

pub fn install_directory(root: impl Into<PathBuf>) {
    install(Arc::new(DirectoryLocaleSource::new(root)));
}

pub fn reload(code: &str) {
    let catalogue = Arc::clone(&runtime().read().expect("i18n runtime poisoned").catalogue);
    let loaded = Arc::new(catalogue.load(code));
    runtime().write().expect("i18n runtime poisoned").active = loaded;
}

/// Warm a language ahead of a switch. Synchronous: the caller picks the thread that pays the parse.
pub fn prewarm(code: &str) {
    let catalogue = Arc::clone(&runtime().read().expect("i18n runtime poisoned").catalogue);
    catalogue.prewarm(code);
}

pub fn shared() -> Arc<I18n> {
    Arc::clone(&runtime().read().expect("i18n runtime poisoned").active)
}

pub fn tr(key: &str) -> String {
    shared().translate(key).to_owned()
}

pub fn tr_args(key: &str, args: &[FormatArg<'_>]) -> String {
    shared().translate_args(key, args)
}

pub fn tr_named(key: &str, args: &[(&str, &str)]) -> String {
    shared().translate_named(key, args)
}

pub fn tr_plural(key: &str, n: i64) -> String {
    shared().translate_plural(key, n)
}

pub fn tr_plural_arg(key: &str, n: i64, arg: FormatArg<'_>) -> String {
    shared().translate_plural_arg(key, n, arg)
}

pub fn tr_plural_named(key: &str, n: i64, args: &[(&str, &str)]) -> String {
    shared().translate_plural_named(key, n, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MapSource {
        locales: HashMap<String, Dictionary>,
        loads: AtomicUsize,
    }

    impl MapSource {
        fn new(locales: &[(&str, &[(&str, &str)])]) -> Self {
            Self {
                locales: locales
                    .iter()
                    .map(|(code, entries)| {
                        let dict = entries
                            .iter()
                            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                            .collect();
                        ((*code).to_owned(), dict)
                    })
                    .collect(),
                loads: AtomicUsize::new(0),
            }
        }
    }

    impl LocaleSource for MapSource {
        fn load(&self, code: &str) -> Option<Dictionary> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            self.locales.get(code).cloned()
        }
    }

    fn fixture() -> Catalogue {
        Catalogue::new(Arc::new(MapSource::new(&[
            (
                "en",
                &[
                    ("hello", "Hello"),
                    ("greet", "Hello, %@!"),
                    ("both", "%1$@ added %2$@"),
                    ("progress", "Uploading · %d%%"),
                    ("millions", "%.1fM"),
                    ("members_one", "%d member"),
                    ("members_other", "%d members"),
                    ("guests_other", "%d guests"),
                    ("chat_thread_title_one", "{count} comment"),
                    ("chat_thread_title_other", "{count} comments"),
                    ("named", "Delete the chat with {name} permanently?"),
                    ("doc", "Use /bot<TOKEN>/{method} with the Bot API."),
                ],
            ),
            ("pt", &[("hello", "Olá"), ("members_one", "%d membro")]),
            ("pt-BR", &[("hello", "Oi")]),
            (
                "ru",
                &[
                    ("hello", "Привет"),
                    ("members_one", "%d участник"),
                    ("members_few", "%d участника"),
                    ("members_many", "%d участников"),
                    ("members_other", "%d участников"),
                    ("chat_thread_title_one", "{count} комментарий"),
                    ("chat_thread_title_few", "{count} комментария"),
                    ("chat_thread_title_many", "{count} комментариев"),
                    ("chat_thread_title_other", "{count} комментариев"),
                ],
            ),
            ("de", &[("ghost", "Geist")]),
        ])))
    }

    #[test]
    fn a_missing_key_renders_as_itself() {
        let catalogue = fixture();
        let ru = catalogue.load("ru");
        assert_eq!(
            ru.translate("ghost"),
            "ghost",
            "a key only German defines must never leak German copy"
        );
        assert_eq!(ru.translate("nonexistent_key"), "nonexistent_key");
    }

    #[test]
    fn the_fallback_chain_walks_variant_then_base_then_english() {
        let catalogue = fixture();
        let br = catalogue.load("pt-BR");
        assert_eq!(br.code(), "pt-BR");
        assert_eq!(br.translate("hello"), "Oi");
        assert_eq!(
            br.translate("members_one"),
            "%d membro",
            "missing in pt-BR, present in pt"
        );
        assert_eq!(
            br.translate("greet"),
            "Hello, %@!",
            "missing in pt-BR and pt, present in en"
        );
    }

    #[test]
    fn an_unknown_language_loads_the_base_language() {
        let catalogue = fixture();
        let unknown = catalogue.load("klingon");
        assert_eq!(unknown.code(), "en");
        assert_eq!(unknown.plural_rules(), PluralRules::OneOther);
        assert_eq!(unknown.translate("hello"), "Hello");
    }

    #[test]
    fn a_tag_that_is_not_a_catalogue_code_still_loads_its_language() {
        let catalogue = fixture();
        let ru = catalogue.load("ru_RU");
        assert_eq!(ru.code(), "ru");
        assert_eq!(ru.plural_rules(), PluralRules::Slavic);
        assert_eq!(ru.translate("hello"), "Привет");
    }

    #[test]
    fn dictionaries_are_parsed_once_per_code() {
        let source = Arc::new(MapSource::new(&[("en", &[("hello", "Hello")])]));
        let catalogue = Catalogue::new(Arc::clone(&source) as Arc<dyn LocaleSource>);

        catalogue.prewarm("ru");
        let after_prewarm = source.loads.load(Ordering::SeqCst);
        assert_eq!(after_prewarm, 2, "ru and en");

        let _ = catalogue.load("ru");
        let _ = catalogue.load("ru");
        assert_eq!(
            source.loads.load(Ordering::SeqCst),
            after_prewarm,
            "a prewarmed switch is a map lookup, not disk I/O"
        );
    }

    #[test]
    fn plural_selection_follows_the_language_family() {
        let catalogue = fixture();
        let ru = catalogue.load("ru");
        assert_eq!(ru.plural_rules(), PluralRules::Slavic);
        assert_eq!(ru.translate_plural("members", 1), "1 участник");
        assert_eq!(ru.translate_plural("members", 3), "3 участника");
        assert_eq!(ru.translate_plural("members", 5), "5 участников");
        assert_eq!(ru.translate_plural("members", 21), "21 участник");

        let en = catalogue.load("en");
        assert_eq!(en.translate_plural("members", 1), "1 member");
        assert_eq!(en.translate_plural("members", 5), "5 members");
    }

    #[test]
    fn plural_falls_back_to_other_when_the_category_is_absent() {
        let catalogue = fixture();
        let ru = catalogue.load("ru");
        assert_eq!(
            ru.translate_plural("guests", 3),
            "3 guests",
            "no guests_few anywhere, so _other from the fallback dictionary"
        );
        assert_eq!(
            ru.translate_plural("phantom", 3),
            "phantom_other",
            "a counted key nothing defines renders as the _other key itself"
        );
    }

    #[test]
    fn a_plural_category_present_only_in_a_fallback_is_used() {
        let catalogue = fixture();
        let br = catalogue.load("pt-BR");
        assert_eq!(br.translate_plural("members", 1), "1 membro");
        assert_eq!(br.translate_plural("members", 7), "7 members");
    }

    #[test]
    fn plural_arg_replaces_the_count_with_the_caller_value() {
        let catalogue = fixture();
        let en = catalogue.load("en");
        assert_eq!(
            en.translate_plural_arg("guests", 5, FormatArg::from("many")),
            "many guests"
        );
    }

    #[test]
    fn braced_plural_family_substitutes_the_count() {
        let catalogue = fixture();
        let ru = catalogue.load("ru");
        for (count, want) in [
            (1_i64, "1 комментарий"),
            (3, "3 комментария"),
            (7, "7 комментариев"),
        ] {
            assert_eq!(
                ru.translate_plural_named(
                    "chat_thread_title",
                    count,
                    &[("count", &count.to_string())]
                ),
                want
            );
        }
        assert_eq!(
            ru.translate_plural("chat_thread_title", 3),
            "{count} комментария",
            "the printf helper cannot substitute a braced value — this is why the reference bypasses it"
        );
    }

    #[test]
    fn printf_substitutes_positionally() {
        assert_eq!(
            format_printf("Hello, %@!", &[FormatArg::from("Ada")]),
            "Hello, Ada!"
        );
        assert_eq!(
            format_printf(
                "Exporting %d of %d…",
                &[FormatArg::Int(1), FormatArg::Int(9)]
            ),
            "Exporting 1 of 9…"
        );
    }

    #[test]
    fn printf_honours_explicit_indices() {
        assert_eq!(
            format_printf(
                "%1$@ added %2$@",
                &[FormatArg::from("Ada"), FormatArg::from("Bob")]
            ),
            "Ada added Bob"
        );
        assert_eq!(
            format_printf(
                "%2$@ was added by %1$@",
                &[FormatArg::from("Ada"), FormatArg::from("Bob")]
            ),
            "Bob was added by Ada"
        );
        assert_eq!(
            format_printf("%1$d h %2$d min", &[FormatArg::Int(2), FormatArg::Int(5)]),
            "2 h 5 min"
        );
    }

    #[test]
    fn printf_emits_a_literal_percent() {
        assert_eq!(
            format_printf("Uploading · %d%%", &[FormatArg::Int(42)]),
            "Uploading · 42%"
        );
        assert_eq!(format_printf("100%%", &[]), "100%");
    }

    #[test]
    fn printf_honours_precision() {
        assert_eq!(format_printf("%.1fM", &[FormatArg::Float(1.25)]), "1.2M");
        assert_eq!(format_printf("%.0fK", &[FormatArg::Float(12.6)]), "13K");
    }

    #[test]
    fn printf_pads_to_a_width() {
        assert_eq!(format_printf("[%5d]", &[FormatArg::Int(42)]), "[   42]");
        assert_eq!(format_printf("[%-5d]", &[FormatArg::Int(42)]), "[42   ]");
        assert_eq!(format_printf("[%05d]", &[FormatArg::Int(42)]), "[00042]");
    }

    /// Before the cap, `%1000000d` returned a one-million-char String and `%.1000000f` panicked.
    #[test]
    fn printf_degrades_on_a_width_no_string_could_need() {
        assert_eq!(
            format_printf("%1000000d", &[FormatArg::Int(1)]),
            "%1000000d"
        );
        assert_eq!(
            format_printf("%2000000000d", &[FormatArg::Int(1)]),
            "%2000000000d"
        );
        assert_eq!(
            format_printf("%.1000000f", &[FormatArg::Float(1.0)]),
            "%.1000000f"
        );

        let at_cap = format!("%{MAX_FIELD_WIDTH}d");
        assert_eq!(
            format_printf(&at_cap, &[FormatArg::Int(1)]).chars().count(),
            MAX_FIELD_WIDTH
        );

        let over_cap = format!("%{}d", MAX_FIELD_WIDTH + 1);
        assert_eq!(format_printf(&over_cap, &[FormatArg::Int(1)]), over_cap);
    }

    #[test]
    fn printf_degrades_on_too_few_arguments() {
        assert_eq!(
            format_printf("%@ and %@", &[FormatArg::from("Ada")]),
            "Ada and %@"
        );
        assert_eq!(format_printf("%1$@ added %2$@", &[]), "%1$@ added %2$@");
        assert_eq!(format_printf("%@", &[]), "%@");
    }

    #[test]
    fn printf_ignores_extra_arguments() {
        assert_eq!(
            format_printf("Hello, %@!", &[FormatArg::from("Ada"), FormatArg::Int(7)]),
            "Hello, Ada!"
        );
    }

    #[test]
    fn printf_leaves_a_malformed_specifier_alone() {
        assert_eq!(format_printf("50% off", &[]), "50% off");
        assert_eq!(format_printf("trailing %", &[]), "trailing %");
        assert_eq!(format_printf("%z is not a thing", &[]), "%z is not a thing");
        assert_eq!(
            format_printf("%99999999999999999999d", &[FormatArg::Int(7)]),
            "%99999999999999999999d"
        );
        assert_eq!(
            format_printf("%0$@ is one-based", &[FormatArg::from("Ada")]),
            "%0$@ is one-based"
        );
    }

    #[test]
    fn printf_survives_multibyte_copy() {
        assert_eq!(
            format_printf(
                "%@さんが「%@」を追加",
                &[FormatArg::from("あ"), FormatArg::from("い")]
            ),
            "あさんが「い」を追加"
        );
    }

    #[test]
    fn named_substitutes_and_leaves_the_unknown_intact() {
        assert_eq!(
            format_named("Delete the chat with {name}?", &[("name", "Ada")]),
            "Delete the chat with Ada?"
        );
        assert_eq!(
            format_named("Use /bot<TOKEN>/{method} now", &[("name", "Ada")]),
            "Use /bot<TOKEN>/{method} now"
        );
        assert_eq!(format_named("{} and {", &[("", "x")]), "{} and {");
    }

    #[test]
    fn named_substitutes_every_occurrence_once() {
        assert_eq!(
            format_named("{name} met {name}", &[("name", "Ada")]),
            "Ada met Ada"
        );
        assert_eq!(
            format_named("{a}{b}", &[("a", "{b}"), ("b", "!")]),
            "{b}!",
            "a substituted value is never re-scanned"
        );
    }

    #[test]
    fn named_translation_reads_the_catalogue() {
        let catalogue = fixture();
        let en = catalogue.load("en");
        assert_eq!(
            en.translate_named("named", &[("name", "Ada")]),
            "Delete the chat with Ada permanently?"
        );
        assert_eq!(
            en.translate_named("doc", &[]),
            "Use /bot<TOKEN>/{method} with the Bot API."
        );
    }

    #[test]
    fn the_two_dialects_do_not_touch_each_others_syntax() {
        assert_eq!(
            format_printf("{count} of %d", &[FormatArg::Int(7)]),
            "{count} of 7",
            "printf leaves braces alone"
        );
        assert_eq!(
            format_named("%d of {count}", &[("count", "7")]),
            "%d of 7",
            "the braced formatter leaves specifiers alone"
        );
    }

    #[test]
    fn the_directory_source_reads_prefixed_files() {
        let root = std::env::temp_dir().join(format!(
            "rise-i18n-{}-{}",
            std::process::id(),
            "directory-source"
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("fixture directory");
        fs::write(root.join("locale-en.json"), r#"{"hello":"Hello"}"#).expect("fixture locale");
        fs::write(root.join("locale-ru.json"), "not json").expect("broken locale");

        let source = DirectoryLocaleSource::new(&root);
        assert_eq!(
            source
                .path_for("pt-BR")
                .file_name()
                .and_then(|n| n.to_str()),
            Some("locale-pt-BR.json")
        );

        let catalogue = Catalogue::directory(&root);
        assert_eq!(catalogue.load("en").translate("hello"), "Hello");
        assert_eq!(
            catalogue.load("ru").translate("hello"),
            "Hello",
            "an unparseable locale is empty, and the chain still answers"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// The process-wide runtime is global state; one test owns it so the suite cannot race.
    #[test]
    fn the_process_runtime_installs_switches_and_prewarms() {
        assert_eq!(tr("hello"), "hello", "no source installed yet");

        install(Arc::new(MapSource::new(&[
            ("en", &[("hello", "Hello"), ("count", "%d left")]),
            ("ru", &[("hello", "Привет")]),
        ])));
        assert_eq!(shared().code(), "en");
        assert_eq!(tr("hello"), "Hello");

        prewarm("ru");
        reload("ru");
        assert_eq!(shared().code(), "ru");
        assert_eq!(tr("hello"), "Привет");
        assert_eq!(
            tr("count"),
            "%d left",
            "russian is missing it, english answers"
        );
        assert_eq!(tr_args("count", &[FormatArg::Int(3)]), "3 left");
        assert_eq!(tr_named("hello", &[("name", "Ada")]), "Привет");

        reload("en");
        assert_eq!(shared().code(), "en");
    }
}
