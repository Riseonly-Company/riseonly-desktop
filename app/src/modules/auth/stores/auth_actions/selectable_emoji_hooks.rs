use rise_canon::RiseChatCustomEmoji;
use rise_core::foundation::foundation_character::scalar_is_emoji;
use rise_core::foundation::foundation_text::{
    character_count, is_whitespace_or_newline, trimming_whitespaces_and_newlines,
};

const VARIATION_SELECTOR_TEXT: u32 = 0xFE0E;
const VARIATION_SELECTOR_EMOJI: u32 = 0xFE0F;
const COMBINING_KEYCAP: u32 = 0x20E3;
const SELECTABLE_EMOJI_MAX_BYTES: usize = 64;
const SELECTABLE_EMOJI_MAX_SCALARS: usize = 16;
const CUSTOM_EMOJI_REF_MAX_BYTES: usize = 64;

/// one unicode emoji grapheme or a ce custom emoji ref, as the product stores it
pub fn normalized_selectable_emoji_or_custom_ref(value: Option<&str>) -> Option<String> {
    let value = trimming_whitespaces_and_newlines(value?);
    if value.is_empty() {
        return None;
    }
    if value.starts_with(RiseChatCustomEmoji::PREFIX) {
        return if is_valid_custom_emoji_ref(value) {
            Some(value.to_owned())
        } else {
            None
        };
    }
    normalized_selectable_emoji(Some(value))
}

/// true for a ce ref the server accepts
pub fn is_valid_custom_emoji_ref(value: &str) -> bool {
    let Some(id) = value.strip_prefix(RiseChatCustomEmoji::PREFIX) else {
        return false;
    };
    if value.len() > CUSTOM_EMOJI_REF_MAX_BYTES || id.is_empty() {
        return false;
    }
    id.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// the value back when it is exactly one emoji grapheme
pub fn normalized_selectable_emoji(value: Option<&str>) -> Option<String> {
    let value = trimming_whitespaces_and_newlines(value?);
    if character_count(value) != 1
        || value.is_empty()
        || value.len() > SELECTABLE_EMOJI_MAX_BYTES
        || value.chars().count() > SELECTABLE_EMOJI_MAX_SCALARS
        || value.chars().any(is_whitespace_or_newline)
        || value.chars().any(|scalar| u32::from(scalar) == VARIATION_SELECTOR_TEXT)
    {
        return None;
    }

    let has_emoji = value.chars().any(scalar_is_emoji);
    let has_keycap = value.chars().any(|scalar| u32::from(scalar) == COMBINING_KEYCAP);
    let only_ascii_emoji_base = value.chars().all(|scalar| {
        let code = u32::from(scalar);
        code <= 0x7F || code == VARIATION_SELECTOR_EMOJI || code == COMBINING_KEYCAP
    });
    if !has_emoji || (only_ascii_emoji_base && !has_keycap) {
        return None;
    }

    Some(value.to_owned())
}
