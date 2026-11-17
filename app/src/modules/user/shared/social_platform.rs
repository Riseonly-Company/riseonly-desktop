use std::cmp::Ordering;

use gpui::{Hsla, black, white};
use rise_canon::RiseValidation;
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::UrlComponents;
use rise_i18n::tr;
use rise_theme::color;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::lib::canon_rules::{RiseValidationCanonRules, ValidationSchemaCanonRules};
use crate::modules::user::shared::profile_field_rules::ProfileFieldRules;
use crate::modules::user::stores::user::user_actions::user_types::ProfileSocialLink;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SocialPlatform {
    Youtube,
    Instagram,
    X,
    Tiktok,
    Telegram,
    Linkedin,
    Github,
    Facebook,
    Spotify,
    Twitch,
    Discord,
    Reddit,
    Threads,
    Snapchat,
    Pinterest,
    Vk,
    Whatsapp,
    Soundcloud,
    Behance,
    Dribbble,
    Medium,
    Website,
}

impl SocialPlatform {
    pub const ALL_CASES: [SocialPlatform; 22] = [
        SocialPlatform::Youtube,
        SocialPlatform::Instagram,
        SocialPlatform::X,
        SocialPlatform::Tiktok,
        SocialPlatform::Telegram,
        SocialPlatform::Linkedin,
        SocialPlatform::Github,
        SocialPlatform::Facebook,
        SocialPlatform::Spotify,
        SocialPlatform::Twitch,
        SocialPlatform::Discord,
        SocialPlatform::Reddit,
        SocialPlatform::Threads,
        SocialPlatform::Snapchat,
        SocialPlatform::Pinterest,
        SocialPlatform::Vk,
        SocialPlatform::Whatsapp,
        SocialPlatform::Soundcloud,
        SocialPlatform::Behance,
        SocialPlatform::Dribbble,
        SocialPlatform::Medium,
        SocialPlatform::Website,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            SocialPlatform::Youtube => "youtube",
            SocialPlatform::Instagram => "instagram",
            SocialPlatform::X => "x",
            SocialPlatform::Tiktok => "tiktok",
            SocialPlatform::Telegram => "telegram",
            SocialPlatform::Linkedin => "linkedin",
            SocialPlatform::Github => "github",
            SocialPlatform::Facebook => "facebook",
            SocialPlatform::Spotify => "spotify",
            SocialPlatform::Twitch => "twitch",
            SocialPlatform::Discord => "discord",
            SocialPlatform::Reddit => "reddit",
            SocialPlatform::Threads => "threads",
            SocialPlatform::Snapchat => "snapchat",
            SocialPlatform::Pinterest => "pinterest",
            SocialPlatform::Vk => "vk",
            SocialPlatform::Whatsapp => "whatsapp",
            SocialPlatform::Soundcloud => "soundcloud",
            SocialPlatform::Behance => "behance",
            SocialPlatform::Dribbble => "dribbble",
            SocialPlatform::Medium => "medium",
            SocialPlatform::Website => "website",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<SocialPlatform> {
        Self::ALL_CASES.into_iter().find(|platform| platform.raw_value() == raw_value)
    }

    pub fn display_name(self) -> String {
        let brand = match self {
            SocialPlatform::Youtube => "YouTube",
            SocialPlatform::Instagram => "Instagram",
            SocialPlatform::X => "X",
            SocialPlatform::Tiktok => "TikTok",
            SocialPlatform::Telegram => "Telegram",
            SocialPlatform::Linkedin => "LinkedIn",
            SocialPlatform::Github => "GitHub",
            SocialPlatform::Facebook => "Facebook",
            SocialPlatform::Spotify => "Spotify",
            SocialPlatform::Twitch => "Twitch",
            SocialPlatform::Discord => "Discord",
            SocialPlatform::Reddit => "Reddit",
            SocialPlatform::Threads => "Threads",
            SocialPlatform::Snapchat => "Snapchat",
            SocialPlatform::Pinterest => "Pinterest",
            SocialPlatform::Vk => "VK",
            SocialPlatform::Whatsapp => "WhatsApp",
            SocialPlatform::Soundcloud => "SoundCloud",
            SocialPlatform::Behance => "Behance",
            SocialPlatform::Dribbble => "Dribbble",
            SocialPlatform::Medium => "Medium",
            SocialPlatform::Website => return tr("social_link_website"),
        };
        brand.to_owned()
    }

    pub fn asset_name(self) -> Option<String> {
        if self == SocialPlatform::Website {
            return None;
        }
        Some(format!("social_{}", self.raw_value()))
    }

    pub fn system_fallback_icon(self) -> &'static str {
        "paperclip"
    }

    fn tile_hex(self) -> &'static str {
        match self {
            SocialPlatform::Youtube => "#FF0000",
            SocialPlatform::Instagram => "#E1306C",
            SocialPlatform::X => "#000000",
            SocialPlatform::Tiktok => "#010101",
            SocialPlatform::Telegram => "#26A5E4",
            SocialPlatform::Linkedin => "#0A66C2",
            SocialPlatform::Github => "#181717",
            SocialPlatform::Facebook => "#1877F2",
            SocialPlatform::Spotify => "#1DB954",
            SocialPlatform::Twitch => "#9146FF",
            SocialPlatform::Discord => "#5865F2",
            SocialPlatform::Reddit => "#FF4500",
            SocialPlatform::Threads => "#000000",
            SocialPlatform::Snapchat => "#FFFC00",
            SocialPlatform::Pinterest => "#BD081C",
            SocialPlatform::Vk => "#0077FF",
            SocialPlatform::Whatsapp => "#25D366",
            SocialPlatform::Soundcloud => "#FF5500",
            SocialPlatform::Behance => "#1769FF",
            SocialPlatform::Dribbble => "#EA4C89",
            SocialPlatform::Medium => "#000000",
            SocialPlatform::Website => "#2481CC",
        }
    }

    pub fn tile_color(self) -> Hsla {
        color(self.tile_hex())
    }

    pub fn uses_gradient(self) -> bool {
        self == SocialPlatform::Instagram
    }

    pub fn tile_gradient(self) -> LinearGradient {
        LinearGradient::colors(
            [color("#F58529"), color("#DD2A7B"), color("#8134AF"), color("#515BD4")],
            UnitPoint::BOTTOM_LEADING,
            UnitPoint::TOP_TRAILING,
        )
    }

    pub fn glyph_color(self) -> Hsla {
        if SocialColorMath::relative_luminance(self.tile_hex()) > 0.6 { black() } else { white() }
    }

    fn hosts(self) -> &'static [&'static str] {
        match self {
            SocialPlatform::Youtube => &["youtube.com", "youtu.be"],
            SocialPlatform::Instagram => &["instagram.com", "instagr.am"],
            SocialPlatform::X => &["x.com", "twitter.com", "t.co"],
            SocialPlatform::Tiktok => &["tiktok.com"],
            SocialPlatform::Telegram => &["t.me", "telegram.me", "telegram.org"],
            SocialPlatform::Linkedin => &["linkedin.com", "lnkd.in"],
            SocialPlatform::Github => &["github.com", "github.io"],
            SocialPlatform::Facebook => &["facebook.com", "fb.com", "fb.me"],
            SocialPlatform::Spotify => &["spotify.com"],
            SocialPlatform::Twitch => &["twitch.tv"],
            SocialPlatform::Discord => &["discord.gg", "discord.com", "discordapp.com"],
            SocialPlatform::Reddit => &["reddit.com"],
            SocialPlatform::Threads => &["threads.net", "threads.com"],
            SocialPlatform::Snapchat => &["snapchat.com"],
            SocialPlatform::Pinterest => &["pinterest.com", "pin.it"],
            SocialPlatform::Vk => &["vk.com", "vk.ru"],
            SocialPlatform::Whatsapp => &["whatsapp.com", "wa.me"],
            SocialPlatform::Soundcloud => &["soundcloud.com"],
            SocialPlatform::Behance => &["behance.net"],
            SocialPlatform::Dribbble => &["dribbble.com"],
            SocialPlatform::Medium => &["medium.com"],
            SocialPlatform::Website => &[],
        }
    }

    pub fn detect(from_url_string: &str) -> SocialPlatform {
        let host = Self::host_component(from_url_string);
        if host.is_empty() {
            return SocialPlatform::Website;
        }
        for platform in Self::ALL_CASES {
            if platform == SocialPlatform::Website {
                continue;
            }
            for h in platform.hosts() {
                if host == *h || host.ends_with(&format!(".{h}")) {
                    return platform;
                }
            }
        }
        SocialPlatform::Website
    }

    pub fn host_component(of_url_string: &str) -> String {
        let trimmed = trimming_whitespaces_and_newlines(of_url_string);
        if trimmed.is_empty() {
            return String::new();
        }
        let with_scheme =
            if trimmed.contains("://") { trimmed.to_owned() } else { format!("https://{trimmed}") };
        let Some(host) = UrlComponents::from_string(&with_scheme).and_then(|comps| comps.host()) else {
            return String::new();
        };
        let host = host.to_lowercase();
        match host.strip_prefix("www.") {
            Some(stripped) => stripped.to_owned(),
            None => host,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SocialLink {
    pub title: String,
    pub url: String,
    pub platform: SocialPlatform,
}

impl SocialLink {
    pub fn id(&self) -> &str {
        &self.url
    }

    pub fn display_url(&self) -> String {
        let mut s = self.url.as_str();
        if let Some(index) = s.find("://") {
            s = &s[index + 3..];
        }
        if let Some(stripped) = s.strip_prefix("www.") {
            s = stripped;
        }
        s.trim_end_matches('/').to_owned()
    }

    pub fn from(links: &[ProfileSocialLink]) -> Vec<SocialLink> {
        links
            .iter()
            .map(|link| {
                let platform = SocialPlatform::detect(&link.url);
                let trimmed_title = trimming_whitespaces_and_newlines(&link.title);
                SocialLink {
                    title: if trimmed_title.is_empty() {
                        platform.display_name()
                    } else {
                        trimmed_title.to_owned()
                    },
                    url: link.url.clone(),
                    platform,
                }
            })
            .collect()
    }

    pub fn normalize(links: &[ProfileSocialLink]) -> Vec<ProfileSocialLink> {
        let title_schema = ProfileFieldRules::social_link_title().schema;
        let url_schema = ProfileFieldRules::social_link_url().schema;
        let maximum =
            RiseValidation::collection_limit(rise_canon::rise_key::user::SOCIAL_LINKS).maximum;
        let mut normalized: Vec<ProfileSocialLink> = Vec::new();
        for link in links {
            let title = title_schema
                .as_ref()
                .map_or_else(|| link.title.clone(), |schema| schema.normalized_clamped(&link.title));
            let raw_url = url_schema
                .as_ref()
                .map_or_else(|| link.url.clone(), |schema| schema.normalized_clamped(&link.url));
            if title.is_empty() || raw_url.is_empty() {
                continue;
            }

            let url = if raw_url.contains("://") { raw_url } else { format!("https://{raw_url}") };
            if normalized
                .iter()
                .any(|existing| compare_case_insensitive(&existing.url, &url) == Ordering::Equal)
            {
                continue;
            }

            normalized.push(ProfileSocialLink { title, url });
            if normalized.len() >= maximum {
                break;
            }
        }
        normalized
    }
}

pub enum SocialColorMath {}

impl SocialColorMath {
    pub fn relative_luminance(hex: &str) -> f64 {
        let s = trimming_whitespaces_and_newlines(hex);
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.chars().count() != 6 {
            return 0.0;
        }
        let Ok(value) = i64::from_str_radix(s, 16) else {
            return 0.0;
        };
        let r = ((value >> 16) & 0xFF) as f64 / 255.0;
        let g = ((value >> 8) & 0xFF) as f64 / 255.0;
        let b = (value & 0xFF) as f64 / 255.0;
        0.299 * r + 0.587 * g + 0.114 * b
    }
}
