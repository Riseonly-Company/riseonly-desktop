use std::ops::Range;
use std::sync::LazyLock;

use regex::{Captures, Regex, RegexBuilder};

use crate::foundation::ns_range::{byte_to_utf16_range, utf16_to_byte_range, NSRange};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NSRegularExpressionOptions {
    pub case_insensitive: bool,
    pub allow_comments_and_whitespace: bool,
    pub ignore_metacharacters: bool,
    pub dot_matches_line_separators: bool,
    pub anchors_match_lines: bool,
    pub use_unix_line_separators: bool,
}

impl NSRegularExpressionOptions {
    pub const NONE: NSRegularExpressionOptions = NSRegularExpressionOptions {
        case_insensitive: false,
        allow_comments_and_whitespace: false,
        ignore_metacharacters: false,
        dot_matches_line_separators: false,
        anchors_match_lines: false,
        use_unix_line_separators: false,
    };
    pub const CASE_INSENSITIVE: NSRegularExpressionOptions =
        NSRegularExpressionOptions { case_insensitive: true, ..Self::NONE };
    pub const DOT_MATCHES_LINE_SEPARATORS: NSRegularExpressionOptions =
        NSRegularExpressionOptions { dot_matches_line_separators: true, ..Self::NONE };
    pub const ANCHORS_MATCH_LINES: NSRegularExpressionOptions =
        NSRegularExpressionOptions { anchors_match_lines: true, ..Self::NONE };
}

/// regex matching flags
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NSMatchingOptions {
    pub anchored: bool,
}

impl NSMatchingOptions {
    pub const NONE: NSMatchingOptions = NSMatchingOptions { anchored: false };
    pub const ANCHORED: NSMatchingOptions = NSMatchingOptions { anchored: true };
}

/// why a pattern didnt compile
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid regular expression pattern: {0}")]
pub struct NSRegularExpressionError(pub String);

/// one regex match result
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NSTextCheckingResult {
    ranges: Vec<Option<NSRange>>,
}

impl NSTextCheckingResult {
    /// range of the whole match
    pub fn range(&self) -> NSRange {
        self.ranges.first().copied().flatten().unwrap_or_default()
    }

    /// range of a capture group, none if it didnt take part
    pub fn range_at(&self, index: usize) -> Option<NSRange> {
        self.ranges.get(index).copied().flatten()
    }

    /// number of ranges, whole match plus one per group
    pub fn number_of_ranges(&self) -> usize {
        self.ranges.len()
    }
}

/// compiled regex
#[derive(Debug)]
pub struct NSRegularExpression {
    pattern: String,
    options: NSRegularExpressionOptions,
    regex: Regex,
}

impl NSRegularExpression {
    pub fn new(pattern: &str, options: NSRegularExpressionOptions) -> Result<Self, NSRegularExpressionError> {
        let regex = Self::build(pattern, options)?;
        Ok(Self { pattern: pattern.to_owned(), options, regex })
    }

    /// compiles a pattern with no options
    pub fn with_pattern(pattern: &str) -> Result<Self, NSRegularExpressionError> {
        Self::new(pattern, NSRegularExpressionOptions::NONE)
    }

    fn build(pattern: &str, options: NSRegularExpressionOptions) -> Result<Regex, NSRegularExpressionError> {
        let source = if options.ignore_metacharacters { regex::escape(pattern) } else { pattern.to_owned() };
        RegexBuilder::new(&source)
            .case_insensitive(options.case_insensitive)
            .ignore_whitespace(options.allow_comments_and_whitespace)
            .dot_matches_new_line(options.dot_matches_line_separators)
            .multi_line(options.anchors_match_lines)
            .crlf(!options.use_unix_line_separators)
            .build()
            .map_err(|e| NSRegularExpressionError(e.to_string()))
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn options(&self) -> NSRegularExpressionOptions {
        self.options
    }

    pub fn number_of_capture_groups(&self) -> usize {
        self.regex.captures_len() - 1
    }

    fn result(text: &str, offset: usize, caps: &Captures<'_>) -> NSTextCheckingResult {
        let ranges = (0..caps.len())
            .map(|i| caps.get(i).map(|m| byte_to_utf16_range(text, m.start() + offset..m.end() + offset)))
            .collect();
        NSTextCheckingResult { ranges }
    }

    /// all matches in range, bad range (out of bounds or half a surrogate pair) gives none
    pub fn matches(&self, text: &str, options: NSMatchingOptions, range: NSRange) -> Vec<NSTextCheckingResult> {
        let Some(bytes) = utf16_to_byte_range(text, range) else { return Vec::new() };
        let slice = &text[bytes.clone()];
        let mut out = Vec::new();
        let mut expected_start = 0usize;
        for caps in self.regex.captures_iter(slice) {
            let Some(whole) = caps.get(0) else { continue };
            if options.anchored && whole.start() != expected_start {
                break;
            }
            expected_start = whole.end();
            out.push(Self::result(text, bytes.start, &caps));
        }
        out
    }

    pub fn first_match(&self, text: &str, options: NSMatchingOptions, range: NSRange) -> Option<NSTextCheckingResult> {
        let bytes = utf16_to_byte_range(text, range)?;
        let caps = self.regex.captures(&text[bytes.clone()])?;
        let whole = caps.get(0)?;
        if options.anchored && whole.start() != 0 {
            return None;
        }
        Some(Self::result(text, bytes.start, &caps))
    }

    pub fn range_of_first_match(&self, text: &str, options: NSMatchingOptions, range: NSRange) -> Option<NSRange> {
        let bytes = utf16_to_byte_range(text, range)?;
        let m = self.regex.find(&text[bytes.clone()])?;
        if options.anchored && m.start() != 0 {
            return None;
        }
        Some(byte_to_utf16_range(text, m.start() + bytes.start..m.end() + bytes.start))
    }

    pub fn number_of_matches(&self, text: &str, options: NSMatchingOptions, range: NSRange) -> usize {
        self.matches(text, options, range).len()
    }

    /// replaces matches with a template, $0 to $n take groups
    /// the longest number that names a group wins and a backslash escapes the next char
    pub fn string_by_replacing_matches(&self, text: &str, options: NSMatchingOptions, range: NSRange, template: &str) -> String {
        let Some(bytes) = utf16_to_byte_range(text, range) else { return text.to_owned() };
        let slice = &text[bytes.clone()];
        let groups = self.regex.captures_len();
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..bytes.start]);
        let mut last = 0usize;
        for caps in self.regex.captures_iter(slice) {
            let Some(whole) = caps.get(0) else { continue };
            if options.anchored && whole.start() != last {
                break;
            }
            out.push_str(&slice[last..whole.start()]);
            expand_template(template, &caps, groups, &mut out);
            last = whole.end();
        }
        out.push_str(&slice[last..]);
        out.push_str(&text[bytes.end..]);
        out
    }

    pub fn escaped_pattern(for_string: &str) -> String {
        regex::escape(for_string)
    }

    pub fn escaped_template(for_string: &str) -> String {
        let mut out = String::with_capacity(for_string.len());
        for c in for_string.chars() {
            if c == '$' || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }
}

fn expand_template(template: &str, caps: &Captures<'_>, groups: usize, out: &mut String) {
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '$' => {
                let mut group: Option<usize> = None;
                while let Some(&d) = chars.peek() {
                    let Some(v) = d.to_digit(10) else { break };
                    let candidate = group.unwrap_or(0) * 10 + v as usize;
                    if group.is_some() && candidate >= groups {
                        break;
                    }
                    group = Some(candidate);
                    chars.next();
                }
                match group {
                    Some(g) => {
                        if let Some(m) = caps.get(g) {
                            out.push_str(m.as_str());
                        }
                    }
                    None => out.push('$'),
                }
            }
            _ => out.push(c),
        }
    }
}

fn compile_fixed(pattern: &str) -> Option<NSRegularExpression> {
    NSRegularExpression::with_pattern(pattern).ok()
}

pub static MENTION: LazyLock<Option<NSRegularExpression>> = LazyLock::new(|| {
    compile_fixed(&format!(r"(?:^|[^\w@])@([A-Za-z0-9_]{{1,{}}})", rise_validate::rule_max::TAG))
});

pub static HEX_32_TO_128: LazyLock<Option<NSRegularExpression>> =
    LazyLock::new(|| compile_fixed("^[a-fA-F0-9]{32,128}$"));

pub static UUID_IN_PATH: LazyLock<Option<NSRegularExpression>> = LazyLock::new(|| {
    compile_fixed("[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
});

pub static NUMBER_IN_PATH: LazyLock<Option<NSRegularExpression>> = LazyLock::new(|| compile_fixed(r"/\d{2,}"));

/// byte range of the first regex match over the whole string, none if no match
/// pattern must be a compiled one
pub fn range_of_regular_expression(text: &str, regex: &NSRegularExpression) -> Option<Range<usize>> {
    regex.regex.find(text).map(|m| m.range())
}

/// replaces every match over the whole string using the template (eg $1)
pub fn replacing_occurrences_regular_expression(text: &str, regex: &NSRegularExpression, template: &str) -> String {
    regex.string_by_replacing_matches(text, NSMatchingOptions::NONE, NSRange::full(text), template)
}

/// a % that doesnt start a percent escape becomes %25
/// lookahead reads the original text so in %%41 only the first % changes
pub fn escaping_stray_percent_signs(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'%' {
            continue;
        }
        let escape = matches!(bytes.get(i + 1..i + 3), Some([h, l]) if h.is_ascii_hexdigit() && l.is_ascii_hexdigit());
        if !escape {
            out.push_str(&text[last..=i]);
            out.push_str("25");
            last = i + 1;
        }
    }
    out.push_str(&text[last..]);
    out
}
