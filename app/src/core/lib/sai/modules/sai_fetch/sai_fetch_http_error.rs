use std::fmt;

use rise_core::foundation::foundation_text::{character_count, character_prefix, trimming_whitespaces_and_newlines};

const BODY_SNIPPET_MAX_CHARACTERS: usize = 800;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaiFetchHttpError {
    InvalidUrl(String),
    MissingBaseUrl(String),
    BadStatus { code: i64, body: Vec<u8> },
    EmptyResponse,
}

impl SaiFetchHttpError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::InvalidUrl(s) => format!("Invalid URL: {s}"),
            Self::MissingBaseUrl(s) => format!("Missing baseURL for relative route: {s}"),
            Self::BadStatus { code, body } => format!("HTTP {code}{}", Self::body_snippet(body)),
            Self::EmptyResponse => "Empty HTTP response body.".to_owned(),
        })
    }

    fn body_snippet(body: &[u8]) -> String {
        let Ok(text) = std::str::from_utf8(body) else {
            return String::new();
        };
        let s = trimming_whitespaces_and_newlines(text);
        if s.is_empty() {
            return String::new();
        }
        if character_count(s) > BODY_SNIPPET_MAX_CHARACTERS {
            return format!(" \u{2014} {}\u{2026}", character_prefix(s, BODY_SNIPPET_MAX_CHARACTERS));
        }
        format!(" \u{2014} {s}")
    }
}

impl fmt::Display for SaiFetchHttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for SaiFetchHttpError {}
