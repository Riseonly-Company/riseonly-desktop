use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, TimeZone};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::{Locale, tr};

use crate::core::lib::canon_identity::RiseIdentity;
use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryLoadState, RiseConversationAuxiliarySnapshot,
};
use crate::modules::chat::engine::messages::rise_message_read_receipt_wire_types::RiseMessageReadReceiptStoredResource;

const MAXIMUM_SCOPES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseMessageReadReceiptScope {
    pub chat_id: String,
    pub message_id: String,
}

impl RiseMessageReadReceiptScope {
    pub fn new(chat_id: impl Into<String>, message_id: impl Into<String>) -> Self {
        Self { chat_id: chat_id.into(), message_id: message_id.into() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMessageReadReceiptSummaryKind {
    Readers,
    Timestamp,
}

impl RiseMessageReadReceiptSummaryKind {
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Readers => "readers",
            Self::Timestamp => "timestamp",
        }
    }

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        match raw_value {
            "readers" => Some(Self::Readers),
            "timestamp" => Some(Self::Timestamp),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RiseMessageReadReceiptUserPresentation {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
    pub is_premium: bool,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
    pub read_at_milliseconds: i64,
    pub read_at_text: String,
}

#[derive(Clone, Debug)]
pub struct RiseMessageReadReceiptPresentationSnapshot {
    pub account_id: String,
    pub scope: RiseMessageReadReceiptScope,
    pub source_revision: i64,
    pub readers: Vec<RiseMessageReadReceiptUserPresentation>,
    pub total_count: i64,
    pub summary_kind: Option<RiseMessageReadReceiptSummaryKind>,
    pub representative_read_at_milliseconds: Option<i64>,
    pub representative_read_at_text: Option<String>,
    pub is_local_hint: bool,
    pub load_state: RiseConversationAuxiliaryLoadState,
    pub error: Option<String>,
}

impl RiseMessageReadReceiptPresentationSnapshot {
    pub fn has_resolved_initial(&self) -> bool {
        self.source_revision > 0
            || self.load_state == RiseConversationAuxiliaryLoadState::Loaded
            || self.load_state == RiseConversationAuxiliaryLoadState::Failed
    }

    /// a local hint knows when but never by whom, so a group summary waits for the real list
    pub fn has_resolved_summary(&self, is_direct: bool) -> bool {
        if !self.has_resolved_initial() {
            return false;
        }
        if !self.is_local_hint {
            return true;
        }
        is_direct && self.representative_read_at_text.is_some()
    }

    pub fn including_reactors<F>(&self, reactors: &[MessageReadReceiptReactor], make_formatter: impl FnOnce() -> F) -> Self
    where
        F: FnMut(i64) -> String,
    {
        if self.summary_kind != Some(RiseMessageReadReceiptSummaryKind::Readers)
            || !self.has_resolved_initial()
            || self.is_local_hint
            || self.total_count > self.readers.len() as i64
        {
            return self.clone();
        }
        let mut known: HashSet<&str> = self.readers.iter().map(|reader| reader.id.as_str()).collect();
        let missing: Vec<&MessageReadReceiptReactor> = reactors
            .iter()
            .filter(|reactor| reactor.reacted_at_milliseconds > 0 && known.insert(reactor.id.as_str()))
            .collect();
        if missing.is_empty() {
            return self.clone();
        }
        let mut formatted = make_formatter();
        let mut extra: Vec<RiseMessageReadReceiptUserPresentation> = Vec::with_capacity(missing.len());
        for reactor in missing {
            extra.push(RiseMessageReadReceiptUserPresentation {
                id: reactor.id.clone(),
                name: if reactor.name.is_empty() { reactor.id.clone() } else { reactor.name.clone() },
                tag: String::new(),
                logo: reactor.logo.clone(),
                is_premium: false,
                premium_badge_emoji: None,
                name_color: None,
                read_at_milliseconds: reactor.reacted_at_milliseconds,
                read_at_text: formatted(reactor.reacted_at_milliseconds),
            });
        }
        let extra_count = extra.len() as i64;
        let mut merged: Vec<RiseMessageReadReceiptUserPresentation> =
            self.readers.iter().cloned().chain(extra).collect();
        merged.sort_by(|a, b| {
            if a.read_at_milliseconds != b.read_at_milliseconds {
                b.read_at_milliseconds.cmp(&a.read_at_milliseconds)
            } else {
                a.id.cmp(&b.id)
            }
        });
        Self {
            account_id: self.account_id.clone(),
            scope: self.scope.clone(),
            source_revision: self.source_revision,
            readers: merged,
            total_count: self.total_count.max(self.readers.len() as i64) + extra_count,
            summary_kind: self.summary_kind,
            representative_read_at_milliseconds: self.representative_read_at_milliseconds,
            representative_read_at_text: self.representative_read_at_text.clone(),
            is_local_hint: self.is_local_hint,
            load_state: self.load_state,
            error: self.error.clone(),
        }
    }

    pub fn empty(account_id: &str, scope: RiseMessageReadReceiptScope) -> Self {
        Self {
            account_id: account_id.to_owned(),
            scope,
            source_revision: 0,
            readers: Vec::new(),
            total_count: 0,
            summary_kind: None,
            representative_read_at_milliseconds: None,
            representative_read_at_text: None,
            is_local_hint: false,
            load_state: RiseConversationAuxiliaryLoadState::Idle,
            error: None,
        }
    }
}

/// a person shown under a reaction in the hold menu
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageReadReceiptReactor {
    pub id: String,
    pub name: String,
    pub logo: String,
    pub reacted_at_milliseconds: i64,
}

#[derive(Clone, Debug)]
pub struct RiseMessageReadReceiptFormatSeed {
    pub locale_identifier: String,
    pub time_zone_identifier: String,
    pub now_milliseconds: i64,
    pub today_format: String,
    pub yesterday_format: String,
    pub date_format: String,
}

impl RiseMessageReadReceiptFormatSeed {
    pub fn current(_cx: &mut App) -> Self {
        Self {
            locale_identifier: Locale::app_locale().identifier().to_owned(),
            time_zone_identifier: TimeZone::current().identifier(),
            now_milliseconds: (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64,
            today_format: tr("date_today_at_format"),
            yesterday_format: tr("date_yesterday_at_format"),
            date_format: tr("message_read_receipt_date_at_format"),
        }
    }

    /// one formatter for a batch of rows so the preparer and the reactor merge read stamps the same way
    pub fn text_formatter(&self) -> impl FnMut(i64) -> String + use<> {
        let mut formatter = DateFormatter::new();
        formatter.set_locale(self.locale_identifier.as_str());
        if let Some(time_zone) = TimeZone::with_identifier(&self.time_zone_identifier) {
            formatter.set_time_zone(time_zone);
        }
        let calendar = Calendar::gregorian();
        let now = Date::from_time_interval_since_1970(self.now_milliseconds as f64 / 1_000.0);
        let today_format = self.today_format.clone();
        let yesterday_format = self.yesterday_format.clone();
        let date_format = self.date_format.clone();

        move |milliseconds: i64| -> String {
            let date = Date::from_time_interval_since_1970(RiseIdentity::milliseconds(milliseconds) as f64 / 1_000.0);
            if calendar.is_date_in_same_day_as(&date, &now) {
                formatter.set_date_format(today_format.as_str());
            } else if calendar
                .date_by_adding(CalendarComponent::Day, -1, &now)
                .is_some_and(|yesterday| calendar.is_date_in_same_day_as(&date, &yesterday))
            {
                formatter.set_date_format(yesterday_format.as_str());
            } else {
                formatter.set_date_format(date_format.as_str());
            }
            formatter.string_from(&date)
        }
    }
}

#[derive(Default)]
pub struct RiseMessageReadReceiptPresentationPreparer;

impl RiseMessageReadReceiptPresentationPreparer {
    pub fn new() -> Self {
        Self
    }

    pub async fn prepare(
        &self,
        account_id: &str,
        scope: &RiseMessageReadReceiptScope,
        source: RiseConversationAuxiliarySnapshot<RiseMessageReadReceiptStoredResource>,
        format_seed: RiseMessageReadReceiptFormatSeed,
    ) -> RiseMessageReadReceiptPresentationSnapshot {
        let response = source.value.as_ref().map(|value| &value.response);
        let mut formatted = format_seed.text_formatter();

        let mut readers: Vec<RiseMessageReadReceiptUserPresentation> = Vec::new();
        for receipt in response.and_then(|response| response.receipts.as_ref()).into_iter().flatten() {
            let Some(user) = receipt.user.as_ref() else {
                continue;
            };
            let Some(id) = Self::normalized(user.id.as_deref()) else {
                continue;
            };
            let Some(read_at) = receipt.read_at.filter(|read_at| *read_at > 0) else {
                continue;
            };
            let tag = Self::normalized(user.tag.as_deref());
            readers.push(RiseMessageReadReceiptUserPresentation {
                name: Self::normalized(user.name.as_deref()).or_else(|| tag.clone()).unwrap_or_else(|| id.clone()),
                id,
                tag: tag.unwrap_or_default(),
                logo: Self::normalized(user.more.as_ref().and_then(|more| more.logo.as_deref())).unwrap_or_default(),
                is_premium: user.is_premium.unwrap_or(false),
                premium_badge_emoji: Self::normalized(user.premium_badge_emoji.as_deref()),
                name_color: Self::normalized(user.name_color.as_deref()),
                read_at_milliseconds: read_at,
                read_at_text: formatted(read_at),
            });
        }
        let readers_count = readers.len() as i64;
        let representative_read_at = response
            .and_then(|response| response.representative_read_at)
            .or_else(|| readers.first().map(|reader| reader.read_at_milliseconds));
        let summary_kind = response
            .and_then(|response| response.summary_kind.as_deref())
            .and_then(RiseMessageReadReceiptSummaryKind::from_raw_value);
        let total_count = response.and_then(|response| response.total_count).unwrap_or(readers_count).max(readers_count);
        let representative_read_at_text = representative_read_at.map(&mut formatted);
        RiseMessageReadReceiptPresentationSnapshot {
            account_id: account_id.to_owned(),
            scope: scope.clone(),
            source_revision: source.revision,
            readers,
            total_count,
            summary_kind,
            representative_read_at_milliseconds: representative_read_at,
            representative_read_at_text,
            is_local_hint: source.value.as_ref().is_some_and(|value| value.is_local_hint == Some(true)),
            load_state: source.load_state,
            error: source.error.clone(),
        }
    }

    fn normalized(value: Option<&str>) -> Option<String> {
        let normalized = trimming_whitespaces_and_newlines(value?);
        (!normalized.is_empty()).then(|| normalized.to_owned())
    }
}

pub struct RiseMessageReadReceiptPresentation {
    account_id: Option<String>,
    revision: u64,
    snapshots: HashMap<RiseMessageReadReceiptScope, RiseMessageReadReceiptPresentationSnapshot>,
    order: Vec<RiseMessageReadReceiptScope>,
}

impl RiseMessageReadReceiptPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self { account_id: None, revision: 0, snapshots: HashMap::new(), order: Vec::new() }
    }

    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn activate(&mut self, account_id: &str, cx: &mut Context<Self>) {
        let normalized = trimming_whitespaces_and_newlines(account_id);
        if normalized.is_empty() {
            self.deactivate(cx);
            return;
        }
        if self.account_id.as_deref() == Some(normalized) {
            return;
        }
        self.account_id = Some(normalized.to_owned());
        self.snapshots = HashMap::new();
        self.order = Vec::new();
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }

    pub fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.account_id = None;
        self.snapshots = HashMap::new();
        self.order = Vec::new();
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }

    pub fn publish(&mut self, snapshot: RiseMessageReadReceiptPresentationSnapshot, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(snapshot.account_id.as_str()) {
            return;
        }
        let scope = snapshot.scope.clone();
        self.snapshots.insert(scope.clone(), snapshot);
        self.order.retain(|existing| *existing != scope);
        self.order.push(scope);
        while self.order.len() > MAXIMUM_SCOPES {
            let oldest = self.order.remove(0);
            self.snapshots.remove(&oldest);
        }
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }

    pub fn remove(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        let count = self.snapshots.len();
        self.snapshots.retain(|scope, _| scope.chat_id != chat_id);
        if self.snapshots.len() == count {
            return;
        }
        self.order.retain(|scope| scope.chat_id != chat_id);
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }

    pub fn snapshot(&self, chat_id: &str, message_id: &str) -> RiseMessageReadReceiptPresentationSnapshot {
        let scope = RiseMessageReadReceiptScope::new(chat_id, message_id);
        match self.snapshots.get(&scope) {
            Some(snapshot) => snapshot.clone(),
            None => RiseMessageReadReceiptPresentationSnapshot::empty(self.account_id.as_deref().unwrap_or_default(), scope),
        }
    }
}
