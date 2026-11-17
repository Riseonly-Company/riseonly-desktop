use std::sync::LazyLock;

use regex::Regex;

use crate::foundation::foundation_text::{characters as grapheme_slices, is_newline};

/// one character, a grapheme cluster slice of its string
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Character<'a>(&'a str);

/// iterates the characters of s as grapheme clusters
pub fn characters(s: &str) -> impl DoubleEndedIterator<Item = Character<'_>> + '_ {
    grapheme_slices(s).map(Character)
}

static GENERAL_PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"\p{P}"));
static GENERAL_SYMBOL: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"\p{S}"));
static GENERAL_CURRENCY: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"\p{Sc}"));
static GENERAL_NUMBER: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"\p{N}"));
static PROPERTY_MATH: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"\p{Math}"));
static PROPERTY_CASED: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"\p{Cased}"));
static PROPERTY_EMOJI: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"\p{Emoji}"));
static PROPERTY_EMOJI_PRESENTATION: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"\p{Emoji_Presentation}"));
static PROPERTY_EXTENDED_PICTOGRAPHIC: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"\p{Extended_Pictographic}"));
static GENERAL_CONTROL_OR_FORMAT: LazyLock<Regex> = LazyLock::new(|| unicode_class(r"[\p{Cc}\p{Cf}]"));

const HAN_NUMERIC: [u32; 91] = [
    0x3405, 0x3483, 0x382A, 0x3B4D, 0x4E00, 0x4E03, 0x4E07, 0x4E09, 0x4E24, 0x4E5D, 0x4E8C, 0x4E94,
    0x4E96, 0x4EAC, 0x4EBF, 0x4EC0, 0x4EDF, 0x4EE8, 0x4F0D, 0x4F70, 0x4FE9, 0x5006, 0x5104, 0x5146,
    0x5169, 0x516B, 0x516D, 0x5341, 0x5343, 0x5344, 0x5345, 0x534C, 0x53C1, 0x53C2, 0x53C3, 0x53C4,
    0x56DB, 0x58F1, 0x58F9, 0x5E7A, 0x5EFE, 0x5EFF, 0x5F0C, 0x5F0D, 0x5F0E, 0x5F10, 0x62D0, 0x62FE,
    0x634C, 0x67D2, 0x6D1E, 0x6F06, 0x7396, 0x767E, 0x7695, 0x79ED, 0x8086, 0x842C, 0x8CAE, 0x8CB3,
    0x8D30, 0x920E, 0x94A9, 0x9621, 0x9646, 0x964C, 0x9678, 0x96F6, 0xF96B, 0xF973, 0xF978, 0xF9B2,
    0xF9D1, 0xF9D3, 0xF9FD, 0x20001, 0x20064, 0x200E2, 0x20121, 0x2092A, 0x20983, 0x2098C, 0x2099C,
    0x20AEA, 0x20AFD, 0x20B19, 0x22390, 0x22998, 0x23B1B, 0x2626D, 0x2F890,
];

fn unicode_class(class: &str) -> Regex {
    Regex::new(&format!(r"\A{class}\z")).expect("static Unicode property class")
}

fn scalar_in(class: &Regex, c: char) -> bool {
    let mut buf = [0u8; 4];
    class.is_match(c.encode_utf8(&mut buf))
}

pub fn scalar_is_emoji(c: char) -> bool {
    scalar_in(&PROPERTY_EMOJI, c)
}

/// true if the scalar has the emoji presentation property
pub fn scalar_is_emoji_presentation(c: char) -> bool {
    scalar_in(&PROPERTY_EMOJI_PRESENTATION, c)
}

pub fn scalar_is_extended_pictographic(c: char) -> bool {
    scalar_in(&PROPERTY_EXTENDED_PICTOGRAPHIC, c)
}

pub fn scalar_is_alphabetic(c: char) -> bool {
    c.is_alphabetic()
}

pub fn scalar_is_white_space(c: char) -> bool {
    c.is_whitespace()
}

/// true for control characters, general category Cc or Cf
pub fn scalar_is_control_character(c: char) -> bool {
    scalar_in(&GENERAL_CONTROL_OR_FORMAT, c)
}

impl<'a> Character<'a> {
    /// character from a string holding exactly one, none otherwise
    pub fn new(s: &'a str) -> Option<Self> {
        let mut it = grapheme_slices(s);
        match (it.next(), it.next()) {
            (Some(one), None) => Some(Character(one)),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'a str {
        self.0
    }

    pub fn unicode_scalars(&self) -> std::str::Chars<'a> {
        self.0.chars()
    }

    fn first_scalar(&self) -> char {
        self.0.chars().next().unwrap_or('\0')
    }

    fn is_single_scalar(&self) -> bool {
        let mut it = self.0.chars();
        it.next().is_some() && it.next().is_none()
    }

    fn is_crlf(&self) -> bool {
        self.0 == "\r\n"
    }

    /// ascii value if any, crlf reads as lf
    pub fn ascii_value(&self) -> Option<u8> {
        if self.is_crlf() {
            return Some(b'\n');
        }
        let c = self.first_scalar();
        (self.is_single_scalar() && c.is_ascii()).then_some(c as u8)
    }

    pub fn is_ascii(&self) -> bool {
        self.ascii_value().is_some()
    }

    /// true if the first scalar has the White_Space property
    pub fn is_whitespace(&self) -> bool {
        scalar_is_white_space(self.first_scalar())
    }

    /// true if the first scalar is a newline char like lf or cr
    pub fn is_newline(&self) -> bool {
        is_newline(self.first_scalar())
    }

    /// true if the first scalar is numeric, category N or a han numeral
    pub fn is_number(&self) -> bool {
        let c = self.first_scalar();
        scalar_in(&GENERAL_NUMBER, c) || HAN_NUMERIC.binary_search(&u32::from(c)).is_ok()
    }

    /// true if the first scalar is alphabetic
    pub fn is_letter(&self) -> bool {
        scalar_is_alphabetic(self.first_scalar())
    }

    /// true if the first scalar is punctuation, category P
    pub fn is_punctuation(&self) -> bool {
        scalar_in(&GENERAL_PUNCTUATION, self.first_scalar())
    }

    pub fn is_symbol(&self) -> bool {
        scalar_in(&GENERAL_SYMBOL, self.first_scalar())
    }

    pub fn is_currency_symbol(&self) -> bool {
        scalar_in(&GENERAL_CURRENCY, self.first_scalar())
    }

    pub fn is_math_symbol(&self) -> bool {
        scalar_in(&PROPERTY_MATH, self.first_scalar())
    }

    fn is_uppercased(&self) -> bool {
        self.0.to_uppercase() == self.0
    }

    fn is_lowercased(&self) -> bool {
        self.0.to_lowercase() == self.0
    }

    pub fn is_cased(&self) -> bool {
        if self.is_single_scalar() && scalar_in(&PROPERTY_CASED, self.first_scalar()) {
            return true;
        }
        !self.is_uppercased() || !self.is_lowercased()
    }

    pub fn is_uppercase(&self) -> bool {
        if self.is_single_scalar() && self.first_scalar().is_uppercase() {
            return true;
        }
        self.is_uppercased() && self.is_cased()
    }

    pub fn is_lowercase(&self) -> bool {
        if self.is_single_scalar() && self.first_scalar().is_lowercase() {
            return true;
        }
        self.is_lowercased() && self.is_cased()
    }

    /// hex digit value of one scalar, ascii and fullwidth digits both work
    pub fn hex_digit_value(&self) -> Option<u32> {
        if !self.is_single_scalar() {
            return None;
        }
        let v = self.first_scalar() as u32;
        match v {
            0x30..=0x39 => Some(v - 0x30),
            0x41..=0x46 => Some(v - 0x41 + 10),
            0x61..=0x66 => Some(v - 0x61 + 10),
            0xFF10..=0xFF19 => Some(v - 0xFF10),
            0xFF21..=0xFF26 => Some(v - 0xFF21 + 10),
            0xFF41..=0xFF46 => Some(v - 0xFF41 + 10),
            _ => None,
        }
    }

    pub fn is_hex_digit(&self) -> bool {
        self.hex_digit_value().is_some()
    }

    /// uppercased copy of the char
    pub fn uppercased(&self) -> String {
        self.0.to_uppercase()
    }

    /// lowercased copy of the char
    pub fn lowercased(&self) -> String {
        self.0.to_lowercase()
    }
}

/// parses an int, optional sign and ascii digits only
/// none on overflow or spaces
pub fn int_from_string(s: &str) -> Option<i64> {
    int_from_string_radix(s, 10)
}

/// parses an int in radix 2 to 36, letters in any case
pub fn int_from_string_radix(s: &str, radix: u32) -> Option<i64> {
    if !(2..=36).contains(&radix) {
        return None;
    }
    i64::from_str_radix(s, radix).ok()
}

/// parses a double, also hex like 0x1.8p3 and inf or nan in any case
/// anything else (spaces too) is none
pub fn double_from_string(s: &str) -> Option<f64> {
    if s.is_empty() || s.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let (negative, body) = match s.as_bytes()[0] {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let lower = body.to_ascii_lowercase();
    let value = if let Some(hex) = lower.strip_prefix("0x") {
        hex_float(hex)?
    } else if lower.starts_with("nan(") && lower.ends_with(')') {
        let payload = &lower[4..lower.len() - 1];
        payload.chars().all(|c| c.is_ascii_alphanumeric() || c == '_').then_some(f64::NAN)?
    } else if body.starts_with(['+', '-']) {
        return None;
    } else {
        body.parse::<f64>().ok()?
    };
    Some(if negative { -value } else { value })
}

fn hex_float(s: &str) -> Option<f64> {
    let (mantissa, exponent) = match s.find('p') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (int_part, frac_part) = match mantissa.find('.') {
        Some(i) => (&mantissa[..i], &mantissa[i + 1..]),
        None => (mantissa, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    let mut value = 0f64;
    for c in int_part.chars() {
        value = value * 16.0 + f64::from(c.to_digit(16)?);
    }
    let mut scale = 1.0 / 16.0;
    for c in frac_part.chars() {
        value += f64::from(c.to_digit(16)?) * scale;
        scale /= 16.0;
    }
    let exp = match exponent {
        Some(e) => e.parse::<i32>().ok()?,
        None => 0,
    };
    Some(value * 2f64.powi(exp))
}
