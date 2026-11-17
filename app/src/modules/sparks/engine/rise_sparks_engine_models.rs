use rise_canon::RiseLimits;
use rise_canon::rise_proto::ledger::{
    ChannelEarnings, Counterparty, ListSparksHistoryResponse, Reference, SparksBalance, SparksEarnings,
    SparksHistoryItem,
};
use rise_canon::rise_proto::message::ProtoSendMessageSparksResponse;
use rise_canon::rise_proto::post::GatewaySendPostSparksResponse;
use rise_canon::rise_proto::subscription::VerifySparksPurchaseResponse;
use rise_canon::rise_view::ledger::{Balance, Earnings, Transfer};
use rise_canon::rise_view::message::{SparkSender, SparkUser, Sparks};
use rise_canon::rise_view::post::{PostSparks, SparksSender, SparksUser};
use serde::{Deserialize, Serialize};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

pub type RiseSparksBalanceDTO = SparksBalance;
pub type RiseSparksBalanceResponseDTO = Balance;
pub type RiseSparksCounterpartyDTO = Counterparty;
pub type RiseSparksReferenceDTO = Reference;
pub type RiseSparksHistoryItemDTO = SparksHistoryItem;
pub type RiseSparksHistoryResponseDTO = ListSparksHistoryResponse;
/// sparks received by one chat the account owns, a channel sparks sit on its own account
pub type RiseSparksChannelEarningsDTO = ChannelEarnings;
/// what the account received and may withdraw to its balance, own earned account and owned channels
pub type RiseSparksEarningsDTO = SparksEarnings;
pub type RiseSparksEarningsResponseDTO = Earnings;
/// answer of a withdrawal, what moved and the balance after it and what is still received
pub type RiseSparksTransferResponseDTO = Transfer;
pub type RiseSparksPurchaseResponseDTO = VerifySparksPurchaseResponse;

/// zero balance constant
pub trait SparksBalanceExt {
    fn zero() -> Self;
}

impl SparksBalanceExt for SparksBalance {
    fn zero() -> Self {
        RiseSparksBalanceDTO::default()
    }
}

pub trait SparksHistoryItemIdentifiable {
    fn id(&self) -> String;
}

impl SparksHistoryItemIdentifiable for SparksHistoryItem {
    fn id(&self) -> String {
        self.id.clone()
    }
}

pub trait ChannelEarningsIdentifiable {
    fn id(&self) -> String;
}

impl ChannelEarningsIdentifiable for ChannelEarnings {
    fn id(&self) -> String {
        self.chat_id.clone()
    }
}

/// balance with the journal position it reflects per account purpose,
/// events can come out of order so only a later transaction replaces one
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiseSparksBalanceState {
    pub balance: RiseSparksBalanceDTO,
    #[serde(rename = "readUpdatedAtMs")]
    pub read_updated_at_ms: i64,
    #[serde(rename = "spendableTransactionID")]
    pub spendable_transaction_id: i64,
    #[serde(rename = "earnedTransactionID")]
    pub earned_transaction_id: i64,
}

impl RiseSparksBalanceState {
    pub fn empty() -> Self {
        RiseSparksBalanceState {
            balance: SparksBalance::zero(),
            read_updated_at_ms: 0,
            spendable_transaction_id: 0,
            earned_transaction_id: 0,
        }
    }

    pub fn replace(&mut self, server: RiseSparksBalanceDTO) {
        if server.updated_at_ms < self.read_updated_at_ms {
            return;
        }
        self.read_updated_at_ms = server.updated_at_ms;
        self.balance = server;
    }

    /// applies one balance changed payload, true if anything moved
    pub fn apply(&mut self, event: &RiseSparksBalanceEvent) -> bool {
        if event.updated_at_ms < self.read_updated_at_ms {
            return false;
        }
        let mut next = self.balance.clone();
        let mut changed = false;
        if let Some(spendable) = event.spendable
            && event.transaction_id > self.spendable_transaction_id
        {
            self.spendable_transaction_id = event.transaction_id;
            let held = event.held.unwrap_or(next.held);
            next.spendable = spendable;
            next.held = held;
            next.available = spendable - held;
            next.spends_blocked = event.spends_blocked.unwrap_or(spendable < 0);
            next.updated_at_ms = next.updated_at_ms.max(event.updated_at_ms);
            changed = true;
        }
        if let Some(earned) = event.earned
            && event.transaction_id > self.earned_transaction_id
        {
            self.earned_transaction_id = event.transaction_id;
            next.earned = earned;
            next.updated_at_ms = next.updated_at_ms.max(event.updated_at_ms);
            changed = true;
        }
        if changed {
            self.balance = next;
        }
        changed
    }
}

/// balance changed event from the ledger,
/// absolute balances after the transaction for the touched purposes
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseSparksBalanceEvent {
    pub transaction_id: i64,
    pub kind: String,
    pub spendable: Option<i64>,
    pub earned: Option<i64>,
    pub held: Option<i64>,
    pub spends_blocked: Option<bool>,
    pub updated_at_ms: i64,
}

/// one history page per filter, capped by the canon page ceiling times the retention factor
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiseSparksHistoryPage {
    pub items: Vec<RiseSparksHistoryItemDTO>,
    #[serde(rename = "hasMore")]
    pub has_more: bool,
}

impl RiseSparksHistoryPage {
    pub fn empty() -> Self {
        RiseSparksHistoryPage { items: Vec::new(), has_more: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RiseSparksLoadPhase {
    Idle,
    Loading,
    Loaded,
    Failed(String),
}

pub enum RiseSparksLimits {}

impl RiseSparksLimits {
    /// server clamp of one history page, asking for more just ends pages early
    pub fn history_page_size() -> i64 {
        RiseLimits::service_value("ledger_history_page_max_items")
    }

    /// largest single send the ledger accepts
    pub fn tip_max_amount() -> i64 {
        RiseLimits::service_value("sparks_tip_max_amount")
    }

    pub fn top_senders_page_size() -> i64 {
        RiseLimits::service_value("ledger_reference_initiators_page_max")
    }
}

/// top list sender, the two canon types share one shape and are read into one
/// user is none for an anonymous sender other than the viewer
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStoredSparksSenderUser {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_official: bool,
}

impl RiseStoredSparksSenderUser {
    pub fn new(id: String, name: String, tag: String, logo: String, is_premium: bool, is_official: bool) -> Self {
        RiseStoredSparksSenderUser { id, name, tag, logo, is_premium, is_official }
    }

    pub fn from_post_canon(user: &SparksUser) -> Self {
        Self::new(
            user.id.clone(),
            user.name.clone(),
            user.tag.clone(),
            user.logo.clone(),
            user.is_premium,
            user.is_official,
        )
    }

    pub fn from_message_canon(user: &SparkUser) -> Self {
        Self::new(
            user.id.clone(),
            user.name.clone(),
            user.tag.clone(),
            user.logo.clone(),
            user.is_premium,
            user.is_official,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStoredSparksSender {
    pub user: Option<RiseStoredSparksSenderUser>,
    pub amount: i64,
    pub is_anonymous: bool,
    pub is_me: bool,
}

impl RiseStoredSparksSender {
    pub fn new(user: Option<RiseStoredSparksSenderUser>, amount: i64, is_anonymous: bool, is_me: bool) -> Self {
        RiseStoredSparksSender { user, amount, is_anonymous, is_me }
    }

    pub fn from_post_canon(sender: &SparksSender) -> Self {
        Self::new(
            sender.user.as_ref().map(RiseStoredSparksSenderUser::from_post_canon),
            sender.amount,
            sender.is_anonymous,
            sender.is_me,
        )
    }

    pub fn from_message_canon(sender: &SparkSender) -> Self {
        Self::new(
            sender.user.as_ref().map(RiseStoredSparksSenderUser::from_message_canon),
            sender.amount,
            sender.is_anonymous,
            sender.is_me,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStoredSparksTop {
    pub sparks_total: i64,
    pub my_sparks: i64,
    pub senders_count: i64,
    pub senders: Vec<RiseStoredSparksSender>,
    pub has_more: bool,
    pub sparks_hidden: bool,
}

impl RiseStoredSparksTop {
    pub fn from_post_canon(top: &PostSparks) -> Self {
        RiseStoredSparksTop {
            sparks_total: top.sparks_total,
            my_sparks: top.my_sparks,
            senders_count: top.senders_count,
            senders: top.senders.iter().map(RiseStoredSparksSender::from_post_canon).collect(),
            has_more: top.has_more,
            sparks_hidden: top.sparks_hidden,
        }
    }

    pub fn from_message_canon(top: &Sparks) -> Self {
        RiseStoredSparksTop {
            sparks_total: top.sparks_total,
            my_sparks: top.my_sparks,
            senders_count: top.senders_count,
            senders: top.senders.iter().map(RiseStoredSparksSender::from_message_canon).collect(),
            has_more: top.has_more,
            sparks_hidden: false,
        }
    }
}

/// answer of a send to a message or post, target totals after posting and the sender spendable balance
/// absent on a replay of the same key
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStoredSparksSendResult {
    pub sparks_total: i64,
    pub my_sparks: i64,
    pub replayed: bool,
    pub spendable_after: Option<i64>,
    pub sparks_hidden: bool,
}

impl RiseStoredSparksSendResult {
    pub fn from_post_canon(result: &GatewaySendPostSparksResponse) -> Self {
        RiseStoredSparksSendResult {
            sparks_total: result.sparks_total,
            my_sparks: result.my_sparks,
            replayed: result.replayed,
            spendable_after: result.spendable_after,
            sparks_hidden: result.sparks_hidden,
        }
    }

    pub fn from_message_canon(result: &ProtoSendMessageSparksResponse) -> Self {
        RiseStoredSparksSendResult {
            sparks_total: result.sparks_total,
            my_sparks: result.my_sparks,
            replayed: result.replayed,
            spendable_after: result.spendable_after,
            sparks_hidden: false,
        }
    }
}
