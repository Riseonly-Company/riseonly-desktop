use std::cmp::Ordering;
use std::collections::HashSet;
use std::ops::Range;

use gpui::Hsla;
use rise_core::foundation::foundation_compare::{CompareOptions, compare, range_of};
use rise_energy::rise_energy_saving::RiseEnergySavingStore;
use rise_i18n::tr;
use rise_theme::color;
use rise_ui::settings_icon_palette::SettingsIconPalette;

use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::rise_feature_gate::RiseFeatureGate;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::PackKind;
use crate::modules::user::shared::privacy_mock::PrivacyRuleCatalog;

#[derive(Clone, Debug, PartialEq)]
pub enum SettingsSearchTarget {
    Route(RootRoute),
    BugReport,
    MyBugReports,
    GiftShop,
}

#[derive(Clone, Debug)]
pub struct SettingsSearchEntry {
    pub id: String,
    pub title_key: &'static str,
    pub path_keys: Vec<&'static str>,
    pub icon: &'static str,
    pub icon_background: Hsla,
    pub target: SettingsSearchTarget,
}

#[derive(Clone, Debug)]
pub struct SettingsSearchResult {
    pub entry: SettingsSearchEntry,
    pub title: String,
    pub crumb: String,
    pub highlights: Vec<Range<usize>>,
    pub rank: u8,
}

const PROFILE: &str = "settings_section_profile";
const SECURITY: &str = "settings_section_security";
const CHATS: &str = "settings_section_chats";
const APP: &str = "settings_section_app";
const SERVICES: &str = "settings_section_services";
const ACCOUNT: &str = "settings_section_account";
const PROFILE_PAGE: &str = "settings_profile_title";
const PRIVACY_PAGE: &str = "settings_privacy_title";
const NOTIFICATIONS_PAGE: &str = "settings_notifications_title";
const CUSTOMIZATION_PAGE: &str = "settings_customization_title";
const MEMORY_PAGE: &str = "settings_memory_title";
const REPORT: &str = "report_riseonly_title";
const CALLS: &str = "desktop_settings_calls_title";
const STREAMS_PAGE: &str = "desktop_settings_streams_title";
const CRUMB_SEPARATOR: &str = " \u{203A} ";
const OPTIONS: CompareOptions = CompareOptions::CASE_INSENSITIVE.union(CompareOptions::DIACRITIC_INSENSITIVE);

pub enum SettingsSearchIndex {}

impl SettingsSearchIndex {
    pub fn entries() -> Vec<SettingsSearchEntry> {
        use SettingsSearchTarget::Route;
        let mut out = vec![
            entry(
                "profile",
                "settings_profile_title",
                &[PROFILE],
                "person.fill",
                SettingsIconPalette::profile(),
                Route(RootRoute::ProfileSettings),
            ),
            entry(
                "achievements",
                "settings_achievements_title",
                &[PROFILE],
                "trophy",
                SettingsIconPalette::achievements(),
                Route(RootRoute::Achievements { user_id: None }),
            ),
            entry(
                "stories_archive",
                "archive_title",
                &[PROFILE],
                "archivebox.fill",
                SettingsIconPalette::profile(),
                Route(RootRoute::StoriesArchive),
            ),
            entry(
                "close_friends",
                "story_close_friends_title",
                &[PROFILE],
                "star.fill",
                SettingsIconPalette::privacy_friends(),
                Route(RootRoute::CloseFriends),
            ),
            entry(
                "social_links",
                "social_links_title",
                &[PROFILE, PROFILE_PAGE],
                "link",
                SettingsIconPalette::profile(),
                Route(RootRoute::SocialLinksEditor),
            ),
            entry(
                "premium_badge",
                "settings_premium_badge_choose",
                &[PROFILE, PROFILE_PAGE],
                "seal.fill",
                SettingsIconPalette::subscription(),
                Route(RootRoute::PremiumBadgeSettings),
            ),
            entry(
                "name_color",
                "settings_premium_name_color_choose",
                &[PROFILE, PROFILE_PAGE],
                "paintpalette.fill",
                SettingsIconPalette::subscription(),
                Route(RootRoute::ProfileSettings),
            ),
            entry(
                "story_stealth",
                "settings_premium_story_stealth_title",
                &[PROFILE, PROFILE_PAGE],
                "eye.slash.fill",
                SettingsIconPalette::subscription(),
                Route(RootRoute::ProfileSettings),
            ),
            entry(
                "privacy",
                "settings_privacy_title",
                &[SECURITY],
                "lock.fill",
                SettingsIconPalette::privacy(),
                Route(RootRoute::PrivacySettings),
            ),
            entry(
                "blocked_users",
                "privacy_blocked_users_title",
                &[SECURITY, PRIVACY_PAGE],
                "person.crop.circle.badge.xmark",
                SettingsIconPalette::blocks(),
                Route(RootRoute::BlockedUsers),
            ),
            entry(
                "phone_public",
                "privacy_phone_public_title",
                &[SECURITY, PRIVACY_PAGE],
                "phone.fill",
                SettingsIconPalette::privacy(),
                Route(RootRoute::PrivacySettings),
            ),
            entry(
                "message_requests",
                "privacy_message_requests_title",
                &[SECURITY, PRIVACY_PAGE],
                "tray.full.fill",
                SettingsIconPalette::privacy(),
                Route(RootRoute::PrivacySettings),
            ),
            entry(
                "contact_sync",
                "contact_sync_settings_title",
                &[SECURITY, PRIVACY_PAGE],
                "person.2.fill",
                SettingsIconPalette::privacy(),
                Route(RootRoute::PrivacySettings),
            ),
            entry(
                "abuse_appeal",
                "abuse_appeal_settings_row",
                &[SECURITY, PRIVACY_PAGE],
                "exclamationmark.shield.fill",
                SettingsIconPalette::privacy(),
                Route(RootRoute::PrivacySettings),
            ),
            entry(
                "sessions",
                "settings_sessions_title",
                &[SECURITY],
                "iphone",
                SettingsIconPalette::sessions(),
                Route(RootRoute::SessionsSettings),
            ),
            entry(
                "blocks",
                "blocks_page_title",
                &[SECURITY],
                "hand.raised.fill",
                SettingsIconPalette::blocks(),
                Route(RootRoute::MyBlocks),
            ),
            entry(
                "view_history",
                "view_history_page_title",
                &[SECURITY],
                "clock.arrow.circlepath",
                color("#5AC8FA"),
                Route(RootRoute::PostViewHistory),
            ),
            entry(
                "moderation",
                "settings_moderations_title",
                &[SECURITY],
                "shield.lefthalf.filled",
                SettingsIconPalette::moderation(),
                Route(RootRoute::ModerationSettings),
            ),
            entry(
                "notify",
                "settings_notifications_title",
                &[CHATS],
                "bell.fill",
                SettingsIconPalette::notifications(),
                Route(RootRoute::NotificationSettings),
            ),
            entry(
                "notify_mute_all",
                "notification_settings_mute_all",
                &[CHATS, NOTIFICATIONS_PAGE],
                "bell.slash.fill",
                SettingsIconPalette::notifications(),
                Route(RootRoute::NotificationSettings),
            ),
            entry(
                "notify_language",
                "settings_notification_language_title",
                &[CHATS, NOTIFICATIONS_PAGE],
                "globe",
                SettingsIconPalette::notifications(),
                Route(RootRoute::NotificationLanguageSettings),
            ),
            entry(
                "notify_music",
                "notification_settings_music",
                &[CHATS, NOTIFICATIONS_PAGE],
                "music.note",
                SettingsIconPalette::notifications(),
                Route(RootRoute::MusicNotificationSettings),
            ),
            entry(
                "quick_reaction",
                "settings_quick_reaction_title",
                &[CHATS],
                "hand.thumbsup.fill",
                SettingsIconPalette::quick_reaction(),
                Route(RootRoute::QuickReactionSettings),
            ),
            entry(
                "stickers",
                "settings_stickers_title",
                &[CHATS],
                "face.smiling.inverse",
                SettingsIconPalette::stickers(),
                Route(RootRoute::StickerSettings { kind: PackKind::Sticker }),
            ),
            entry(
                "emoji",
                "settings_emoji_title",
                &[CHATS],
                "face.smiling",
                SettingsIconPalette::stickers(),
                Route(RootRoute::StickerSettings { kind: PackKind::Emoji }),
            ),
            entry(
                "chat_folders",
                "settings_chat_folders_title",
                &[CHATS],
                "folder.fill",
                SettingsIconPalette::chat_folders(),
                Route(RootRoute::ChatFoldersSettings),
            ),
            entry(
                "my_bots",
                "bot_my_bots_title",
                &[CHATS],
                "command.circle.fill",
                color("#5856D6"),
                Route(RootRoute::MyBots),
            ),
            entry(
                "guides",
                "guide_catalogue_title",
                &[APP],
                "book.closed.fill",
                SettingsIconPalette::customization(),
                Route(RootRoute::GuidesCatalogue),
            ),
            entry(
                "custom",
                "settings_customization_title",
                &[APP],
                "paintpalette.fill",
                SettingsIconPalette::customization(),
                Route(RootRoute::CustomizationSettings),
            ),
            entry(
                "theme",
                "settings_customization_your_theme",
                &[APP, CUSTOMIZATION_PAGE],
                "paintpalette",
                color("#00A9BB"),
                Route(RootRoute::ThemeSettings),
            ),
            entry(
                "my_themes",
                "settings_my_themes",
                &[APP, CUSTOMIZATION_PAGE],
                "swatchpalette",
                color("#E17055"),
                Route(RootRoute::ThemesList),
            ),
            entry(
                "chat_wallpapers",
                "settings_customization_chat_wallpapers",
                &[APP, CUSTOMIZATION_PAGE],
                "photo.on.rectangle.angled",
                color("#0083E0"),
                Route(RootRoute::ChatWallpapersSettings),
            ),
            entry(
                "tab_bar",
                "settings_tab_bar",
                &[APP, CUSTOMIZATION_PAGE],
                "dock.rectangle",
                color("#5E5CE6"),
                Route(RootRoute::TabBarSettings),
            ),
            entry(
                "energy",
                "settings_energy",
                &[APP, CUSTOMIZATION_PAGE],
                "bolt.batteryblock",
                color("#FDCB6E"),
                Route(RootRoute::EnergySavingSettings),
            ),
            entry(
                "launch_animation",
                "settings_launch_animation",
                &[APP, CUSTOMIZATION_PAGE],
                "play.circle",
                SettingsIconPalette::customization(),
                Route(RootRoute::LaunchAnimationSettings),
            ),
            entry(
                "text_size",
                "settings_text_size",
                &[APP, CUSTOMIZATION_PAGE],
                "textformat.size",
                color("#00B894"),
                Route(RootRoute::TextSizeSettings),
            ),
            entry(
                "bg_color",
                "settings_bg_color",
                &[APP, CUSTOMIZATION_PAGE],
                "rectangle.fill",
                color("#0984E3"),
                Route(RootRoute::BgColorSettings),
            ),
            entry(
                "block_color",
                "settings_block_color",
                &[APP, CUSTOMIZATION_PAGE],
                "square.grid.2x2.fill",
                color("#00B894"),
                Route(RootRoute::BlockColorSettings),
            ),
            entry(
                "btn_color",
                "settings_btn_color",
                &[APP, CUSTOMIZATION_PAGE],
                "capsule.fill",
                color("#6C5CE7"),
                Route(RootRoute::BtnColorSettings),
            ),
            entry(
                "primary_color",
                "settings_primary_color",
                &[APP, CUSTOMIZATION_PAGE],
                "circle.lefthalf.filled",
                color("#E84393"),
                Route(RootRoute::PrimaryColorSettings),
            ),
            entry(
                "text_color",
                "settings_text_color",
                &[APP, CUSTOMIZATION_PAGE],
                "textformat",
                color("#00A9BB"),
                Route(RootRoute::TextColorSettings),
            ),
            entry(
                "secondary_text_color",
                "settings_secondary_text_color",
                &[APP, CUSTOMIZATION_PAGE],
                "textformat.alt",
                color("#5AC8FA"),
                Route(RootRoute::SecondaryTextColorSettings),
            ),
            entry(
                "border_color",
                "settings_border_color",
                &[APP, CUSTOMIZATION_PAGE],
                "square.dashed",
                color("#8E8E93"),
                Route(RootRoute::BorderColorSettings),
            ),
            entry(
                "language",
                "settings_language_title",
                &[APP],
                "globe",
                SettingsIconPalette::language(),
                Route(RootRoute::LanguageSettings),
            ),
            entry(
                "region",
                "settings_region_title",
                &[APP],
                "globe.europe.africa.fill",
                SettingsIconPalette::language(),
                Route(RootRoute::RegionSettings),
            ),
            entry(
                "memory",
                "settings_memory_title",
                &[APP],
                "externaldrive.fill",
                SettingsIconPalette::memory(),
                Route(RootRoute::MemorySettings),
            ),
            entry(
                "memory_media",
                "memory_settings_cached_media",
                &[APP, MEMORY_PAGE],
                "photo.stack.fill",
                SettingsIconPalette::memory(),
                Route(RootRoute::MemorySettings),
            ),
            entry(
                "memory_chats",
                "memory_settings_cached_chats",
                &[APP, MEMORY_PAGE],
                "bubble.left.and.bubble.right.fill",
                SettingsIconPalette::memory(),
                Route(RootRoute::MemorySettings),
            ),
            entry(
                "voice_video",
                "rtc_voice_settings",
                &[CALLS],
                "waveform.and.mic",
                color("#34C759"),
                Route(RootRoute::RtcAudioSettings),
            ),
            entry(
                "streams",
                "desktop_settings_streams_title",
                &[CALLS],
                "wave.3.right",
                color("#AF52DE"),
                Route(RootRoute::RtcStreamSettings),
            ),
            entry(
                "streamer_mode",
                "desktop_streamer_mode_title",
                &[CALLS, STREAMS_PAGE],
                "wave.3.right",
                color("#AF52DE"),
                Route(RootRoute::RtcStreamSettings),
            ),
            entry(
                "bug_report_open",
                "report_riseonly_title",
                &[REPORT],
                "bubble.left.and.exclamationmark.bubble.right.fill",
                color("#FF3B30"),
                SettingsSearchTarget::BugReport,
            ),
            entry(
                "bug_report_mine",
                "bug_report_my_title",
                &[REPORT],
                "tray.full.fill",
                color("#5856D6"),
                SettingsSearchTarget::MyBugReports,
            ),
            entry(
                "music",
                "settings_music_title",
                &[SERVICES],
                "music.note",
                color("#E84393"),
                Route(RootRoute::MusicSettings),
            ),
            entry(
                "subscription",
                "settings_subscription_title",
                &[ACCOUNT],
                "crown.fill",
                SettingsIconPalette::subscription(),
                Route(RootRoute::SubscriptionSettings),
            ),
            entry(
                "sparks",
                "sparks_my_title",
                &[ACCOUNT],
                "sparkles",
                SettingsIconPalette::subscription(),
                Route(RootRoute::SparksWallet),
            ),
            entry(
                "items",
                "settings_items_title",
                &[ACCOUNT],
                "gift.fill",
                SettingsIconPalette::privacy_gifts(),
                SettingsSearchTarget::GiftShop,
            ),
            entry(
                "about_project",
                "settings_about_project",
                &["more"],
                "info.circle.fill",
                SettingsIconPalette::legal(),
                Route(RootRoute::AboutProject),
            ),
        ];
        if RiseFeatureGate::VACANCIES {
            out.push(entry(
                "vacancies",
                "settings_vacancies_title",
                &[SERVICES],
                "briefcase.fill",
                color("#5856D6"),
                Route(RootRoute::VacancySettings),
            ));
        }
        for rule in PrivacyRuleCatalog::settings() {
            let mut privacy_rule = entry(
                "privacy_rule",
                rule.title_key(),
                &[SECURITY, PRIVACY_PAGE],
                "lock.fill",
                SettingsIconPalette::privacy(),
                Route(RootRoute::PrivacySetting { r#type: Some(rule.id().to_owned()) }),
            );
            privacy_rule.id = format!("privacy_rule_{}", rule.id());
            out.push(privacy_rule);
        }
        if !RiseEnergySavingStore::applies_to_device_now() {
            out.retain(|entry| entry.id != "energy");
        }
        out
    }

    pub fn results(raw_query: &str, entries: &[SettingsSearchEntry]) -> Vec<SettingsSearchResult> {
        let query = raw_query.trim();
        if query.is_empty() {
            return Vec::new();
        }
        let words: Vec<&str> = query.split_whitespace().collect();
        let mut results = Vec::new();
        let mut seen_titles = HashSet::new();
        for entry in entries {
            let title = tr(entry.title_key);
            if title.is_empty() || !seen_titles.insert(format!("{title}|{}", entry.path_keys.concat())) {
                continue;
            }
            let path = entry.path_keys.iter().map(|key| tr(key)).collect::<Vec<_>>().join(" ");
            let Some((rank, highlights)) = Self::matched(&words, query, &title, &path) else {
                continue;
            };
            let crumb = Self::crumb(entry, &title);
            results.push(SettingsSearchResult { entry: entry.clone(), title, crumb, highlights, rank });
        }
        results.sort_by(|left, right| {
            left.rank
                .cmp(&right.rank)
                .then(left.entry.path_keys.len().cmp(&right.entry.path_keys.len()))
                .then(left.title.chars().count().cmp(&right.title.chars().count()))
        });
        results
    }

    fn crumb(entry: &SettingsSearchEntry, title: &str) -> String {
        let mut parts: Vec<String> = Vec::new();
        for key in &entry.path_keys {
            let part = tr(key);
            if part.is_empty() || part == title || parts.last() == Some(&part) {
                continue;
            }
            parts.push(part);
        }
        parts.join(CRUMB_SEPARATOR)
    }

    fn matched(words: &[&str], whole: &str, title: &str, path: &str) -> Option<(u8, Vec<Range<usize>>)> {
        if let Some(range) = range_of(title, whole, OPTIONS) {
            let rank = if range.start == 0 {
                0
            } else if Self::starts_word(&range, title) {
                1
            } else {
                2
            };
            return Some((rank, vec![range]));
        }
        if words.len() > 1 {
            let ranges: Vec<Range<usize>> = words.iter().filter_map(|word| range_of(title, word, OPTIONS)).collect();
            if ranges.len() == words.len() {
                return Some((2, ranges));
            }
        }
        if words.len() == 1
            && whole.chars().count() > 1
            && let Some(letters) = Self::subsequence(whole, title)
        {
            return Some((3, letters));
        }
        if words.iter().all(|word| range_of(path, word, OPTIONS).is_some()) {
            return Some((4, Vec::new()));
        }
        None
    }

    fn starts_word(range: &Range<usize>, text: &str) -> bool {
        text[..range.start].chars().next_back().is_none_or(|previous| !previous.is_alphanumeric())
    }

    fn subsequence(query: &str, title: &str) -> Option<Vec<Range<usize>>> {
        let mut ranges = Vec::new();
        let mut wanted = query.chars().peekable();
        let mut buffer = [0_u8; 4];
        let mut wanted_buffer = [0_u8; 4];
        for (index, character) in title.char_indices() {
            let Some(next) = wanted.peek().copied() else {
                break;
            };
            let left: &str = character.encode_utf8(&mut buffer);
            let right: &str = next.encode_utf8(&mut wanted_buffer);
            if compare(left, right, OPTIONS) == Ordering::Equal {
                ranges.push(index..index + character.len_utf8());
                wanted.next();
            }
        }
        wanted.peek().is_none().then_some(ranges)
    }
}

fn entry(
    id: &str,
    title_key: &'static str,
    path_keys: &[&'static str],
    icon: &'static str,
    icon_background: Hsla,
    target: SettingsSearchTarget,
) -> SettingsSearchEntry {
    SettingsSearchEntry { id: id.to_owned(), title_key, path_keys: path_keys.to_vec(), icon, icon_background, target }
}
