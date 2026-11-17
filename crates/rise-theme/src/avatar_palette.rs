use std::sync::LazyLock;

use gpui::{Hsla, rgb};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::theme::app_theme::readable_ink;
use rise_avatar::{
    FAVOURITES_BACKGROUND, HASH_MULTIPLIER, KEY_FALLBACK, KEY_SCOPE_SEPARATOR, LETTER_COLOR, PALETTE, Rgb,
};

const GENERATED_AVATAR_SEGMENT: &str = "/avatars/generated/";

static PALETTE_COLORS: LazyLock<Vec<Hsla>> =
    LazyLock::new(|| PALETTE.iter().copied().map(color_from_avatar_rgb).collect());

const MULTIPLIER: i32 = HASH_MULTIPLIER as i32;

/// converts an avatar rgb into a color
/// orphan rule blocks a From impl between two foreign types here
pub fn color_from_avatar_rgb(value: Rgb) -> Hsla {
    rgb(value.hex()).into()
}

pub struct AvatarPalette;

impl AvatarPalette {
    pub fn palette() -> &'static [Hsla] {
        PALETTE_COLORS.as_slice()
    }

    pub fn favourites_background() -> Hsla {
        color_from_avatar_rgb(FAVOURITES_BACKGROUND)
    }

    /// one color for the letter over every palette entry, same as the server draws
    pub fn letter_color() -> Hsla {
        color_from_avatar_rgb(LETTER_COLOR)
    }

    /// letter color over the circle background
    /// white while it reads at 3:1 else the better of white and black
    /// one white letter vanishes on light entries like lime
    pub fn letter_color_on(background: Hsla) -> Hsla {
        readable_ink(background, Self::letter_color())
    }

    /// must match the server index for the same key
    /// wrap width, signedness and iteration unit all matter
    /// canon crate has the one implementation
    pub fn simple_hash(value: &str) -> u32 {
        rise_avatar::simple_hash(value)
    }

    fn accumulate(hash: &mut i32, value: &str) {
        for scalar in value.chars() {
            *hash = hash.wrapping_mul(MULTIPLIER).wrapping_add(scalar as i32);
        }
    }

    fn color_for_hash(hash: u32) -> Hsla {
        let colors = Self::palette();
        match colors.len() {
            0 => Self::letter_color(),
            count => colors[hash as usize % count],
        }
    }

    /// hashes the separator in place instead of building scope:entity, saves a string alloc per avatar
    /// same scalar sequence either way, None is the key fallback
    pub fn color_in(scope_id: &str, entity_id: Option<&str>) -> Hsla {
        let mut hash: i32 = 0;
        Self::accumulate(&mut hash, scope_id);
        Self::accumulate(&mut hash, KEY_SCOPE_SEPARATOR);
        Self::accumulate(&mut hash, entity_id.unwrap_or(KEY_FALLBACK));
        Self::color_for_hash(hash.unsigned_abs())
    }

    pub fn color_in_chat(chat_id: &str, user_id: Option<&str>) -> Hsla {
        let user_id = user_id.filter(|value| !value.is_empty()).unwrap_or(KEY_FALLBACK);
        Self::color_in(chat_id, Some(user_id))
    }

    pub fn color_for_user(user_id: Option<&str>, name: Option<&str>) -> Hsla {
        Self::color_for_hash(Self::simple_hash(rise_avatar::user_key(user_id, name)))
    }

    pub fn color_for_chat(chat_id: &str) -> Hsla {
        Self::color_for_hash(Self::simple_hash(rise_avatar::key(chat_id)))
    }

    pub fn display_letter(name: Option<&str>) -> String {
        rise_avatar::display_letter(name.unwrap_or_default())
    }

    fn trimmed_non_empty(value: Option<&str>) -> Option<&str> {
        value.map(trimming_whitespaces_and_newlines).filter(|raw| !raw.is_empty())
    }

    pub fn is_http_logo(value: Option<&str>) -> bool {
        Self::trimmed_non_empty(value).is_some_and(|raw| raw.to_lowercase().starts_with("http"))
    }

    pub fn is_local_logo(value: Option<&str>) -> bool {
        Self::trimmed_non_empty(value)
            .is_some_and(|raw| raw.to_lowercase().starts_with("file://") || is_local_absolute_path(raw))
    }

    pub fn is_generated_avatar_url(value: Option<&str>) -> bool {
        Self::trimmed_non_empty(value).is_some_and(|raw| raw.contains(GENERATED_AVATAR_SEGMENT))
    }

    /// server letter avatars are drawn locally like telegram does
    /// older ones had a random color so the picture differed from the local circle
    /// fresh group logo showed an empty circle until upload landed
    pub fn resolved_logo_url(source: Option<&str>) -> Option<String> {
        if !(Self::is_http_logo(source) || Self::is_local_logo(source)) {
            return None;
        }
        let url = trimming_whitespaces_and_newlines(source?);
        if Self::is_generated_avatar_url(Some(url)) {
            None
        } else {
            Some(url.to_owned())
        }
    }

    pub fn letter_avatar(owner: &str, name: &str) -> (Rgb, Rgb, String) {
        (rise_avatar::background(owner), LETTER_COLOR, rise_avatar::display_letter(name))
    }

    /// owner id of a server letter avatar, entity.generation_palette_letter.png
    /// thats the key the server color hashes, so the local circle matches with no user id
    pub fn generated_avatar_owner_id(source: Option<&str>) -> Option<String> {
        let raw = source?;
        let start = raw.find(GENERATED_AVATAR_SEGMENT)? + GENERATED_AVATAR_SEGMENT.len();
        let rest = &raw[start..];
        let end = rest.find(['_', '.', '/']).unwrap_or(rest.len());
        let owner = &rest[..end];
        if owner.is_empty() { None } else { Some(owner.to_owned()) }
    }
}

#[cfg(test)]
#[path = "avatar_palette_tests.rs"]
mod tests;
