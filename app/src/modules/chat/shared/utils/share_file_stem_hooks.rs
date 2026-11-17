use rise_core::foundation::foundation_character::scalar_is_control_character;
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};

/// file stem rules of a file handed to the system
pub enum ShareFileStem {}

impl ShareFileStem {
    const CHARACTER_LIMIT: usize = 96;
    pub const FALLBACK: &'static str = "Riseonly";

    pub fn sanitize(raw: &str) -> String {
        let replaced: String = raw
            .chars()
            .map(|scalar| if Self::is_disallowed(scalar) { '_' } else { scalar })
            .collect();
        let value = trimming_whitespaces_and_newlines(&replaced);
        let resolved = if value.is_empty() || value == "." || value == ".." {
            Self::FALLBACK
        } else {
            value
        };
        character_prefix(resolved, Self::CHARACTER_LIMIT).to_owned()
    }

    fn is_disallowed(scalar: char) -> bool {
        matches!(scalar, '/' | '\\' | ':') || scalar_is_control_character(scalar)
    }
}
