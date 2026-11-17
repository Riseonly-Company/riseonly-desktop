use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::language_store::{LanguageStore, Locale};
use rise_i18n::tr;
use rise_i18n_catalog::RELATIVE_TIME_BANDS;
use rise_notify::{NOTIFICATION_COPY, NOTIFICATION_TYPES, ROUTE_RULES, RouteKind};

use crate::core::lib::canon_format::{CalendarBand, RiseFormat};
use crate::core::push_notifications::admin_console_link::AdminConsoleLink;
use crate::modules::notify::engine::core::rise_announcements_resource_repository::RiseAnnouncementsCacheSource;
use crate::modules::notify::stores::notify_actions::notify_types::{
    NotificationActorItem, NotificationItem, NotificationKind, NotifyCopyCanon,
};

/// calendar band of a feed row, titles come from the canon chrome copy
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotifySectionKind {
    Today,
    Yesterday,
    Week,
    Month,
    Earlier,
}

impl NotifySectionKind {
    pub const ALL_CASES: [NotifySectionKind; 5] = [
        NotifySectionKind::Today,
        NotifySectionKind::Yesterday,
        NotifySectionKind::Week,
        NotifySectionKind::Month,
        NotifySectionKind::Earlier,
    ];

    pub fn raw_value(self) -> i64 {
        match self {
            NotifySectionKind::Today => 0,
            NotifySectionKind::Yesterday => 1,
            NotifySectionKind::Week => 2,
            NotifySectionKind::Month => 3,
            NotifySectionKind::Earlier => 4,
        }
    }

    pub fn from_raw_value(raw: i64) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|kind| kind.raw_value() == raw)
    }

    pub fn title_event(self) -> &'static str {
        match self {
            NotifySectionKind::Today => "feed_section_today",
            NotifySectionKind::Yesterday => "feed_section_yesterday",
            NotifySectionKind::Week => "feed_section_week",
            NotifySectionKind::Month => "feed_section_month",
            NotifySectionKind::Earlier => "feed_section_earlier",
        }
    }
}

/// feed row with every string already resolved
/// names and action and time and section
/// row view only composes runs and colors
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseNotificationRow {
    pub id: String,
    pub item: NotificationItem,
    pub section_kind: NotifySectionKind,
    pub leading: String,
    pub text: String,
    pub time: String,
    pub is_read: bool,
    pub facepile: Vec<NotificationActorItem>,
    pub badge_symbol: &'static str,
    pub leading_is_heading: bool,
    pub accessibility_hint: String,
}

impl RiseNotificationRow {
    pub fn kind(&self) -> NotificationKind {
        self.item.kind
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseNotificationsSection {
    pub kind: NotifySectionKind,
    pub title: String,
    pub rows: Vec<RiseNotificationRow>,
}

impl RiseNotificationsSection {
    pub fn id(&self) -> i64 {
        self.kind.raw_value()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseNotificationsLoadState {
    Idle,
    Loading,
    Loaded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseNotificationsSnapshot {
    pub sections: Vec<RiseNotificationsSection>,
    pub comment_sections: Vec<RiseNotificationsSection>,
    pub row_count: i64,
    pub comment_row_count: i64,
    pub has_more: bool,
    pub is_loading_more: bool,
    pub total_unread: i64,
    pub load_state: RiseNotificationsLoadState,
    pub is_refreshing: bool,
    pub cache_source: RiseAnnouncementsCacheSource,
    pub revision: u64,
    pub has_newer: bool,
    pub is_loading_newer: bool,
    pub head_generation: u64,
}

impl RiseNotificationsSnapshot {
    pub const EMPTY: RiseNotificationsSnapshot = RiseNotificationsSnapshot {
        sections: Vec::new(),
        comment_sections: Vec::new(),
        row_count: 0,
        comment_row_count: 0,
        has_more: false,
        is_loading_more: false,
        total_unread: 0,
        load_state: RiseNotificationsLoadState::Idle,
        is_refreshing: false,
        cache_source: RiseAnnouncementsCacheSource::Miss,
        revision: 0,
        has_newer: false,
        is_loading_newer: false,
        head_generation: 0,
    };

    pub fn has_unread(&self) -> bool {
        self.total_unread > 0
            || self
                .sections
                .iter()
                .any(|section| section.rows.iter().any(|row| !row.is_read))
    }
}

/// ui snapshot of the notification feed, published as a whole value and gated by account and session
pub struct RiseNotificationsPresentation {
    snapshot: RiseNotificationsSnapshot,
    generation: u64,
    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseNotificationsPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseNotificationsSnapshot::EMPTY,
            generation: 0,
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn snapshot(&self) -> &RiseNotificationsSnapshot {
        &self.snapshot
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_active(&self) -> bool {
        self.account_id.is_some()
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        if self.account_id.as_deref() == Some(account_id)
            && self.account_epoch.as_deref() == Some(account_epoch)
            && self.session_id == Some(session_id)
        {
            return;
        }
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.generation = self.generation.wrapping_add(1);
        self.snapshot = RiseNotificationsSnapshot::EMPTY;
        cx.notify();
    }

    pub fn publish(
        &mut self,
        next: RiseNotificationsSnapshot,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.account_id.as_deref() != Some(account_id)
            || self.account_epoch.as_deref() != Some(account_epoch)
            || self.session_id != Some(session_id)
        {
            return false;
        }
        if next.revision < self.snapshot.revision {
            return true;
        }
        if next == self.snapshot {
            return true;
        }
        self.snapshot = next;
        cx.notify();
        true
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.generation = self.generation.wrapping_add(1);
        self.snapshot = RiseNotificationsSnapshot::EMPTY;
        cx.notify();
    }
}

/// every localized string a feed row can need, captured on the main thread once per publish
/// rows are prepared off it from this frozen table
#[derive(Clone, Debug)]
pub struct NotifyFeedChrome {
    pub strings: HashMap<String, String>,
    pub short_date_pattern: String,
    pub language_code: String,
    pub locale: Locale,
}

impl NotifyFeedChrome {
    pub fn capture(cx: &mut App) -> NotifyFeedChrome {
        let mut strings: HashMap<String, String> = HashMap::with_capacity(160);
        let mut add = |key: Option<&str>| {
            let Some(key) = key else {
                return;
            };
            if strings.contains_key(key) {
                return;
            }
            strings.insert(key.to_owned(), tr(key));
        };
        for spec in NOTIFICATION_TYPES.iter() {
            add(Some(spec.row_i18n_key_one));
            add(Some(spec.row_i18n_key_many));
        }
        for copy in NOTIFICATION_COPY.iter() {
            add(copy.client_key);
            add(copy.client_key_many);
            add(copy.client_key_fallback);
        }
        for band in RELATIVE_TIME_BANDS.iter() {
            add(Some(band.key));
            if let Some(template_key) = RiseFormat::relative_time_template_key(band) {
                add(Some(template_key));
            }
        }
        add(Some(RiseNotificationsSnapshotBuilder::ADMIN_HINT_KEY));
        add(Some("music_default_local_playlist_title"));
        let pattern = RiseFormat::calendar_short_date_key(CalendarBand::ThisYear)
            .or_else(|| RiseFormat::calendar_date_key(CalendarBand::ThisYear));
        NotifyFeedChrome {
            strings,
            short_date_pattern: pattern.map(tr).unwrap_or_else(|| "d MMM".to_owned()),
            language_code: LanguageStore::shared(cx).code().to_owned(),
            locale: Locale::app_locale(),
        }
    }

    pub fn text(&self, key: &str) -> String {
        self.strings.get(key).cloned().unwrap_or_else(|| key.to_owned())
    }
}

/// immutable inputs of one prepare
#[derive(Clone, Debug)]
pub struct RiseNotificationsPrepareSeed {
    pub items: Vec<NotificationItem>,
    pub has_more: bool,
    pub is_loading_more: bool,
    pub total_unread: i64,
    pub load_state: RiseNotificationsLoadState,
    pub is_refreshing: bool,
    pub cache_source: RiseAnnouncementsCacheSource,
    pub revision: u64,
    pub has_newer: bool,
    pub is_loading_newer: bool,
    pub head_generation: u64,
}

static MAP_PAYLOAD_TYPES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ROUTE_RULES
        .iter()
        .filter(|rule| rule.route == RouteKind::Map)
        .map(|rule| rule.payload_type)
        .collect()
});

/// pure
/// rows, names and action copy and time and sections from a seed and a frozen chrome table
/// runs off the main thread
pub enum RiseNotificationsSnapshotBuilder {}

impl RiseNotificationsSnapshotBuilder {
    const DATE_INSTEAD_OF_ELAPSED_AFTER_MS: i64 = 7 * 86_400_000;

    pub const ADMIN_HINT_KEY: &'static str = "notify_open_admin_hint";

    pub fn build(
        seed: &RiseNotificationsPrepareSeed,
        chrome: &NotifyFeedChrome,
        now: Date,
        calendar: &Calendar,
        locale: &Locale,
    ) -> RiseNotificationsSnapshot {
        let now_ms = now.ms();
        let formatter_pattern = chrome.short_date_pattern.as_str();

        let mut buckets: HashMap<NotifySectionKind, Vec<RiseNotificationRow>> = HashMap::new();
        let mut comment_buckets: HashMap<NotifySectionKind, Vec<RiseNotificationRow>> =
            HashMap::new();
        for item in &seed.items {
            let row = Self::make_row(
                item,
                chrome,
                now,
                now_ms,
                calendar,
                formatter_pattern,
                locale,
            );
            if item.kind.is_comment_related() {
                comment_buckets
                    .entry(row.section_kind)
                    .or_default()
                    .push(row.clone());
            }
            buckets.entry(row.section_kind).or_default().push(row);
        }

        let sections = |buckets: &mut HashMap<NotifySectionKind, Vec<RiseNotificationRow>>| {
            NotifySectionKind::ALL_CASES
                .into_iter()
                .filter_map(|kind| {
                    let rows = buckets.remove(&kind)?;
                    if rows.is_empty() {
                        return None;
                    }
                    Some(RiseNotificationsSection {
                        kind,
                        title: chrome.text(NotifyCopyCanon::chrome_key(kind.title_event())),
                        rows,
                    })
                })
                .collect::<Vec<_>>()
        };

        let all = sections(&mut buckets);
        let comments = sections(&mut comment_buckets);
        let row_count: i64 = all.iter().map(|section| section.rows.len() as i64).sum();
        let comment_row_count: i64 = comments.iter().map(|section| section.rows.len() as i64).sum();
        RiseNotificationsSnapshot {
            sections: all,
            comment_sections: comments,
            row_count,
            comment_row_count,
            has_more: seed.has_more,
            is_loading_more: seed.is_loading_more,
            total_unread: seed.total_unread,
            load_state: seed.load_state,
            is_refreshing: seed.is_refreshing,
            cache_source: seed.cache_source,
            revision: seed.revision,
            has_newer: seed.has_newer,
            is_loading_newer: seed.is_loading_newer,
            head_generation: seed.head_generation,
        }
    }

    fn make_row(
        item: &NotificationItem,
        chrome: &NotifyFeedChrome,
        now: Date,
        now_ms: i64,
        calendar: &Calendar,
        formatter_pattern: &str,
        locale: &Locale,
    ) -> RiseNotificationRow {
        let translate = |key: &str| chrome.text(key);
        let time = Self::relative_time(
            item.created_at_ms,
            now_ms,
            formatter_pattern,
            calendar,
            locale,
            &translate,
        );
        let section = Self::section_kind(item.created_at_ms, now, calendar);
        let accessibility_hint = if item
            .admin_url()
            .and_then(|url| AdminConsoleLink::url(&url))
            .is_none()
        {
            String::new()
        } else {
            translate(Self::ADMIN_HINT_KEY)
        };
        let is_moderation_verdict = item.kind == NotificationKind::Moderation
            && !trimming_whitespaces_and_newlines(&item.title).is_empty();
        if item.kind == NotificationKind::Announcement
            || item.is_admin_alert()
            || is_moderation_verdict
        {
            return RiseNotificationRow {
                id: item.id.clone(),
                item: item.clone(),
                section_kind: section,
                leading: trimming_whitespaces_and_newlines(&item.title).to_owned(),
                text: trimming_whitespaces_and_newlines(&item.body).to_owned(),
                time,
                is_read: item.is_read,
                facepile: Vec::new(),
                badge_symbol: item.badge_symbol(),
                leading_is_heading: true,
                accessibility_hint,
            };
        }
        let facepile = item.facepile_actors();
        let mut leading = item.names_string(&translate);
        let mut text = item.action_suffix(&translate);
        if MAP_PAYLOAD_TYPES.contains(item.event_type().as_str()) && !item.body.is_empty() {
            let prefix = format!("{leading} ");
            if !leading.is_empty() && text.starts_with(&prefix) {
                text = text[prefix.len()..].to_owned();
            } else {
                leading = String::new();
            }
        } else if !leading.is_empty() && !item.body.is_empty() && text == item.body {
            let prefix = format!("{leading} ");
            if let Some(rest) = text.strip_prefix(&prefix) {
                text = rest.to_owned();
            }
        }
        RiseNotificationRow {
            id: item.id.clone(),
            item: item.clone(),
            section_kind: section,
            leading,
            text,
            time,
            is_read: item.is_read,
            facepile,
            badge_symbol: item.badge_symbol(),
            leading_is_heading: false,
            accessibility_hint,
        }
    }

    /// canon bands like now or 5 min or 3 d
    /// past a week it prints a date
    /// spaces become non breaking so 2 h ago wraps as one piece
    fn relative_time(
        created_at_ms: i64,
        now_ms: i64,
        formatter_pattern: &str,
        calendar: &Calendar,
        locale: &Locale,
        translate: &dyn Fn(&str) -> String,
    ) -> String {
        if created_at_ms <= 0 {
            return String::new();
        }
        let time = if now_ms - created_at_ms >= Self::DATE_INSTEAD_OF_ELAPSED_AFTER_MS {
            RiseFormat::date_formatter(formatter_pattern, locale.identifier(), calendar, None)
                .string_from(&Date::from_ms(created_at_ms))
        } else {
            RiseFormat::elapsed_translating(created_at_ms, now_ms, translate)
        };
        time.replace(' ', "\u{00A0}")
    }

    fn section_kind(created_at_ms: i64, now: Date, calendar: &Calendar) -> NotifySectionKind {
        if created_at_ms <= 0 {
            return NotifySectionKind::Earlier;
        }
        let date = Date::from_ms(created_at_ms);
        if calendar.is_date_in_same_day_as(&date, &now) {
            return NotifySectionKind::Today;
        }
        if let Some(yesterday) = calendar.date_by_adding(CalendarComponent::Day, -1, &now)
            && calendar.is_date_in_same_day_as(&date, &yesterday)
        {
            return NotifySectionKind::Yesterday;
        }
        if let Some(week_ago) = calendar.date_by_adding(CalendarComponent::Day, -7, &now)
            && date > week_ago
        {
            return NotifySectionKind::Week;
        }
        if calendar.is_date_equal_to(&date, &now, CalendarComponent::Month) {
            return NotifySectionKind::Month;
        }
        NotifySectionKind::Earlier
    }
}
