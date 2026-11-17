use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::{tr, tr_args, FormatArg};

const HISTORY_DATE_FORMAT: &str = "yyyy-MM-dd HH:mm:ss.SSS";
const MISSING_VALUE: &str = "\u{2014}";
const HISTORY_SEPARATOR: &str = "\n\n\u{2014} \u{2014} \u{2014}\n\n";

thread_local! {
    static HISTORY_FORMATTER: DateFormatter = {
        let mut formatter = DateFormatter::new();
        formatter.set_date_format(HISTORY_DATE_FORMAT);
        formatter
    };
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsDeferredDebugFifoCard {
    pub id: Uuid,
    pub service: String,
    pub method: String,
    pub body_json: String,
    pub cache_key: Option<String>,
    pub enqueued_at: Date,
    pub position_from_wire: i64,
    pub total_in_fifo: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsDeferredDebugDiskCard {
    pub id: String,
    pub service: String,
    pub method: String,
    pub body_json: String,
    pub cache_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SaiWsDeferredDebugOutcome {
    Sent,
    SendFailed(String),
    Cancelled(String),
    ColdReplaySent,
    ColdReplayFailed(String),
    AckDebtAcknowledged,
    AckDebtTimedOut,
    AckDebtColdReplaySent,
    AckDebtColdReplayFailed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsDeferredDebugHistoryRow {
    pub id: Uuid,
    pub service: String,
    pub method: String,
    pub body_json: String,
    pub cache_key: Option<String>,
    pub enqueued_at: Option<Date>,
    pub processed_at: Date,
    pub wait_seconds: Option<TimeInterval>,
    pub outcome: SaiWsDeferredDebugOutcome,
}

impl SaiWsDeferredDebugHistoryRow {
    pub fn outcome_label(&self) -> String {
        match &self.outcome {
            SaiWsDeferredDebugOutcome::Sent => tr("sai_ws_deferred_debug_outcome_sent"),
            SaiWsDeferredDebugOutcome::SendFailed(message) => {
                with_message("sai_ws_deferred_debug_outcome_send_failed", message)
            }
            SaiWsDeferredDebugOutcome::Cancelled(message) => {
                with_message("sai_ws_deferred_debug_outcome_cancelled", message)
            }
            SaiWsDeferredDebugOutcome::ColdReplaySent => {
                tr("sai_ws_deferred_debug_outcome_cold_replay_sent")
            }
            SaiWsDeferredDebugOutcome::ColdReplayFailed(message) => {
                with_message("sai_ws_deferred_debug_outcome_cold_replay_failed", message)
            }
            SaiWsDeferredDebugOutcome::AckDebtAcknowledged => {
                tr("sai_ws_deferred_debug_outcome_ack_debt_acknowledged")
            }
            SaiWsDeferredDebugOutcome::AckDebtTimedOut => {
                tr("sai_ws_deferred_debug_outcome_ack_debt_timed_out")
            }
            SaiWsDeferredDebugOutcome::AckDebtColdReplaySent => {
                tr("sai_ws_deferred_debug_outcome_ack_debt_cold_replay_sent")
            }
            SaiWsDeferredDebugOutcome::AckDebtColdReplayFailed(message) => {
                with_message("sai_ws_deferred_debug_outcome_ack_debt_cold_replay_failed", message)
            }
        }
    }

    pub fn copy_block_text(&self) -> String {
        let wait = match self.wait_seconds {
            Some(seconds) => format!("{seconds:.3} s"),
            None => MISSING_VALUE.to_owned(),
        };
        let enqueued = self
            .enqueued_at
            .map(|date| Self::formatted(&date))
            .unwrap_or_else(|| MISSING_VALUE.to_owned());
        let processed = Self::formatted(&self.processed_at);
        let cache_key = self.cache_key.as_deref().unwrap_or(MISSING_VALUE);
        format!(
            "{}.{}\noutcome: {}\nenqueued: {enqueued}\nprocessed: {processed}\nwait: {wait}\ncacheKey: {cache_key}\n\n{}",
            self.service,
            self.method,
            self.outcome_label(),
            self.body_json,
        )
    }

    fn formatted(date: &Date) -> String {
        HISTORY_FORMATTER.with(|formatter| formatter.string_from(date))
    }
}

fn with_message(key: &str, message: &str) -> String {
    tr_args(key, &[FormatArg::from(message)])
}

/// one row of the fifo snapshot the outbound coordinator hands over, oldest first
#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsDeferredDebugFifoRow {
    pub id: Uuid,
    pub enqueued_at: Date,
    pub service: String,
    pub method: String,
    pub body_json: String,
    pub cache_key: Option<String>,
}

/// one persisted row of a snapshot, the disk backlog or the ack debt
#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsDeferredDebugDiskRow {
    pub id: String,
    pub service: String,
    pub method: String,
    pub body_json: String,
    pub cache_key: Option<String>,
}

impl From<SaiWsDeferredDebugDiskRow> for SaiWsDeferredDebugDiskCard {
    fn from(row: SaiWsDeferredDebugDiskRow) -> Self {
        Self {
            id: row.id,
            service: row.service,
            method: row.method,
            body_json: row.body_json,
            cache_key: row.cache_key,
        }
    }
}

/// deferred debug store, an entity where every published write notifies observers
pub struct SaiWsDeferredDebugStore {
    fifo_cards: Vec<SaiWsDeferredDebugFifoCard>,
    disk_backlog_cards: Vec<SaiWsDeferredDebugDiskCard>,
    ack_debt_disk_cards: Vec<SaiWsDeferredDebugDiskCard>,
    history: Vec<SaiWsDeferredDebugHistoryRow>,
    max_history: usize,
}

impl SaiWsDeferredDebugStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        Self {
            fifo_cards: Vec::new(),
            disk_backlog_cards: Vec::new(),
            ack_debt_disk_cards: Vec::new(),
            history: Vec::new(),
            max_history: 200,
        }
    }

    pub fn fifo_cards(&self) -> &[SaiWsDeferredDebugFifoCard] {
        &self.fifo_cards
    }

    pub fn disk_backlog_cards(&self) -> &[SaiWsDeferredDebugDiskCard] {
        &self.disk_backlog_cards
    }

    pub fn ack_debt_disk_cards(&self) -> &[SaiWsDeferredDebugDiskCard] {
        &self.ack_debt_disk_cards
    }

    pub fn history(&self) -> &[SaiWsDeferredDebugHistoryRow] {
        &self.history
    }

    pub fn apply_snapshot(
        &mut self,
        fifo_oldest_first: Vec<SaiWsDeferredDebugFifoRow>,
        disk_not_in_fifo: Vec<SaiWsDeferredDebugDiskRow>,
        cx: &mut Context<Self>,
    ) {
        let total = i64::try_from(fifo_oldest_first.len()).unwrap_or(i64::MAX);
        self.fifo_cards = fifo_oldest_first
            .into_iter()
            .zip(1_i64..)
            .map(|(row, position)| SaiWsDeferredDebugFifoCard {
                id: row.id,
                service: row.service,
                method: row.method,
                body_json: row.body_json,
                cache_key: row.cache_key,
                enqueued_at: row.enqueued_at,
                position_from_wire: position,
                total_in_fifo: total,
            })
            .collect();
        self.disk_backlog_cards = disk_not_in_fifo.into_iter().map(Into::into).collect();
        cx.notify();
    }

    pub fn apply_ack_debt_snapshot(
        &mut self,
        rows: Vec<SaiWsDeferredDebugDiskRow>,
        cx: &mut Context<Self>,
    ) {
        self.ack_debt_disk_cards = rows.into_iter().map(Into::into).collect();
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_ack_debt_history(
        &mut self,
        service: String,
        method: String,
        body_json: String,
        cache_key: Option<String>,
        enqueued_at: Option<Date>,
        processed_at: Date,
        outcome: SaiWsDeferredDebugOutcome,
        cx: &mut Context<Self>,
    ) {
        self.record_resolved(
            Uuid::new_v4(),
            service,
            method,
            body_json,
            cache_key,
            enqueued_at,
            processed_at,
            outcome,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_resolved(
        &mut self,
        id: Uuid,
        service: String,
        method: String,
        body_json: String,
        cache_key: Option<String>,
        enqueued_at: Option<Date>,
        processed_at: Date,
        outcome: SaiWsDeferredDebugOutcome,
        cx: &mut Context<Self>,
    ) {
        let wait_seconds = enqueued_at.map(|enqueued| processed_at.time_interval_since(&enqueued).max(0.0));
        let row = SaiWsDeferredDebugHistoryRow {
            id,
            service,
            method,
            body_json,
            cache_key,
            enqueued_at,
            processed_at,
            wait_seconds,
            outcome,
        };
        self.history.insert(0, row);
        if self.history.len() > self.max_history {
            self.history.truncate(self.max_history);
        }
        cx.notify();
    }

    pub fn remove_history(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.history.retain(|row| row.id != id);
        cx.notify();
    }

    pub fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.history.clear();
        cx.notify();
    }

    pub fn copy_all_history_text(&self) -> String {
        self.history
            .iter()
            .map(SaiWsDeferredDebugHistoryRow::copy_block_text)
            .collect::<Vec<_>>()
            .join(HISTORY_SEPARATOR)
    }
}
