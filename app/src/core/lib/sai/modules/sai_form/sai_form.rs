use std::collections::{HashMap, HashSet};
use std::ops::RangeInclusive;
use std::sync::Arc;

use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_regex::{NSMatchingOptions, NSRegularExpression};
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::{FormatArg, tr, tr_args};

const EMAIL_SEPARATOR: char = '@';
const EMAIL_PARTS: usize = 2;

/// form validation rule, empty or missing message falls back to the canon text
#[derive(Clone)]
pub enum SaiFormRule {
    Required(String),
    MinLength(usize, Option<String>),
    MaxLength(usize, Option<String>),
    Pattern(String, String),
    Email(String),
    IntRange(RangeInclusive<i64>, String),
    OneOf(HashSet<String>, String),
    Custom(Arc<dyn Fn(&str) -> Option<String> + Send + Sync>),
}

impl SaiFormRule {
    pub fn validate(&self, raw: &str) -> Option<String> {
        let s = raw;
        match self {
            Self::Required(msg) => {
                let resolved = if msg.is_empty() { tr("form_required") } else { msg.clone() };
                trimming_whitespaces_and_newlines(s).is_empty().then_some(resolved)
            }
            Self::MinLength(n, msg) => {
                if character_count(s) >= *n {
                    return None;
                }
                Some(msg.clone().unwrap_or_else(|| length_message("form_min_length", *n)))
            }
            Self::MaxLength(n, msg) => {
                if character_count(s) <= *n {
                    return None;
                }
                Some(msg.clone().unwrap_or_else(|| length_message("form_max_length", *n)))
            }
            Self::Pattern(pat, msg) => {
                let Ok(re) = NSRegularExpression::with_pattern(pat) else {
                    return Some(msg.clone());
                };
                let range = NSRange::full(s);
                match re.first_match(s, NSMatchingOptions::default(), range) {
                    Some(_) => None,
                    None => Some(msg.clone()),
                }
            }
            Self::Email(msg) => {
                let t = trimming_whitespaces_and_newlines(s);
                if t.is_empty() {
                    return None;
                }
                let parts = t.split(EMAIL_SEPARATOR).filter(|part| !part.is_empty()).count();
                let ok = t.contains(EMAIL_SEPARATOR) && parts == EMAIL_PARTS;
                if ok {
                    None
                } else if msg.is_empty() {
                    Some(tr("form_invalid_email"))
                } else {
                    Some(msg.clone())
                }
            }
            Self::IntRange(r, msg) => {
                let t = trimming_whitespaces_and_newlines(s);
                if t.is_empty() {
                    return Some(msg.clone());
                }
                let Some(v) = int_from_string(t) else {
                    return Some(msg.clone());
                };
                if r.contains(&v) { None } else { Some(msg.clone()) }
            }
            Self::OneOf(set, msg) => {
                if set.contains(trimming_whitespaces_and_newlines(s)) {
                    None
                } else {
                    Some(msg.clone())
                }
            }
            Self::Custom(f) => f(s),
        }
    }
}

fn length_message(key: &str, n: usize) -> String {
    tr_args(key, &[FormatArg::Int(i64::try_from(n).unwrap_or(i64::MAX))])
}

#[derive(Clone, Default)]
pub struct SaiFormSchema {
    pub fields: HashMap<String, Vec<SaiFormRule>>,
}

impl SaiFormSchema {
    pub fn new(fields: HashMap<String, Vec<SaiFormRule>>) -> Self {
        Self { fields }
    }
}

pub struct SaiForm {
    values: HashMap<String, String>,
    errors: HashMap<String, String>,
    pub schema: SaiFormSchema,
}

impl SaiForm {
    pub fn new(values: HashMap<String, String>, schema: SaiFormSchema) -> Self {
        Self {
            values,
            errors: HashMap::new(),
            schema,
        }
    }

    pub fn values(&self) -> &HashMap<String, String> {
        &self.values
    }

    pub fn errors(&self) -> &HashMap<String, String> {
        &self.errors
    }

    pub fn value(&self, name: &str) -> String {
        self.values.get(name).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, name: &str, value: &str, clear_error: bool) {
        self.values.insert(name.to_owned(), value.to_owned());
        if clear_error {
            self.errors.remove(name);
        }
    }

    /// error text for a field if it has one
    pub fn error(&self, name: &str) -> Option<&str> {
        self.errors.get(name).map(String::as_str)
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// validates the form or only the given fields, true when all pass
    pub fn validate(&mut self, only: Option<&HashSet<String>>) -> bool {
        self.errors.clear();
        let mut ok = true;
        for (name, rules) in &self.schema.fields {
            if let Some(subset) = only
                && !subset.contains(name)
            {
                continue;
            }
            let raw = self.values.get(name).map(String::as_str).unwrap_or_default();
            for rule in rules {
                if let Some(msg) = rule.validate(raw) {
                    self.errors.insert(name.clone(), msg);
                    ok = false;
                    break;
                }
            }
        }
        ok
    }

    pub fn validate_field(&mut self, name: &str) -> bool {
        let only = HashSet::from([name.to_owned()]);
        self.validate(Some(&only))
    }

    pub fn clear_errors(&mut self) {
        self.errors.clear();
    }

    pub fn reset(&mut self, new_values: HashMap<String, String>) {
        self.values = new_values;
        self.errors.clear();
    }
}
