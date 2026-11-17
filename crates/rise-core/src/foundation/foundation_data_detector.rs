use std::collections::HashSet;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::foundation::foundation_regex::{NSMatchingOptions, NSRegularExpressionError};
use crate::foundation::foundation_url::Url;
use crate::foundation::ns_range::{byte_to_utf16_range, utf16_to_byte_range, NSRange};

/// bit set of text checking types
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NSTextCheckingType(pub u64);

impl NSTextCheckingType {
    pub const LINK: NSTextCheckingType = NSTextCheckingType(1 << 5);

    pub const fn raw_value(&self) -> u64 {
        self.0
    }

    pub const fn contains(&self, other: NSTextCheckingType) -> bool {
        self.0 & other.0 == other.0
    }
}

/// one match found by the detector
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NSDataDetectorResult {
    pub result_type: NSTextCheckingType,
    pub range: NSRange,
    pub url: Option<Url>,
}

/// finds links in text
#[derive(Debug)]
pub struct NSDataDetector {
    checking_types: NSTextCheckingType,
    scheme_link: Regex,
    mailto_link: Regex,
    email: Regex,
    host_link: Regex,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LinkKind {
    Scheme,
    Mailto,
    Email,
    Host,
}

struct Candidate {
    bytes: Range<usize>,
    url: Url,
}

const LABEL: &str = r"[\p{L}\p{N}](?:[\p{L}\p{N}-]{0,61}[\p{L}\p{N}])?";
const TOP_LEVEL: &str = r"(?:xn--[A-Za-z0-9-]{1,59}|\p{L}{2,63})";
const BODY: &str = r#"[^\s<>"\x{AB}\x{BB}]"#;

impl NSDataDetector {
    /// makes a detector, only the link type works here and anything else fails
    pub fn new(types: NSTextCheckingType) -> Result<NSDataDetector, NSRegularExpressionError> {
        if types != NSTextCheckingType::LINK {
            return Err(NSRegularExpressionError(format!("unsupported checking types {:#x}", types.0)));
        }
        let host = format!(r"(?:{LABEL}\.)+{TOP_LEVEL}");
        Ok(NSDataDetector {
            checking_types: types,
            scheme_link: build(&format!(r"[A-Za-z][A-Za-z0-9+.-]*://{BODY}+"))?,
            mailto_link: build(&format!(r"(?i:mailto):{BODY}+"))?,
            email: build(&format!(r"[\p{{L}}\p{{N}}_%+-][\p{{L}}\p{{N}}._%+-]*@{host}"))?,
            host_link: build(&format!(r"{host}(?::[0-9]{{1,5}})?(?:[/?#]{BODY}*)?"))?,
        })
    }

    /// the types this detector looks for
    pub fn checking_types(&self) -> NSTextCheckingType {
        self.checking_types
    }

    /// all matches in range, bad range (out of bounds or half a surrogate pair) gives none
    /// with anchored each match must start where the last one ended
    pub fn matches(&self, text: &str, options: NSMatchingOptions, range: NSRange) -> Vec<NSDataDetectorResult> {
        let Some(bytes) = utf16_to_byte_range(text, range) else { return Vec::new() };
        let slice = &text[bytes.clone()];
        let mut out = Vec::new();
        let mut expected_start = 0usize;
        for candidate in self.candidates(slice) {
            if options.anchored && candidate.bytes.start != expected_start {
                break;
            }
            expected_start = candidate.bytes.end;
            let absolute = candidate.bytes.start + bytes.start..candidate.bytes.end + bytes.start;
            out.push(NSDataDetectorResult {
                result_type: NSTextCheckingType::LINK,
                range: byte_to_utf16_range(text, absolute),
                url: Some(candidate.url),
            });
        }
        out
    }

    pub fn first_match(&self, text: &str, options: NSMatchingOptions, range: NSRange) -> Option<NSDataDetectorResult> {
        self.matches(text, options, range).into_iter().next()
    }

    pub fn number_of_matches(&self, text: &str, options: NSMatchingOptions, range: NSRange) -> usize {
        self.matches(text, options, range).len()
    }

    /// every link in slice in order, no overlaps
    /// byte ranges are relative to slice
    fn candidates(&self, slice: &str) -> Vec<Candidate> {
        let mut found: Vec<Candidate> = Vec::new();
        for (regex, kind) in [
            (&self.scheme_link, LinkKind::Scheme),
            (&self.mailto_link, LinkKind::Mailto),
            (&self.email, LinkKind::Email),
            (&self.host_link, LinkKind::Host),
        ] {
            for m in regex.find_iter(slice) {
                if let Some(candidate) = accept(slice, m.range(), kind) {
                    found.push(candidate);
                }
            }
        }
        found.sort_by(|a, b| a.bytes.start.cmp(&b.bytes.start).then(b.bytes.end.cmp(&a.bytes.end)));
        let mut kept: Vec<Candidate> = Vec::with_capacity(found.len());
        for candidate in found {
            if kept.last().is_some_and(|last| candidate.bytes.start < last.bytes.end) {
                continue;
            }
            kept.push(candidate);
        }
        kept
    }
}

fn build(pattern: &str) -> Result<Regex, NSRegularExpressionError> {
    Regex::new(pattern).map_err(|e| NSRegularExpressionError(e.to_string()))
}

/// checks boundary and trailing punctuation and tld rules on one raw match
fn accept(slice: &str, raw: Range<usize>, kind: LinkKind) -> Option<Candidate> {
    let before = slice[..raw.start].chars().next_back();
    let blocked_before = match kind {
        LinkKind::Scheme | LinkKind::Mailto => before.is_some_and(char::is_alphanumeric),
        LinkKind::Email => false,
        LinkKind::Host => before.is_some_and(|c| c.is_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | '/' | ':')),
    };
    if blocked_before {
        return None;
    }
    let end = raw.start + trimmed_link_len(&slice[raw.clone()]);
    let text = &slice[raw.start..end];
    let url = match kind {
        LinkKind::Scheme => {
            let after_scheme = text.find("://").map(|i| &text[i + 3..])?;
            if after_scheme.is_empty() {
                return None;
            }
            Url::from_string(text)?
        }
        LinkKind::Mailto => {
            let address = &text[text.find(':')? + 1..];
            if !address.contains('@') {
                return None;
            }
            Url::from_string(text)?
        }
        LinkKind::Email => {
            let domain = &text[text.rfind('@')? + 1..];
            if !is_known_top_level(top_level_of(domain)) {
                return None;
            }
            Url::from_string(&format!("mailto:{text}"))?
        }
        LinkKind::Host => {
            let host_end = text.find([':', '/', '?', '#']).unwrap_or(text.len());
            let host = &text[..host_end];
            let www = host.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("www."));
            if !www && !is_known_top_level(top_level_of(host)) {
                return None;
            }
            Url::from_string(&format!("http://{text}"))?
        }
    };
    Some(Candidate { bytes: raw.start..end, url })
}

fn trimmed_link_len(link: &str) -> usize {
    let mut end = link.len();
    while let Some(last) = link[..end].chars().next_back() {
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | '\u{2019}' | '\u{201D}' | '\u{2026}' => true,
            ')' => unbalanced(&link[..end], '(', ')'),
            ']' => unbalanced(&link[..end], '[', ']'),
            '}' => unbalanced(&link[..end], '{', '}'),
            _ => false,
        };
        if !drop {
            break;
        }
        end -= last.len_utf8();
    }
    end
}

fn unbalanced(text: &str, open: char, close: char) -> bool {
    text.chars().filter(|&c| c == close).count() > text.chars().filter(|&c| c == open).count()
}

fn top_level_of(host: &str) -> &str {
    host.rsplit('.').next().unwrap_or(host)
}

fn is_known_top_level(top_level: &str) -> bool {
    let lowered = top_level.to_lowercase();
    lowered.starts_with("xn--") || KNOWN_TOP_LEVEL_DOMAINS.contains(lowered.as_str())
}

static KNOWN_TOP_LEVEL_DOMAINS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    const GENERIC: &str = "com net org edu gov mil int info biz name pro aero coop museum mobi asia tel travel jobs cat \
        app dev page blog shop store online site website tech xyz club top live news space cloud art design media \
        digital agency email link click studio world today network group global center solutions company fun games \
        game chat social team life love moscow";
    const COUNTRY: &str = "ac ad ae af ag ai al am ao aq ar as at au aw ax az ba bb bd be bf bg bh bi bj bm bn bo br bs \
        bt bw by bz ca cc cd cf cg ch ci ck cl cm cn co cr cu cv cw cx cy cz de dj dk dm do dz ec ee eg er es et eu fi \
        fj fk fm fo fr ga gd ge gf gg gh gi gl gm gn gp gq gr gs gt gu gw gy hk hm hn hr ht hu id ie il im in io iq ir \
        is it je jm jo jp ke kg kh ki km kn kp kr kw ky kz la lb lc li lk lr ls lt lu lv ly ma mc md me mg mh mk ml mm \
        mn mo mp mq mr ms mt mu mv mw mx my mz na nc ne nf ng ni nl no np nr nu nz om pa pe pf pg ph pk pl pm pn pr ps \
        pt pw py qa re ro rs ru rw sa sb sc sd se sg sh si sk sl sm sn so sr ss st su sv sx sy sz tc td tf tg th tj tk \
        tl tm tn to tr tt tv tw tz ua ug uk us uy uz va vc ve vg vi vn vu wf ws ye yt za zm zw";
    const CYRILLIC_RF: &str = "\u{0440}\u{0444}";
    GENERIC.split_whitespace().chain(COUNTRY.split_whitespace()).chain([CYRILLIC_RF]).collect()
});

pub static LINK_DETECTOR: LazyLock<Option<NSDataDetector>> =
    LazyLock::new(|| NSDataDetector::new(NSTextCheckingType::LINK).ok());
