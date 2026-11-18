use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_uchar, c_uint, c_void};

use cosmic_text::{Fallback, PlatformFallback};
use unicode_script::Script;

const RTLD_LAZY: c_int = 0x0001;
const FC_MATCH_PATTERN: c_int = 0;
const FC_RESULT_MATCH: c_int = 0;
const FC_LANG_EQUAL: c_int = 0;
const FC_FAMILY: &CStr = c"family";
const FC_CHARSET: &CStr = c"charset";
const FC_LANG: &CStr = c"lang";
const EMOJI: char = '\u{1F600}';

const SCRIPTS: [(Script, char, Option<&str>); 28] = [
    (Script::Hiragana, '\u{3042}', Some("ja")),
    (Script::Katakana, '\u{30A2}', Some("ja")),
    (Script::Hangul, '\u{D55C}', Some("ko")),
    (Script::Bopomofo, '\u{3105}', Some("zh-tw")),
    (Script::Arabic, '\u{0628}', Some("ar")),
    (Script::Hebrew, '\u{05D0}', Some("he")),
    (Script::Syriac, '\u{0710}', Some("syr")),
    (Script::Thaana, '\u{0780}', Some("dv")),
    (Script::Thai, '\u{0E01}', Some("th")),
    (Script::Lao, '\u{0E81}', Some("lo")),
    (Script::Khmer, '\u{1780}', Some("km")),
    (Script::Myanmar, '\u{1000}', Some("my")),
    (Script::Devanagari, '\u{0915}', Some("hi")),
    (Script::Bengali, '\u{0995}', Some("bn")),
    (Script::Gurmukhi, '\u{0A15}', Some("pa")),
    (Script::Gujarati, '\u{0A95}', Some("gu")),
    (Script::Oriya, '\u{0B15}', Some("or")),
    (Script::Tamil, '\u{0B95}', Some("ta")),
    (Script::Telugu, '\u{0C15}', Some("te")),
    (Script::Kannada, '\u{0C95}', Some("kn")),
    (Script::Malayalam, '\u{0D15}', Some("ml")),
    (Script::Sinhala, '\u{0D9A}', Some("si")),
    (Script::Tibetan, '\u{0F40}', Some("bo")),
    (Script::Georgian, '\u{10D0}', Some("ka")),
    (Script::Armenian, '\u{0561}', Some("hy")),
    (Script::Ethiopic, '\u{1200}', Some("am")),
    (Script::Mongolian, '\u{1820}', Some("mn-cn")),
    (Script::Cherokee, '\u{13A0}', Some("chr")),
];

#[repr(C)]
struct FcFontSet {
    nfont: c_int,
    _sfont: c_int,
    fonts: *mut *mut c_void,
}

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

struct Fontconfig {
    name_parse: unsafe extern "C" fn(*const c_uchar) -> *mut c_void,
    config_substitute: unsafe extern "C" fn(*mut c_void, *mut c_void, c_int) -> c_int,
    default_substitute: unsafe extern "C" fn(*mut c_void),
    font_sort: unsafe extern "C" fn(*mut c_void, *mut c_void, c_int, *mut *mut c_void, *mut c_int) -> *mut FcFontSet,
    get_string: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut *mut c_uchar) -> c_int,
    get_pointer: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut *mut c_void) -> c_int,
    get_lang_set: unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut *mut c_void) -> c_int,
    has_char: unsafe extern "C" fn(*const c_void, c_uint) -> c_int,
    has_lang: unsafe extern "C" fn(*const c_void, *const c_uchar) -> c_int,
    set_destroy: unsafe extern "C" fn(*mut FcFontSet),
    pattern_destroy: unsafe extern "C" fn(*mut c_void),
}

impl Fontconfig {
    fn open() -> Option<Self> {
        let handle = unsafe { dlopen(c"libfontconfig.so.1".as_ptr(), RTLD_LAZY) };
        if handle.is_null() {
            return None;
        }
        let symbol = |name: &CStr| {
            let address = unsafe { dlsym(handle, name.as_ptr()) };
            (!address.is_null()).then_some(address)
        };
        unsafe {
            Some(Self {
                name_parse: std::mem::transmute(symbol(c"FcNameParse")?),
                config_substitute: std::mem::transmute(symbol(c"FcConfigSubstitute")?),
                default_substitute: std::mem::transmute(symbol(c"FcDefaultSubstitute")?),
                font_sort: std::mem::transmute(symbol(c"FcFontSort")?),
                get_string: std::mem::transmute(symbol(c"FcPatternGetString")?),
                get_pointer: std::mem::transmute(symbol(c"FcPatternGetCharSet")?),
                get_lang_set: std::mem::transmute(symbol(c"FcPatternGetLangSet")?),
                has_char: std::mem::transmute(symbol(c"FcCharSetHasChar")?),
                has_lang: std::mem::transmute(symbol(c"FcLangSetHasLang")?),
                set_destroy: std::mem::transmute(symbol(c"FcFontSetDestroy")?),
                pattern_destroy: std::mem::transmute(symbol(c"FcPatternDestroy")?),
            })
        }
    }

    fn sorted(&self, name: &CStr) -> Vec<SortedFont> {
        let mut fonts = Vec::new();
        unsafe {
            let pattern = (self.name_parse)(name.as_ptr().cast());
            if pattern.is_null() {
                return fonts;
            }
            (self.config_substitute)(std::ptr::null_mut(), pattern, FC_MATCH_PATTERN);
            (self.default_substitute)(pattern);
            let mut result = FC_RESULT_MATCH;
            let set = (self.font_sort)(std::ptr::null_mut(), pattern, 0, std::ptr::null_mut(), &mut result);
            (self.pattern_destroy)(pattern);
            if set.is_null() {
                return fonts;
            }
            let count = usize::try_from((*set).nfont).unwrap_or(0);
            for index in 0..count {
                let font = *(*set).fonts.add(index);
                if font.is_null() {
                    continue;
                }
                let mut family = std::ptr::null_mut();
                if (self.get_string)(font, FC_FAMILY.as_ptr(), 0, &mut family) != FC_RESULT_MATCH || family.is_null() {
                    continue;
                }
                let Ok(family) = CStr::from_ptr(family.cast()).to_str() else {
                    continue;
                };
                let mut charset = std::ptr::null_mut();
                if (self.get_pointer)(font, FC_CHARSET.as_ptr(), 0, &mut charset) != FC_RESULT_MATCH {
                    charset = std::ptr::null_mut();
                }
                let mut langs = std::ptr::null_mut();
                if (self.get_lang_set)(font, FC_LANG.as_ptr(), 0, &mut langs) != FC_RESULT_MATCH {
                    langs = std::ptr::null_mut();
                }
                let covers = |ch: char| !charset.is_null() && (self.has_char)(charset, ch as c_uint) != 0;
                let speaks = |lang: &str| {
                    let Ok(lang) = std::ffi::CString::new(lang) else {
                        return false;
                    };
                    !langs.is_null() && (self.has_lang)(langs, lang.as_ptr().cast()) == FC_LANG_EQUAL
                };
                let mut sorted = SortedFont { family: family.to_owned(), covered: Vec::new(), spoken: Vec::new() };
                for (_, sample, lang) in SCRIPTS.iter().chain([(Script::Han, '\u{4E2D}', None)].iter()) {
                    sorted.covered.push(covers(*sample));
                    sorted.spoken.push(lang.is_some_and(|lang| speaks(lang)));
                }
                sorted.covered.push(covers(EMOJI));
                for lang in HAN_LANGS {
                    sorted.spoken.push(speaks(lang));
                }
                fonts.push(sorted);
            }
            (self.set_destroy)(set);
        }
        fonts
    }
}

const HAN_LANGS: [&str; 5] = ["ja", "ko", "zh-tw", "zh-hk", "zh-cn"];
const HAN_FAMILIES: [&str; 5] =
    ["Noto Sans CJK JP", "Noto Sans CJK KR", "Noto Sans CJK TC", "Noto Sans CJK HK", "Noto Sans CJK SC"];

struct SortedFont {
    family: String,
    covered: Vec<bool>,
    spoken: Vec<bool>,
}

fn han_lang(locale: &str) -> Option<usize> {
    let locale = locale.to_ascii_lowercase().replace('_', "-");
    let lang = if locale.starts_with("ja") {
        "ja"
    } else if locale.starts_with("ko") {
        "ko"
    } else if locale.starts_with("zh-tw") || locale.starts_with("zh-hant") {
        "zh-tw"
    } else if locale.starts_with("zh-hk") || locale.starts_with("zh-mo") {
        "zh-hk"
    } else if locale.starts_with("zh") {
        "zh-cn"
    } else {
        return None;
    };
    HAN_LANGS.iter().position(|known| *known == lang)
}

fn leak(family: &str) -> &'static str {
    Box::leak(family.to_owned().into_boxed_str())
}

/// fallback families fontconfig prefers for each script, then the cosmic text lists
pub(crate) struct FontconfigFallback {
    platform: PlatformFallback,
    common: Vec<&'static str>,
    scripts: HashMap<Script, Vec<&'static str>>,
}

impl FontconfigFallback {
    pub(crate) fn new(locale: &str) -> Option<Self> {
        let fontconfig = Fontconfig::open()?;
        let sans = fontconfig.sorted(c"sans-serif");
        if sans.is_empty() {
            return None;
        }
        let platform = PlatformFallback;
        let mut scripts = HashMap::default();
        let pick = |index: usize, lang: Option<usize>| {
            lang.and_then(|lang| sans.iter().find(|font| font.covered[index] && font.spoken[lang]))
                .or_else(|| sans.iter().find(|font| font.covered[index]))
                .map(|font| font.family.as_str())
        };
        for (index, (script, _, lang)) in SCRIPTS.iter().enumerate() {
            let spoken = lang.map(|_| index);
            let Some(family) = pick(index, spoken) else {
                continue;
            };
            let mut list = vec![leak(family)];
            list.extend(platform.script_fallback(*script, locale).iter().filter(|known| **known != family));
            scripts.insert(*script, list);
        }
        let han = SCRIPTS.len();
        let han_locale = han_lang(locale);
        let mut list: Vec<&'static str> = Vec::new();
        if let Some(family) = pick(han, han_locale.map(|lang| han + 1 + lang)) {
            list.push(leak(family));
        }
        if let Some(lang) = han_locale {
            list.push(HAN_FAMILIES[lang]);
        }
        for family in platform.script_fallback(Script::Han, locale).iter().chain(HAN_FAMILIES.iter()) {
            if !list.contains(family) {
                list.push(*family);
            }
        }
        scripts.insert(Script::Han, list);
        let emoji_index = han + 1;
        let emoji = fontconfig.sorted(c"emoji");
        let mut common = Vec::new();
        if let Some(font) = emoji.iter().chain(sans.iter()).find(|font| font.covered[emoji_index]) {
            common.push(leak(&font.family));
        }
        for family in platform.common_fallback() {
            if !common.contains(family) {
                common.push(*family);
            }
        }
        Some(Self { platform, common, scripts })
    }
}

impl Fallback for FontconfigFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        self.platform.forbidden_fallback()
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        match self.scripts.get(&script) {
            Some(list) => list.as_slice(),
            None => self.platform.script_fallback(script, locale),
        }
    }
}
