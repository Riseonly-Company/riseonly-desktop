use gpui::{App, Hsla, rgb};
use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};

use crate::core::ui::premium_auth_hooks::PremiumAuthHooks;
use crate::modules::post::stores::post_actions::post_types::AuthorInfoMore;

pub enum PremiumNameColor {}

impl PremiumNameColor {
    /// color to draw the name in, none for the theme default
    pub fn color(more: Option<&AuthorInfoMore>) -> Option<Hsla> {
        let more = more?;
        if !more.is_premium {
            return None;
        }
        Self::parse(more.name_color.as_deref())
    }

    /// prefers the live session profile for the viewer own name so a change lands the moment its saved
    /// main thread only, the session store is main thread
    pub fn color_for_user(user_id: Option<&str>, fallback: Option<&AuthorInfoMore>, cx: &App) -> Option<Hsla> {
        if let Some(user_id) = user_id
            && !user_id.is_empty()
            && PremiumAuthHooks::is_current_user(user_id, cx)
            && let Some(profile) = PremiumAuthHooks::profile(cx)
        {
            return Self::color(Some(&profile.more));
        }
        Self::color(fallback)
    }

    /// #RRGGBB only in either case, anything else is the default color
    pub fn parse(raw: Option<&str>) -> Option<Hsla> {
        let raw = trimming_whitespaces_and_newlines(raw?);
        if character_count(raw) != 7 {
            return None;
        }
        let digits = raw.strip_prefix('#')?;
        let value = u32::from_str_radix(digits, 16).ok()?;
        Some(rgb(value).into())
    }
}
