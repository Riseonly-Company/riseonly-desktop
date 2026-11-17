use std::time::Duration;

use gpui::{Context, Entity, Point, WeakEntity};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_canon::rise_ledger::{HistoryDirection, OwnerKind, ReferenceKind, TransactionKind};

use crate::modules::sparks::engine::rise_sparks_engine_models::RiseSparksHistoryItemDTO;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SparksTarget {
    Message { chat_id: String, message_id: String },
    Post { post_id: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SparksBurst {
    pub id: Uuid,
    pub center: Point<f32>,
    pub side: f32,
}

impl SparksBurst {
    pub const ANIMATION: &'static str = "sparks_burst";
    pub const CANVAS_PER_ICON: f32 = 8.0;
    pub const LIFETIME: Duration = Duration::from_secs(1);
    pub const MAXIMUM_IN_FLIGHT: usize = 6;
    pub const AFTER_SHEET: Duration = Duration::from_millis(260);
    pub const FRESHNESS: TimeInterval = 0.3;
    pub const SIGNALS_PRUNE_FLOOR: usize = 64;

    pub fn allows_decorative_motion() -> bool {
        !RiseEnergySavingStore::snapshot_reduces(RiseEnergySavingFeature::InterfaceEffects)
    }
}

pub struct SparksBurstSignal {
    tick: i64,
    fired_at: TimeInterval,
}

impl SparksBurstSignal {
    pub fn new() -> Self {
        SparksBurstSignal { tick: 0, fired_at: 0.0 }
    }

    pub fn tick(&self) -> i64 {
        self.tick
    }

    pub fn fired_at(&self) -> TimeInterval {
        self.fired_at
    }

    pub fn is_fresh(&self) -> bool {
        RiseClock::monotonic().as_secs_f64() - self.fired_at < SparksBurst::FRESHNESS
    }

    pub fn fire(&mut self, cx: &mut Context<Self>) {
        self.fired_at = RiseClock::monotonic().as_secs_f64();
        self.tick = self.tick.wrapping_add(1);
        cx.notify();
    }
}

impl Default for SparksBurstSignal {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SparksWeakBurstSignal {
    pub signal: WeakEntity<SparksBurstSignal>,
}

impl SparksWeakBurstSignal {
    pub fn new(signal: &Entity<SparksBurstSignal>) -> Self {
        SparksWeakBurstSignal { signal: signal.downgrade() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SparksSendSession {
    pub id: Uuid,
    pub account_id: String,
    pub target: SparksTarget,
    pub amount: i64,
    pub idempotency_key: String,
    pub anonymous: bool,
    pub toast_id: Option<Uuid>,
    pub recipient_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SparksOffersPhase {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SparksPurchasePhase {
    Idle,
    Purchasing { product_id: String },
    Pending,
    ContinueOnPhone { product_id: String },
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SparksHistoryRowKind {
    Purchase,
    Refund,
    RefundReversed,
    GiftPurchase,
    MarketPurchase,
    SentPost,
    SentMessage,
    ReceivedPost,
    ReceivedMessage,
    AdminGrant,
    AdminRevoke,
    EarnedTransfer,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SparksHistoryRow {
    pub id: String,
    pub kind: SparksHistoryRowKind,
    pub is_incoming: bool,
    pub amount: i64,
    pub counterparty_name: Option<String>,
    pub counterparty_logo: Option<String>,
    pub counterparty_user_id: Option<String>,
    pub counterparty_tag: Option<String>,
    pub created_at: Option<Date>,
    pub post_id: Option<String>,
}

impl SparksHistoryRow {
    pub fn new(item: &RiseSparksHistoryItemDTO) -> Self {
        let is_incoming = HistoryDirection::from_slug(&item.direction) == Some(HistoryDirection::Incoming);
        let hidden = item.counterparty.as_ref().is_none_or(|counterparty| counterparty.is_hidden);
        let name = item
            .counterparty
            .as_ref()
            .map(|counterparty| trimming_whitespaces_and_newlines(&counterparty.name).to_string());
        let name_is_empty = name.as_deref().is_none_or(str::is_empty);
        let counterparty_name = if hidden || name_is_empty { None } else { name };
        let counterparty_logo = if hidden {
            None
        } else {
            item.counterparty
                .as_ref()
                .and_then(|counterparty| if counterparty.logo.is_empty() { None } else { Some(counterparty.logo.clone()) })
        };
        let user_party = if hidden || counterparty_name.is_none() {
            None
        } else {
            item.counterparty
                .as_ref()
                .filter(|counterparty| counterparty.kind == OwnerKind::User.slug() && !counterparty.id.is_empty())
        };
        let counterparty_user_id = user_party.map(|counterparty| counterparty.id.clone());
        let counterparty_tag = user_party.and_then(|counterparty| {
            if counterparty.tag.is_empty() { None } else { Some(counterparty.tag.clone()) }
        });
        let created_at = if item.created_at_ms > 0 { Some(Date::from_ms(item.created_at_ms)) } else { None };
        let reference = item.reference.as_ref().and_then(|reference| ReferenceKind::from_slug(&reference.kind));
        let reference_id = item
            .reference
            .as_ref()
            .map(|reference| trimming_whitespaces_and_newlines(&reference.id).to_string());
        let has_reference_id = reference_id.as_deref().is_some_and(|id| !id.is_empty());
        let post_id = if reference == Some(ReferenceKind::Post) && has_reference_id { reference_id } else { None };
        let kind = match TransactionKind::from_slug(&item.kind) {
            Some(TransactionKind::Purchase) => SparksHistoryRowKind::Purchase,
            Some(TransactionKind::PurchaseRefund) => SparksHistoryRowKind::Refund,
            Some(TransactionKind::PurchaseRefundReversal) => SparksHistoryRowKind::RefundReversed,
            Some(TransactionKind::GiftPurchase) => SparksHistoryRowKind::GiftPurchase,
            Some(TransactionKind::MarketPurchase) => SparksHistoryRowKind::MarketPurchase,
            Some(TransactionKind::AdminGrant) => SparksHistoryRowKind::AdminGrant,
            Some(TransactionKind::AdminRevoke) => SparksHistoryRowKind::AdminRevoke,
            Some(TransactionKind::EarnedTransfer) => SparksHistoryRowKind::EarnedTransfer,
            Some(TransactionKind::Tip) => match (reference, is_incoming) {
                (Some(ReferenceKind::Post), true) => SparksHistoryRowKind::ReceivedPost,
                (Some(ReferenceKind::Post), false) => SparksHistoryRowKind::SentPost,
                (Some(ReferenceKind::Message), true) => SparksHistoryRowKind::ReceivedMessage,
                (Some(ReferenceKind::Message), false) => SparksHistoryRowKind::SentMessage,
                _ => SparksHistoryRowKind::Other,
            },
            _ => SparksHistoryRowKind::Other,
        };
        SparksHistoryRow {
            id: item.id.clone(),
            kind,
            is_incoming,
            amount: item.amount,
            counterparty_name,
            counterparty_logo,
            counterparty_user_id,
            counterparty_tag,
            created_at,
            post_id,
        }
    }

    pub fn title_key(&self) -> &'static str {
        match self.kind {
            SparksHistoryRowKind::Purchase => "sparks_history_purchase",
            SparksHistoryRowKind::Refund => "sparks_history_purchase_refund",
            SparksHistoryRowKind::RefundReversed => "sparks_history_refund_reversed",
            SparksHistoryRowKind::GiftPurchase => "sparks_history_gift_purchase",
            SparksHistoryRowKind::MarketPurchase => "sparks_history_market_purchase",
            SparksHistoryRowKind::SentPost => "sparks_history_sent_post",
            SparksHistoryRowKind::SentMessage => "sparks_history_sent_message",
            SparksHistoryRowKind::ReceivedPost => "sparks_history_received_post",
            SparksHistoryRowKind::ReceivedMessage => "sparks_history_received_message",
            SparksHistoryRowKind::AdminGrant => "sparks_history_admin_grant",
            SparksHistoryRowKind::AdminRevoke => "sparks_history_admin_revoke",
            SparksHistoryRowKind::EarnedTransfer => "sparks_history_earned_transfer",
            SparksHistoryRowKind::Other => "sparks_history_other",
        }
    }
}
