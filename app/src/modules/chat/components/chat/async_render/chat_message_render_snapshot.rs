use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use gpui::App;
use parking_lot::Mutex;
use rise_canon::rise_key::otp as otp_key;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarIdentifier, TimeZone};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::language_store::Locale;
use rise_i18n::tr;

use crate::core::lib::canon_format::{CalendarBand, RiseFormat};
#[cfg(debug_assertions)]
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
#[cfg(debug_assertions)]
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::rise_async_text_message_row::RiseAsyncTextMessageEligibility;
use crate::modules::chat::components::chat::async_render::rise_message_fallback_height_cache::RiseMessageGeometryVersion;
use crate::modules::chat::components::chat::bubble::message_link_preview_view::MessageVoiceLinkGlimpse;
use crate::modules::chat::components::chat::media_viewer::chat_media_gallery_builder::{
    ChatViewLimitedDestruction, ChatViewLimitedDestructionStrings,
};
use crate::modules::chat::components::chat::message_system_pill::{MessageSystemFormatContext, MessageSystemFormatter};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseMessagePresentationContentStamp;
use crate::modules::chat::engine::messages::rise_chat_calendar::RiseChatCalendarWindow;
use crate::modules::chat::shared::chat_saved_message_presentation::ChatSavedMessageIdentity;
use crate::modules::chat::shared::chat_search_highlight::ChatMessageSearchHighlight;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageContentType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatMessageRenderEntryKind {
    DateSeparator,
    TopicSeparator,
    UnreadSeparator,
    System,
    Message,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChatMessageRenderEntryGroupFlags {
    pub is_first: bool,
    pub is_last: bool,
}

#[derive(Clone, Debug)]
pub struct ChatMessageRenderEntry {
    pub id: String,
    pub kind: ChatMessageRenderEntryKind,
    pub label: String,
    pub topic_id: Option<String>,
    pub day_start_ms: Option<i64>,
    pub message: Option<Arc<GetMessageMessage>>,
    pub group: Option<ChatMessageRenderEntryGroupFlags>,
    pub source_content_stamp: Option<RiseMessagePresentationContentStamp>,
    pub prepared_geometry_version: Option<RiseMessageGeometryVersion>,
    pub search_highlight_ranges: Option<Vec<NSRange>>,
    pub plain_text_eligibility: Option<bool>,
    pub content_version: i64,
}

impl ChatMessageRenderEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        kind: ChatMessageRenderEntryKind,
        label: String,
        topic_id: Option<String>,
        day_start_ms: Option<i64>,
        message: Option<Arc<GetMessageMessage>>,
        group: Option<ChatMessageRenderEntryGroupFlags>,
        source_content_stamp: Option<RiseMessagePresentationContentStamp>,
        prepared_geometry_version: Option<RiseMessageGeometryVersion>,
        search_highlight_ranges: Option<Vec<NSRange>>,
        plain_text_eligibility: Option<bool>,
        content_version: i64,
    ) -> Self {
        Self {
            id,
            kind,
            label,
            topic_id,
            day_start_ms,
            message,
            group,
            source_content_stamp,
            prepared_geometry_version,
            search_highlight_ranges,
            plain_text_eligibility,
            content_version,
        }
    }
}

pub enum RiseMessageGeometryFingerprintPolicy {}

impl RiseMessageGeometryFingerprintPolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn version(
        id: &str,
        kind: ChatMessageRenderEntryKind,
        label: &str,
        topic_id: Option<&str>,
        message: Option<&GetMessageMessage>,
        group: Option<ChatMessageRenderEntryGroupFlags>,
        source_content_stamp: Option<RiseMessagePresentationContentStamp>,
        request_key: &ChatMessageRenderRequestKey,
    ) -> Option<RiseMessageGeometryVersion> {
        let message = message?;
        let mut hasher = DefaultHasher::new();
        "rise-message-prepared-geometry-v1".hash(&mut hasher);
        request_key.chat_id.hash(&mut hasher);
        request_key.calendar_identifier.hash(&mut hasher);
        request_key.time_zone_identifier.hash(&mut hasher);
        id.hash(&mut hasher);
        kind.hash(&mut hasher);
        label.hash(&mut hasher);
        topic_id.hash(&mut hasher);
        group.hash(&mut hasher);
        if let Some(source_content_stamp) = source_content_stamp {
            0_i64.hash(&mut hasher);
            source_content_stamp.hash(&mut hasher);
        } else {
            1_i64.hash(&mut hasher);
            hash_message(message, &mut hasher);
        }
        Some(RiseMessageGeometryVersion {
            raw_value: hasher.finish(),
        })
    }
}

fn hash_message(message: &GetMessageMessage, hasher: &mut DefaultHasher) {
    struct HashingWriter<'a>(&'a mut DefaultHasher);

    impl std::fmt::Write for HashingWriter<'_> {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            self.0.write(text.as_bytes());
            Ok(())
        }
    }

    let _ = write!(HashingWriter(hasher), "{message:?}");
}

/// goes from the background preparer to the main thread as an immutable value tree, never mutated after
#[derive(Clone)]
pub struct ChatMessageRenderSnapshot {
    pub source_version: i64,
    pub context_key: ChatMessageRenderContextKey,
    pub message_ids: Vec<String>,
    pub message_contexts_by_id: HashMap<String, ChatMessageRenderMessageContext>,
    pub entry_bundles_by_message_id: HashMap<String, Vec<Arc<ChatMessageRenderEntry>>>,
    pub entries: Vec<Arc<ChatMessageRenderEntry>>,
    pub ids: Vec<String>,
    pub entries_by_id: HashMap<String, Arc<ChatMessageRenderEntry>>,
    pub versions_by_id: HashMap<String, i64>,
    pub indices_by_id: HashMap<String, usize>,
    pub delta: ChatMessageRenderDelta,
}

impl ChatMessageRenderSnapshot {
    pub fn new(
        source_version: i64,
        context_key: ChatMessageRenderContextKey,
        message_ids: Vec<String>,
        message_contexts_by_id: HashMap<String, ChatMessageRenderMessageContext>,
        entry_bundles_by_message_id: HashMap<String, Vec<Arc<ChatMessageRenderEntry>>>,
        entries: Vec<Arc<ChatMessageRenderEntry>>,
        previous: Option<&ChatMessageRenderSnapshot>,
    ) -> Self {
        let mut ids: Vec<String> = Vec::with_capacity(entries.len());
        let mut entries_by_id: HashMap<String, Arc<ChatMessageRenderEntry>> = HashMap::with_capacity(entries.len());
        let mut versions_by_id: HashMap<String, i64> = HashMap::with_capacity(entries.len());
        let mut indices_by_id: HashMap<String, usize> = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            ids.push(entry.id.clone());
            entries_by_id.insert(entry.id.clone(), Arc::clone(entry));
            versions_by_id.insert(entry.id.clone(), entry.content_version);
            indices_by_id.insert(entry.id.clone(), index);
        }
        let delta = match previous {
            None => ChatMessageRenderDelta {
                inserted_ids: ids.iter().cloned().collect(),
                removed_ids: HashSet::new(),
                updated_ids: HashSet::new(),
            },
            Some(previous) => {
                let previous_id_set: HashSet<&String> = previous.ids.iter().collect();
                let current_id_set: HashSet<&String> = ids.iter().collect();
                let mut updated_ids: HashSet<String> = HashSet::with_capacity(ids.len().min(previous.ids.len()));
                for id in current_id_set.intersection(&previous_id_set) {
                    if previous.versions_by_id.get(*id) != versions_by_id.get(*id) {
                        updated_ids.insert((*id).clone());
                    }
                }
                ChatMessageRenderDelta {
                    inserted_ids: current_id_set
                        .difference(&previous_id_set)
                        .map(|id| (*id).clone())
                        .collect(),
                    removed_ids: previous_id_set
                        .difference(&current_id_set)
                        .map(|id| (*id).clone())
                        .collect(),
                    updated_ids,
                }
            }
        };
        Self {
            source_version,
            context_key,
            message_ids,
            message_contexts_by_id,
            entry_bundles_by_message_id,
            entries,
            ids,
            entries_by_id,
            versions_by_id,
            indices_by_id,
            delta,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessageRenderContextKey {
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
    pub chat_id: String,
    pub group_by_topic: bool,
    pub topic_titles: HashMap<String, String>,
    pub locale_identifier: String,
    pub calendar_identifier: String,
    pub time_zone_identifier: String,
    pub reference_day: TimeInterval,
    pub system_current_user_ids: HashSet<String>,
    pub system_strings: HashMap<String, String>,
    pub view_limited_opened: HashSet<String>,
    pub view_limited_closed: HashSet<String>,
}

impl ChatMessageRenderContextKey {
    pub fn new(
        key: &ChatMessageRenderRequestKey,
        reference_day: TimeInterval,
        system_format_context: &MessageSystemFormatContext,
    ) -> Self {
        Self {
            account_user_id: key.account_user_id.clone(),
            account_epoch: key.account_epoch.clone(),
            chat_id: key.chat_id.clone(),
            group_by_topic: key.group_by_topic,
            topic_titles: key.topic_titles.clone(),
            locale_identifier: key.locale_identifier.clone(),
            calendar_identifier: key.calendar_identifier.clone(),
            time_zone_identifier: key.time_zone_identifier.clone(),
            reference_day,
            system_current_user_ids: system_format_context.current_user_ids.clone(),
            system_strings: system_format_context.strings.clone(),
            view_limited_opened: key.view_limited_opened.clone(),
            view_limited_closed: key.view_limited_closed.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessageRenderMessageContext {
    pub content_stamp: Option<RiseMessagePresentationContentStamp>,
    pub previous_id: Option<String>,
    pub previous_content_stamp: Option<RiseMessagePresentationContentStamp>,
    pub next_id: Option<String>,
    pub next_content_stamp: Option<RiseMessagePresentationContentStamp>,
    pub search_highlight_ranges: Option<Vec<NSRange>>,
    pub opens_unread: bool,
    pub precedes_unread: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatMessageRenderDelta {
    pub inserted_ids: HashSet<String>,
    pub removed_ids: HashSet<String>,
    pub updated_ids: HashSet<String>,
}

impl ChatMessageRenderDelta {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessageRenderRequestKey {
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
    pub chat_id: String,
    pub source_version: i64,
    pub group_by_topic: bool,
    pub topic_titles: HashMap<String, String>,
    pub locale_identifier: String,
    pub calendar_identifier: String,
    pub time_zone_identifier: String,
    pub saved_messages_chat: bool,
    pub view_limited_opened: HashSet<String>,
    pub view_limited_closed: HashSet<String>,
}

pub enum ChatMessageRenderRequestPolicy {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatMessageRenderRequestPolicyAction {
    UseCached,
    CoalesceInFlight,
    ReplaceInFlight,
}

impl ChatMessageRenderRequestPolicy {
    pub fn action(
        cached_key: Option<&ChatMessageRenderRequestKey>,
        pending_key: Option<&ChatMessageRenderRequestKey>,
        has_in_flight_task: bool,
        incoming_key: &ChatMessageRenderRequestKey,
    ) -> ChatMessageRenderRequestPolicyAction {
        if cached_key == Some(incoming_key) {
            return ChatMessageRenderRequestPolicyAction::UseCached;
        }
        if has_in_flight_task && pending_key == Some(incoming_key) {
            return ChatMessageRenderRequestPolicyAction::CoalesceInFlight;
        }
        ChatMessageRenderRequestPolicyAction::ReplaceInFlight
    }

    pub fn accepts_prepared_result(
        expected_generation: u64,
        current_generation: u64,
        expected_key: &ChatMessageRenderRequestKey,
        pending_key: Option<&ChatMessageRenderRequestKey>,
        active_account_user_id: Option<&str>,
        active_account_epoch: Option<&str>,
    ) -> bool {
        expected_generation == current_generation
            && pending_key == Some(expected_key)
            && active_account_user_id == expected_key.account_user_id.as_deref()
            && active_account_epoch == expected_key.account_epoch.as_deref()
    }
}

/// strings and patterns resolve on the main thread and travel into preparation by value
/// tr reads a shared dictionary while snapshot assembly runs off the main thread
#[derive(Clone, Debug)]
pub struct ChatMessageRenderPreparationSeedLocalization {
    pub today: String,
    pub yesterday: String,
    pub day_month_pattern: String,
    pub day_month_year_pattern: String,
    pub general_topic: String,
    pub unknown_topic: String,
    pub unread_messages: String,
    pub view_limited: ChatViewLimitedDestructionStrings,
}

pub struct ChatMessageRenderPreparationSeed {
    pub key: ChatMessageRenderRequestKey,
    pub messages: Vec<Arc<GetMessageMessage>>,
    pub content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp>,
    pub system_format_context: MessageSystemFormatContext,
    pub localization: ChatMessageRenderPreparationSeedLocalization,
    pub locale: Locale,
    pub calendar: Calendar,
    pub time_zone: TimeZone,
    pub now: Date,
    pub search_highlight: Option<ChatMessageSearchHighlight>,
    pub unread_separator_message_id: Option<String>,
}

impl ChatMessageRenderPreparationSeed {
    #[allow(clippy::too_many_arguments)]
    pub fn make(
        messages: Vec<Arc<GetMessageMessage>>,
        chat_id: &str,
        source_version: i64,
        content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp>,
        group_by_topic: bool,
        topic_titles: HashMap<String, String>,
        locale: Locale,
        calendar: &Calendar,
        time_zone: TimeZone,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        saved_messages_chat: bool,
        search_highlight: Option<ChatMessageSearchHighlight>,
        unread_separator_message_id: Option<String>,
        view_limited_opened: HashSet<String>,
        view_limited_closed: HashSet<String>,
        cx: &App,
    ) -> Self {
        let mut prepared_calendar = calendar.clone();
        prepared_calendar.time_zone = time_zone.clone();
        let calendar_identifier = match calendar.identifier {
            CalendarIdentifier::Gregorian => "gregorian",
        };
        Self {
            key: ChatMessageRenderRequestKey {
                account_user_id,
                account_epoch,
                chat_id: chat_id.to_owned(),
                source_version,
                group_by_topic,
                topic_titles: if group_by_topic { topic_titles } else { HashMap::new() },
                locale_identifier: locale.identifier().to_owned(),
                calendar_identifier: calendar_identifier.to_owned(),
                time_zone_identifier: time_zone.identifier(),
                saved_messages_chat,
                view_limited_opened,
                view_limited_closed,
            },
            messages: MessageVoiceLinkGlimpse::baking(messages, cx),
            content_stamps_by_local_id,
            system_format_context: MessageSystemFormatContext::current(cx).acting_channel(chat_id, cx),
            localization: ChatMessageRenderPreparationSeedLocalization {
                today: tr(RiseFormat::calendar_word_key(CalendarBand::Today, false).unwrap_or("date_today")),
                yesterday: tr(RiseFormat::calendar_word_key(CalendarBand::Yesterday, false)
                    .unwrap_or("date_yesterday")),
                day_month_pattern: tr(RiseFormat::calendar_date_key(CalendarBand::ThisYear)
                    .unwrap_or("date_day_month_format")),
                day_month_year_pattern: tr(RiseFormat::calendar_date_key(CalendarBand::Older)
                    .unwrap_or("date_day_month_year_format")),
                general_topic: tr("chat_topics_general"),
                unknown_topic: tr("chat_topics_topic"),
                unread_messages: tr("chat_unread_messages_divider"),
                view_limited: ChatViewLimitedDestructionStrings::current(),
            },
            locale,
            calendar: prepared_calendar,
            time_zone,
            now: Date::now(),
            search_highlight,
            unread_separator_message_id,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatMessageRenderSnapshotTraceMetrics {
    pub message_count: usize,
    pub previous_entry_count: usize,
    pub result_entry_count: usize,
    pub reused_message_bundles: usize,
    pub rebuilt_message_bundles: usize,
    pub optional_message_comparisons: usize,
    pub deep_message_comparisons: usize,
    pub reused_content_versions: usize,
    pub changed_content_versions: usize,
    pub grouping_duration_ms: f64,
    pub previous_index_duration_ms: f64,
    pub entry_loop_duration_ms: f64,
    pub message_comparison_duration_ms: f64,
    pub snapshot_index_duration_ms: f64,
    pub total_duration_ms: f64,
}

impl ChatMessageRenderSnapshotTraceMetrics {
    pub fn trace_fields(&self) -> HashMap<String, String> {
        HashMap::from([
            ("messages".to_owned(), self.message_count.to_string()),
            ("previous_entries".to_owned(), self.previous_entry_count.to_string()),
            ("result_entries".to_owned(), self.result_entry_count.to_string()),
            ("reused_message_bundles".to_owned(), self.reused_message_bundles.to_string()),
            ("rebuilt_message_bundles".to_owned(), self.rebuilt_message_bundles.to_string()),
            (
                "optional_message_comparisons".to_owned(),
                self.optional_message_comparisons.to_string(),
            ),
            ("deep_message_comparisons".to_owned(), self.deep_message_comparisons.to_string()),
            ("reused_versions".to_owned(), self.reused_content_versions.to_string()),
            ("changed_versions".to_owned(), self.changed_content_versions.to_string()),
            ("grouping_ms".to_owned(), Self::decimal(self.grouping_duration_ms)),
            ("previous_index_ms".to_owned(), Self::decimal(self.previous_index_duration_ms)),
            ("entry_loop_ms".to_owned(), Self::decimal(self.entry_loop_duration_ms)),
            ("message_compare_ms".to_owned(), Self::decimal(self.message_comparison_duration_ms)),
            ("snapshot_index_ms".to_owned(), Self::decimal(self.snapshot_index_duration_ms)),
            (otp_key::DURATION_MS.to_owned(), Self::decimal(self.total_duration_ms)),
        ])
    }

    fn decimal(value: f64) -> String {
        format!("{value:.3}")
    }
}

#[derive(Clone)]
pub struct ChatMessageRenderPreparationResult {
    pub snapshot: ChatMessageRenderSnapshot,
    pub metrics: ChatMessageRenderSnapshotTraceMetrics,
}

/// shared with the background task behind an Arc
/// the lock replaces the actor so preparations never overlap and content versions stay monotonic
#[derive(Default)]
pub struct ChatMessageRenderSnapshotPreparer {
    next_content_version: Mutex<i64>,
}

impl ChatMessageRenderSnapshotPreparer {
    pub fn new() -> Self {
        Self::default()
    }

    /// is_cancelled reads the generation of the request
    pub fn prepare(
        &self,
        seed: &ChatMessageRenderPreparationSeed,
        previous: Option<&ChatMessageRenderSnapshot>,
        traces_performance: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<ChatMessageRenderPreparationResult> {
        let mut next_content_version = self.next_content_version.lock();
        ChatMessageRenderSnapshotCore::make(
            seed,
            previous,
            &mut next_content_version,
            traces_performance,
            Some(is_cancelled),
        )
    }
}

/// row fields of the snapshot core entry that have default values
struct ChatMessageRenderEntryFields {
    id: String,
    kind: ChatMessageRenderEntryKind,
    label: String,
    topic_id: Option<String>,
    day_start_ms: Option<i64>,
    message: Option<Arc<GetMessageMessage>>,
    group: Option<ChatMessageRenderEntryGroupFlags>,
    source_content_stamp: Option<RiseMessagePresentationContentStamp>,
    search_highlight_ranges: Option<Vec<NSRange>>,
}

impl ChatMessageRenderEntryFields {
    fn new(id: String, kind: ChatMessageRenderEntryKind) -> Self {
        Self {
            id,
            kind,
            label: String::new(),
            topic_id: None,
            day_start_ms: None,
            message: None,
            group: None,
            source_content_stamp: None,
            search_highlight_ranges: None,
        }
    }
}

fn started_at(traces_performance: bool) -> Duration {
    if traces_performance { RiseClock::monotonic() } else { Duration::ZERO }
}

fn elapsed_ms(started_at: Duration) -> f64 {
    RiseClock::monotonic().saturating_sub(started_at).as_secs_f64() * 1_000.0
}

pub enum ChatMessageRenderSnapshotCore {}

impl ChatMessageRenderSnapshotCore {
    const GROUPING_WINDOW: TimeInterval = 600.0;

    /// is_cancelled is none when no cancellation check is wanted
    pub fn make(
        seed: &ChatMessageRenderPreparationSeed,
        previous: Option<&ChatMessageRenderSnapshot>,
        next_content_version: &mut i64,
        traces_performance: bool,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Option<ChatMessageRenderPreparationResult> {
        let cancelled = || is_cancelled.is_some_and(|is_cancelled| is_cancelled());
        let build_started_at = started_at(traces_performance);
        let mut trace_metrics = ChatMessageRenderSnapshotTraceMetrics {
            message_count: seed.messages.len(),
            previous_entry_count: previous.map_or(0, |previous| previous.entries.len()),
            ..Default::default()
        };
        if cancelled() {
            return None;
        }
        let context_key = ChatMessageRenderContextKey::new(
            &seed.key,
            seed.calendar.start_of_day(&seed.now).time_interval_since_1970(),
            &seed.system_format_context,
        );
        if seed.messages.is_empty() {
            let snapshot_started_at = started_at(traces_performance);
            let snapshot = ChatMessageRenderSnapshot::new(
                seed.key.source_version,
                context_key,
                Vec::new(),
                HashMap::new(),
                HashMap::new(),
                Vec::new(),
                previous,
            );
            if traces_performance {
                trace_metrics.snapshot_index_duration_ms = elapsed_ms(snapshot_started_at);
                trace_metrics.total_duration_ms = elapsed_ms(build_started_at);
            }
            return Some(ChatMessageRenderPreparationResult {
                snapshot,
                metrics: trace_metrics,
            });
        }

        let context_started_at = started_at(traces_performance);
        let prepared_contexts = Self::message_contexts(
            &seed.messages,
            &seed.content_stamps_by_local_id,
            seed.search_highlight.as_ref(),
            seed.unread_separator_message_id.as_deref(),
            is_cancelled,
        )?;
        if traces_performance {
            trace_metrics.grouping_duration_ms = elapsed_ms(context_started_at);
        }
        let message_ids: Vec<String> = seed.messages.iter().map(|message| message.id.clone()).collect();
        let has_unique_message_ids = message_ids.iter().collect::<HashSet<_>>().len() == message_ids.len();
        let has_authoritative_content_stamps = prepared_contexts
            .values()
            .all(|context| context.content_stamp.is_some());
        let can_reuse_message_bundles = has_unique_message_ids
            && has_authoritative_content_stamps
            && previous.is_some_and(|previous| previous.context_key == context_key);
        let mut entries: Vec<Arc<ChatMessageRenderEntry>> =
            Vec::with_capacity(seed.messages.len() + (seed.messages.len() / 20).max(1));
        let mut entry_bundles_by_message_id: HashMap<String, Vec<Arc<ChatMessageRenderEntry>>> =
            HashMap::with_capacity(seed.messages.len());
        if cancelled() {
            return None;
        }

        let entry_loop_started_at = started_at(traces_performance);
        for (index, message) in seed.messages.iter().enumerate() {
            if index % 32 == 0 && cancelled() {
                return None;
            }
            let previous_context = previous.and_then(|previous| previous.message_contexts_by_id.get(&message.id));
            let message_context = prepared_contexts.get(&message.id);
            if can_reuse_message_bundles
                && previous_context == message_context
                && let Some(previous_bundle) =
                    previous.and_then(|previous| previous.entry_bundles_by_message_id.get(&message.id))
                && previous_bundle
                    .last()
                    .and_then(|entry| entry.message.as_ref())
                    .and_then(|message| message.link_preview.as_ref())
                    == message.link_preview.as_ref()
            {
                entry_bundles_by_message_id.insert(message.id.clone(), previous_bundle.clone());
                entries.extend(previous_bundle.iter().cloned());
                trace_metrics.reused_message_bundles += 1;
                continue;
            }

            trace_metrics.rebuilt_message_bundles += 1;
            let previous_entry = |id: &str| {
                previous
                    .and_then(|previous| previous.entries_by_id.get(id))
                    .map(Arc::as_ref)
            };
            let mut bundle: Vec<Arc<ChatMessageRenderEntry>> = Vec::with_capacity(3);
            let previous_message = index.checked_sub(1).and_then(|index| seed.messages.get(index));
            let date = Date::from_time_interval_since_1970(message.created_at);
            let day = seed.calendar.start_of_day(&date);
            let previous_day = previous_message.map(|previous_message| {
                seed.calendar
                    .start_of_day(&Date::from_time_interval_since_1970(previous_message.created_at))
            });
            if previous_day != Some(day) {
                let label = Self::format_day(&day, seed);
                let id = format!("date-{:?}-{}", day.time_interval_since_1970(), message.id);
                let previous_row = previous_entry(&id);
                bundle.push(Self::entry(
                    ChatMessageRenderEntryFields {
                        label,
                        day_start_ms: Some(RiseChatCalendarWindow::milliseconds(&day)),
                        ..ChatMessageRenderEntryFields::new(id, ChatMessageRenderEntryKind::DateSeparator)
                    },
                    &seed.key,
                    previous_row,
                    next_content_version,
                    &mut trace_metrics,
                    traces_performance,
                ));
            }

            if message_context.is_some_and(|context| context.opens_unread) {
                let id = format!("unread-{}", message.id);
                let previous_row = previous_entry(&id);
                bundle.push(Self::entry(
                    ChatMessageRenderEntryFields {
                        label: seed.localization.unread_messages.clone(),
                        ..ChatMessageRenderEntryFields::new(id, ChatMessageRenderEntryKind::UnreadSeparator)
                    },
                    &seed.key,
                    previous_row,
                    next_content_version,
                    &mut trace_metrics,
                    traces_performance,
                ));
            }

            if seed.key.group_by_topic {
                let topic_id = message.topic_id.as_ref().filter(|topic_id| !topic_id.is_empty());
                let previous_topic_id = previous_message
                    .and_then(|previous_message| previous_message.topic_id.as_ref())
                    .filter(|topic_id| !topic_id.is_empty());
                if index == 0 || topic_id != previous_topic_id {
                    let title = topic_id
                        .and_then(|topic_id| seed.key.topic_titles.get(topic_id))
                        .cloned()
                        .unwrap_or_else(|| {
                            if topic_id.is_none() {
                                seed.localization.general_topic.clone()
                            } else {
                                seed.localization.unknown_topic.clone()
                            }
                        });
                    let id = format!("topic-{}", message.id);
                    let previous_row = previous_entry(&id);
                    bundle.push(Self::entry(
                        ChatMessageRenderEntryFields {
                            label: title,
                            topic_id: topic_id.cloned(),
                            ..ChatMessageRenderEntryFields::new(id, ChatMessageRenderEntryKind::TopicSeparator)
                        },
                        &seed.key,
                        previous_row,
                        next_content_version,
                        &mut trace_metrics,
                        traces_performance,
                    ));
                }
            }

            let destructed_label = ChatViewLimitedDestruction::label(
                message,
                seed.key.account_user_id.as_deref(),
                &seed.key.view_limited_opened,
                &seed.key.view_limited_closed,
                &seed.now,
                &seed.localization.view_limited,
            );
            if let Some(destructed_label) = destructed_label {
                let id = format!("sys-{}", message.id);
                let previous_row = previous_entry(&id);
                bundle.push(Self::entry(
                    ChatMessageRenderEntryFields {
                        label: destructed_label,
                        message: Some(Arc::clone(message)),
                        source_content_stamp: seed.content_stamps_by_local_id.get(&message.id).copied(),
                        ..ChatMessageRenderEntryFields::new(id, ChatMessageRenderEntryKind::System)
                    },
                    &seed.key,
                    previous_row,
                    next_content_version,
                    &mut trace_metrics,
                    traces_performance,
                ));
            } else if message.content_type == MessageContentType::System {
                let id = format!("sys-{}", message.id);
                let previous_row = previous_entry(&id);
                bundle.push(Self::entry(
                    ChatMessageRenderEntryFields {
                        label: MessageSystemFormatter::render(message, &seed.system_format_context),
                        message: Some(Arc::clone(message)),
                        source_content_stamp: seed.content_stamps_by_local_id.get(&message.id).copied(),
                        ..ChatMessageRenderEntryFields::new(id, ChatMessageRenderEntryKind::System)
                    },
                    &seed.key,
                    previous_row,
                    next_content_version,
                    &mut trace_metrics,
                    traces_performance,
                ));
            } else {
                let id = format!("msg-{}", message.id);
                let previous_row = previous_entry(&id);
                bundle.push(Self::entry(
                    ChatMessageRenderEntryFields {
                        message: Some(Arc::clone(message)),
                        group: Some(Self::group_flags(
                            &seed.messages,
                            index,
                            seed.key.saved_messages_chat,
                            seed.key.account_user_id.as_deref(),
                            message_context,
                        )),
                        source_content_stamp: seed.content_stamps_by_local_id.get(&message.id).copied(),
                        search_highlight_ranges: message_context
                            .and_then(|context| context.search_highlight_ranges.clone()),
                        ..ChatMessageRenderEntryFields::new(id, ChatMessageRenderEntryKind::Message)
                    },
                    &seed.key,
                    previous_row,
                    next_content_version,
                    &mut trace_metrics,
                    traces_performance,
                ));
            }
            entries.extend(bundle.iter().cloned());
            entry_bundles_by_message_id.insert(message.id.clone(), bundle);
        }
        if traces_performance {
            trace_metrics.entry_loop_duration_ms = elapsed_ms(entry_loop_started_at);
        }
        trace_metrics.result_entry_count = entries.len();
        if cancelled() {
            return None;
        }

        let snapshot_started_at = started_at(traces_performance);
        let snapshot = ChatMessageRenderSnapshot::new(
            seed.key.source_version,
            context_key,
            message_ids,
            prepared_contexts,
            entry_bundles_by_message_id,
            entries,
            previous,
        );
        if traces_performance {
            trace_metrics.snapshot_index_duration_ms = elapsed_ms(snapshot_started_at);
            trace_metrics.total_duration_ms = elapsed_ms(build_started_at);
        }
        Some(ChatMessageRenderPreparationResult {
            snapshot,
            metrics: trace_metrics,
        })
    }

    fn message_contexts(
        messages: &[Arc<GetMessageMessage>],
        content_stamps_by_local_id: &HashMap<String, RiseMessagePresentationContentStamp>,
        search_highlight: Option<&ChatMessageSearchHighlight>,
        unread_separator_message_id: Option<&str>,
        is_cancelled: Option<&dyn Fn() -> bool>,
    ) -> Option<HashMap<String, ChatMessageRenderMessageContext>> {
        let mut result: HashMap<String, ChatMessageRenderMessageContext> = HashMap::with_capacity(messages.len());
        for (index, message) in messages.iter().enumerate() {
            if index % 32 == 0 && is_cancelled.is_some_and(|is_cancelled| is_cancelled()) {
                return None;
            }
            let previous = index.checked_sub(1).and_then(|index| messages.get(index));
            let next = messages.get(index + 1);
            result.insert(
                message.id.clone(),
                ChatMessageRenderMessageContext {
                    content_stamp: content_stamps_by_local_id.get(&message.id).copied(),
                    previous_id: previous.map(|previous| previous.id.clone()),
                    previous_content_stamp: previous
                        .and_then(|previous| content_stamps_by_local_id.get(&previous.id).copied()),
                    next_id: next.map(|next| next.id.clone()),
                    next_content_stamp: next.and_then(|next| content_stamps_by_local_id.get(&next.id).copied()),
                    search_highlight_ranges: search_highlight.and_then(|highlight| {
                        message
                            .matches_message_id(&highlight.message_id)
                            .then(|| highlight.ranges.clone())
                    }),
                    opens_unread: unread_separator_message_id == Some(message.id.as_str()),
                    precedes_unread: next.is_some_and(|next| unread_separator_message_id == Some(next.id.as_str())),
                },
            );
        }
        Some(result)
    }

    fn group_flags(
        messages: &[Arc<GetMessageMessage>],
        index: usize,
        saved_messages_chat: bool,
        viewer_user_id: Option<&str>,
        context: Option<&ChatMessageRenderMessageContext>,
    ) -> ChatMessageRenderEntryGroupFlags {
        let message = &messages[index];
        if message.content_type == MessageContentType::System {
            return ChatMessageRenderEntryGroupFlags {
                is_first: true,
                is_last: true,
            };
        }

        let previous = index.checked_sub(1).and_then(|index| messages.get(index));
        let next = messages.get(index + 1);
        let author = Self::grouping_sender_id(message, saved_messages_chat, viewer_user_id);
        let previous_author =
            previous.map(|previous| Self::grouping_sender_id(previous, saved_messages_chat, viewer_user_id));
        let next_author = next.map(|next| Self::grouping_sender_id(next, saved_messages_chat, viewer_user_id));
        let is_first = previous.is_none()
            || context.is_some_and(|context| context.opens_unread)
            || previous.is_some_and(|previous| previous.content_type == MessageContentType::System)
            || previous_author.as_ref() != Some(&author)
            || message.created_at - previous.map_or(0.0, |previous| previous.created_at) > Self::GROUPING_WINDOW;
        let is_last = next.is_none()
            || context.is_some_and(|context| context.precedes_unread)
            || next.is_some_and(|next| next.content_type == MessageContentType::System)
            || next_author.as_ref() != Some(&author)
            || next.map_or(0.0, |next| next.created_at) - message.created_at > Self::GROUPING_WINDOW;
        ChatMessageRenderEntryGroupFlags { is_first, is_last }
    }

    fn grouping_sender_id<'a>(
        message: &'a GetMessageMessage,
        saved_messages_chat: bool,
        viewer_user_id: Option<&str>,
    ) -> Cow<'a, str> {
        if !saved_messages_chat {
            return Cow::Borrowed(message.sender_id.as_str());
        }
        Cow::Owned(ChatSavedMessageIdentity::grouping_sender_id(message, true, viewer_user_id))
    }

    fn entry(
        fields: ChatMessageRenderEntryFields,
        request_key: &ChatMessageRenderRequestKey,
        previous: Option<&ChatMessageRenderEntry>,
        next_content_version: &mut i64,
        trace_metrics: &mut ChatMessageRenderSnapshotTraceMetrics,
        traces_performance: bool,
    ) -> Arc<ChatMessageRenderEntry> {
        let ChatMessageRenderEntryFields {
            id,
            kind,
            label,
            topic_id,
            day_start_ms,
            message,
            group,
            source_content_stamp,
            search_highlight_ranges,
        } = fields;
        let has_matching_prefix = previous
            .is_some_and(|previous| previous.kind == kind && previous.label == label && previous.topic_id == topic_id);
        let messages_match = match previous.filter(|_| has_matching_prefix) {
            Some(previous) => {
                if let Some(source_content_stamp) = source_content_stamp {
                    previous.source_content_stamp == Some(source_content_stamp)
                        && previous.message.as_ref().and_then(|message| message.link_preview.as_ref())
                            == message.as_ref().and_then(|message| message.link_preview.as_ref())
                } else {
                    let comparison_started_at = started_at(traces_performance);
                    let is_match = previous.message == message;
                    if traces_performance {
                        trace_metrics.optional_message_comparisons += 1;
                        if previous.message.is_some() || message.is_some() {
                            trace_metrics.deep_message_comparisons += 1;
                        }
                        trace_metrics.message_comparison_duration_ms += elapsed_ms(comparison_started_at);
                    }
                    is_match
                }
            }
            None => false,
        };

        let reusable_version = previous
            .filter(|previous| {
                has_matching_prefix
                    && messages_match
                    && previous.group == group
                    && previous.search_highlight_ranges == search_highlight_ranges
            })
            .map(|previous| previous.content_version);
        let content_version = match reusable_version {
            Some(content_version) => {
                if traces_performance {
                    trace_metrics.reused_content_versions += 1;
                }
                content_version
            }
            None => {
                *next_content_version = next_content_version.wrapping_add(1);
                if traces_performance {
                    trace_metrics.changed_content_versions += 1;
                }
                *next_content_version
            }
        };

        let prepared_geometry_version = RiseMessageGeometryFingerprintPolicy::version(
            &id,
            kind,
            &label,
            topic_id.as_deref(),
            message.as_deref(),
            group,
            source_content_stamp,
            request_key,
        );

        let plain_text_eligibility = match message.as_deref() {
            Some(message)
                if kind == ChatMessageRenderEntryKind::Message
                    && RiseAsyncTextMessageEligibility::supports_message_content(message) =>
            {
                let cached = previous
                    .filter(|previous| {
                        previous
                            .message
                            .as_ref()
                            .is_some_and(|previous_message| previous_message.text == message.text)
                    })
                    .and_then(|previous| previous.plain_text_eligibility);
                Some(cached.unwrap_or_else(|| RiseAsyncTextMessageEligibility::supports_plain_text(&message.text)))
            }
            _ => None,
        };

        Arc::new(ChatMessageRenderEntry::new(
            id,
            kind,
            label,
            topic_id,
            day_start_ms,
            message,
            group,
            source_content_stamp,
            prepared_geometry_version,
            search_highlight_ranges,
            plain_text_eligibility,
            content_version,
        ))
    }

    fn format_day(day: &Date, seed: &ChatMessageRenderPreparationSeed) -> String {
        let band = RiseFormat::calendar_band(day, &seed.now, &seed.calendar);
        match band {
            CalendarBand::Today => seed.localization.today.clone(),
            CalendarBand::Yesterday => seed.localization.yesterday.clone(),
            CalendarBand::ThisYear | CalendarBand::Older => {
                let pattern = if band == CalendarBand::ThisYear {
                    &seed.localization.day_month_pattern
                } else {
                    &seed.localization.day_month_year_pattern
                };
                RiseFormat::date_formatter(
                    pattern,
                    seed.locale.identifier(),
                    &seed.calendar,
                    Some(&seed.time_zone),
                )
                .string_from(day)
            }
        }
    }
}

#[cfg(debug_assertions)]
pub enum ChatMessageRenderSnapshotBuilder {}

#[cfg(debug_assertions)]
static BUILDER_NEXT_CONTENT_VERSION: Mutex<i64> = Mutex::new(0);

#[cfg(debug_assertions)]
impl ChatMessageRenderSnapshotBuilder {
    #[allow(clippy::too_many_arguments)]
    pub fn make(
        messages: Vec<Arc<GetMessageMessage>>,
        source_version: i64,
        content_stamps_by_local_id: HashMap<String, RiseMessagePresentationContentStamp>,
        group_by_topic: bool,
        topic_titles: HashMap<String, String>,
        saved_messages_chat: bool,
        previous: Option<&ChatMessageRenderSnapshot>,
        cx: &App,
    ) -> ChatMessageRenderSnapshot {
        let traces_performance = ChatPerformanceTrace::shared().is_enabled();
        let chat_id = messages.first().map(|message| message.chat_id.clone()).unwrap_or_default();
        let seed = ChatMessageRenderPreparationSeed::make(
            messages,
            &chat_id,
            source_version,
            content_stamps_by_local_id,
            group_by_topic,
            topic_titles,
            Locale::app_locale(),
            &Calendar::current(),
            TimeZone::current(),
            SaiAccountCacheScope::active_user_id(),
            SaiAccountCacheScope::active_epoch(),
            saved_messages_chat,
            None,
            None,
            HashSet::new(),
            HashSet::new(),
            cx,
        );
        let result = ChatMessageRenderSnapshotCore::make(
            &seed,
            previous,
            &mut BUILDER_NEXT_CONTENT_VERSION.lock(),
            traces_performance,
            None,
        );
        let Some(result) = result else {
            return ChatMessageRenderSnapshot::new(
                source_version,
                ChatMessageRenderContextKey::new(
                    &seed.key,
                    seed.calendar.start_of_day(&seed.now).time_interval_since_1970(),
                    &seed.system_format_context,
                ),
                Vec::new(),
                HashMap::new(),
                HashMap::new(),
                Vec::new(),
                previous,
            );
        };
        if traces_performance {
            ChatPerformanceTrace::shared().record("snapshot.build.detail", result.metrics.trace_fields());
        }
        result.snapshot
    }
}
