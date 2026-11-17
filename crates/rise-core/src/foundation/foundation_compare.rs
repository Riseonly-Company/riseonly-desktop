use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::Range;

use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::{Collator, CollatorBorrowed};
use icu_locale_core::Locale as IcuLocale;
use unicode_normalization::char::{decompose_canonical, decompose_compatible, is_combining_mark};

use crate::foundation::foundation_text::character_boundaries;
use crate::foundation::ns_range::{byte_to_utf16_range, utf16_to_byte_range, NSRange};

/// compare flags that change how matching works
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CompareOptions {
    pub case_insensitive: bool,
    pub diacritic_insensitive: bool,
    pub width_insensitive: bool,
    pub literal: bool,
    pub anchored: bool,
    pub backwards: bool,
}

impl CompareOptions {
    pub const NONE: CompareOptions = CompareOptions {
        case_insensitive: false,
        diacritic_insensitive: false,
        width_insensitive: false,
        literal: false,
        anchored: false,
        backwards: false,
    };
    pub const CASE_INSENSITIVE: CompareOptions = CompareOptions { case_insensitive: true, ..Self::NONE };
    pub const DIACRITIC_INSENSITIVE: CompareOptions = CompareOptions { diacritic_insensitive: true, ..Self::NONE };
    pub const WIDTH_INSENSITIVE: CompareOptions = CompareOptions { width_insensitive: true, ..Self::NONE };
    pub const LITERAL: CompareOptions = CompareOptions { literal: true, ..Self::NONE };
    pub const ANCHORED: CompareOptions = CompareOptions { anchored: true, ..Self::NONE };
    pub const BACKWARDS: CompareOptions = CompareOptions { backwards: true, ..Self::NONE };

    pub const fn union(self, other: CompareOptions) -> CompareOptions {
        CompareOptions {
            case_insensitive: self.case_insensitive || other.case_insensitive,
            diacritic_insensitive: self.diacritic_insensitive || other.diacritic_insensitive,
            width_insensitive: self.width_insensitive || other.width_insensitive,
            literal: self.literal || other.literal,
            anchored: self.anchored || other.anchored,
            backwards: self.backwards || other.backwards,
        }
    }
}

fn push_folded(c: char, options: CompareOptions, mut out: impl FnMut(char)) {
    let mut emit = |d: char| {
        if options.case_insensitive {
            for l in d.to_lowercase() {
                if !(options.diacritic_insensitive && is_combining_mark(l)) {
                    out(l);
                }
            }
        } else if !(options.diacritic_insensitive && is_combining_mark(d)) {
            out(d);
        }
    };
    if options.width_insensitive {
        decompose_compatible(c, &mut emit);
    } else if options.literal {
        emit(c);
    } else {
        decompose_canonical(c, &mut emit);
    }
}

fn fold_mapped(s: &str, options: CompareOptions) -> Vec<(char, usize)> {
    let mut out = Vec::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        push_folded(c, options, |f| out.push((f, i)));
    }
    out
}

fn fold_chars(s: &str, options: CompareOptions) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        push_folded(c, options, |f| out.push(f));
    }
    out
}

/// folded form of s, doesnt depend on locale
pub fn folding(s: &str, options: CompareOptions) -> String {
    fold_chars(s, options).into_iter().collect()
}

/// byte range of the first match (last with backwards)
/// none if no match or needle is empty
pub fn range_of(haystack: &str, needle: &str, options: CompareOptions) -> Option<Range<usize>> {
    let folded_needle = fold_chars(needle, options);
    if folded_needle.is_empty() {
        return None;
    }
    let folded = fold_mapped(haystack, options);
    let n = folded_needle.len();
    if folded.len() < n {
        return None;
    }
    let boundaries = if options.literal { Vec::new() } else { character_boundaries(haystack) };
    let source_start = |a: usize| folded[a].1;
    let source_end = |b: usize| if b < folded.len() { folded[b].1 } else { haystack.len() };
    let accept = |a: usize| -> Option<Range<usize>> {
        let b = a + n;
        if folded[a..b].iter().map(|x| x.0).ne(folded_needle.iter().copied()) {
            return None;
        }
        if a > 0 && folded[a - 1].1 == folded[a].1 {
            return None;
        }
        if b < folded.len() && folded[b - 1].1 == folded[b].1 {
            return None;
        }
        let r = source_start(a)..source_end(b);
        if !options.literal && (boundaries.binary_search(&r.start).is_err() || boundaries.binary_search(&r.end).is_err()) {
            return None;
        }
        Some(r)
    };
    let last = folded.len() - n;
    match (options.anchored, options.backwards) {
        (true, false) => accept(0).map(|r| 0..r.end),
        (true, true) => accept(last),
        (false, false) => (0..=last).find_map(accept),
        (false, true) => (0..=last).rev().find_map(accept),
    }
}

/// same as range_of but only inside a byte range of the haystack
pub fn range_of_in(haystack: &str, needle: &str, options: CompareOptions, search_range: Range<usize>) -> Option<Range<usize>> {
    let slice = haystack.get(search_range.clone())?;
    range_of(slice, needle, options).map(|r| r.start + search_range.start..r.end + search_range.start)
}

/// same search but in utf-16 ranges
pub fn ns_range_of(text: &str, needle: &str, options: CompareOptions, search_range: NSRange) -> Option<NSRange> {
    let bytes = utf16_to_byte_range(text, search_range)?;
    range_of_in(text, needle, options, bytes).map(|r| byte_to_utf16_range(text, r))
}

/// contains check ignoring case and diacritics
pub fn contains_case_diacritic_insensitive(haystack: &str, needle: &str) -> bool {
    range_of(haystack, needle, CompareOptions::CASE_INSENSITIVE.union(CompareOptions::DIACRITIC_INSENSITIVE)).is_some()
}

/// contains check ignoring case, diacritics still count
pub fn localized_case_insensitive_contains(haystack: &str, needle: &str) -> bool {
    range_of(haystack, needle, CompareOptions::CASE_INSENSITIVE).is_some()
}

/// contains check ignoring case and diacritics, like finder search
pub fn localized_standard_contains(haystack: &str, needle: &str) -> bool {
    contains_case_diacritic_insensitive(haystack, needle)
}

/// orders the folded forms by utf-16 code unit
pub fn compare(a: &str, b: &str, options: CompareOptions) -> Ordering {
    let left = folding(a, options);
    let right = folding(b, options);
    left.encode_utf16().cmp(right.encode_utf16())
}

/// case insensitive compare, same text gives Equal
pub fn compare_case_insensitive(a: &str, b: &str) -> Ordering {
    compare(a, b, CompareOptions::CASE_INSENSITIVE)
}

type CollatorsByLocale = HashMap<String, Option<CollatorBorrowed<'static>>>;

thread_local! {
    static COLLATORS: RefCell<[CollatorsByLocale; 2]> = RefCell::new([HashMap::new(), HashMap::new()]);
}

fn new_collator(locale_identifier: &str, case_insensitive: bool) -> Option<CollatorBorrowed<'static>> {
    let locale = IcuLocale::try_from_str(&locale_identifier.replace('_', "-")).unwrap_or(IcuLocale::UNKNOWN);
    let mut options = CollatorOptions::default();
    if case_insensitive {
        options.strength = Some(Strength::Secondary);
    }
    Collator::try_new((&locale).into(), options).ok()
}

fn collate(a: &str, b: &str, locale_identifier: &str, case_insensitive: bool) -> Option<Ordering> {
    COLLATORS.with_borrow_mut(|collators| {
        let by_locale = &mut collators[usize::from(case_insensitive)];
        if !by_locale.contains_key(locale_identifier) {
            by_locale.insert(locale_identifier.to_owned(), new_collator(locale_identifier, case_insensitive));
        }
        by_locale.get(locale_identifier)?.as_ref().map(|collator| collator.compare(a, b))
    })
}

/// compare in the sort order of the locale (eg pt_BR or ru), case still counts
/// unknown locale falls back to the root order
pub fn localized_compare(a: &str, b: &str, locale_identifier: &str) -> Ordering {
    collate(a, b, locale_identifier, false).unwrap_or_else(|| compare(a, b, CompareOptions::NONE))
}

/// like localized_compare but case is ignored, accents still differ
/// callers pass the app locale
pub fn localized_case_insensitive_compare(a: &str, b: &str, locale_identifier: &str) -> Ordering {
    collate(a, b, locale_identifier, true).unwrap_or_else(|| compare_case_insensitive(a, b))
}

/// equal when canonically equivalent, so precomposed and decomposed letters match
pub fn canonically_equal(a: &str, b: &str) -> bool {
    a == b || compare(a, b, CompareOptions::NONE) == Ordering::Equal
}
