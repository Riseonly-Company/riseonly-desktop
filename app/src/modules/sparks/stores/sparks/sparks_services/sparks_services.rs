use std::collections::{HashMap, HashSet};

use gpui::{App, AppContext, Context, Entity, Subscription};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_canon::rise_ledger::{self, HistoryFilter as RiseLedgerHistoryFilter};

use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::core::lib::store_price::StorePriceBook;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::sparks::engine::rise_sparks_engine_models::{
    RiseSparksBalanceDTO, RiseSparksChannelEarningsDTO, RiseSparksEarningsDTO, RiseSparksLimits, RiseSparksLoadPhase,
    RiseStoredSparksSender, RiseStoredSparksSenderUser, RiseStoredSparksTop,
};
use crate::modules::sparks::engine::rise_sparks_presentation::{RiseSparksPresentation, RiseSparksPresentationSnapshot};
use crate::modules::sparks::engine::rise_sparks_purchases::RiseSparksPackOffer;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::{
    SparksBurst, SparksBurstSignal, SparksHistoryRow, SparksOffersPhase, SparksPurchasePhase, SparksSendSession,
    SparksTarget, SparksWeakBurstSignal,
};

pub struct SparksServicesStore {
    pub selected_filter: RiseLedgerHistoryFilter,

    pub is_purchase_sheet_presented: bool,
    pub needed_amount: Option<i64>,
    pub offers: Vec<RiseSparksPackOffer>,
    pub offers_phase: SparksOffersPhase,
    pub purchase_phase: SparksPurchasePhase,

    pub is_success_sheet_presented: bool,
    pub success_amount: i64,

    pub is_send_sheet_presented: bool,
    pub send_view_only: bool,
    pub send_target: Option<SparksTarget>,
    pub send_amount: i64,
    pub send_anonymous: bool,
    pub top: Option<RiseStoredSparksTop>,
    pub top_phase: RiseSparksLoadPhase,

    pub sessions: HashMap<Uuid, SparksSendSession>,
    pub active_session_id: Option<Uuid>,

    pub bursts: Vec<SparksBurst>,
    burst_signals: HashMap<SparksTarget, SparksWeakBurstSignal>,
    burst_signals_prune_at: usize,
    direct_chats_refusing_sparks: HashSet<String>,

    snapshot: RiseSparksPresentationSnapshot,
    _presentation: Subscription,
}

impl SparksServicesStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let book = StorePriceBook::shared(cx);
        cx.observe(&book, |store, book, cx| {
            let book = book.read(cx);
            for offer in &mut store.offers {
                offer.display_price = book.text(&offer.product_id);
            }
            cx.notify();
        })
        .detach();
        let presentation = RiseSparksPresentation::shared(cx);
        let snapshot = presentation.read(cx).snapshot().clone();
        let subscription = cx.observe(&presentation, |store, presentation, cx| {
            store.snapshot = presentation.read(cx).snapshot().clone();
            cx.notify();
        });
        Self {
            selected_filter: RiseLedgerHistoryFilter::All,
            is_purchase_sheet_presented: false,
            needed_amount: None,
            offers: Vec::new(),
            offers_phase: SparksOffersPhase::Idle,
            purchase_phase: SparksPurchasePhase::Idle,
            is_success_sheet_presented: false,
            success_amount: 0,
            is_send_sheet_presented: false,
            send_view_only: false,
            send_target: None,
            send_amount: 1,
            send_anonymous: false,
            top: None,
            top_phase: RiseSparksLoadPhase::Idle,
            sessions: HashMap::new(),
            active_session_id: None,
            bursts: Vec::new(),
            burst_signals: HashMap::new(),
            burst_signals_prune_at: SparksBurst::SIGNALS_PRUNE_FLOOR,
            direct_chats_refusing_sparks: HashSet::new(),
            snapshot,
            _presentation: subscription,
        }
    }

    pub fn note_direct_chat_refuses_sparks(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }
        self.direct_chats_refusing_sparks.insert(chat_id.to_owned());
        cx.notify();
    }

    pub fn direct_chat_refuses_sparks(&self, chat_id: &str) -> bool {
        self.direct_chats_refusing_sparks.contains(chat_id)
    }

    pub fn snapshot(&self) -> &RiseSparksPresentationSnapshot {
        &self.snapshot
    }

    pub fn balance(&self) -> &RiseSparksBalanceDTO {
        &self.snapshot.balance
    }

    pub fn phone_link(&self) -> Option<RiseonlyPublicLink> {
        let SparksPurchasePhase::ContinueOnPhone { product_id } = &self.purchase_phase else {
            return None;
        };
        let (pack, _) = rise_ledger::sparks_pack_for_product_id(product_id)?;
        Some(RiseonlyPublicLink::PurchaseSparks { amount: i64::from(pack.amount), shortfall: self.needed_amount })
    }

    pub fn has_balance(&self) -> bool {
        self.snapshot.has_balance
    }

    pub fn balance_phase(&self) -> &RiseSparksLoadPhase {
        &self.snapshot.balance_phase
    }

    pub fn earnings(&self) -> Option<&RiseSparksEarningsDTO> {
        self.snapshot.earnings.as_ref()
    }

    pub fn earnings_phase(&self) -> &RiseSparksLoadPhase {
        &self.snapshot.earnings_phase
    }

    pub fn is_transferring(&self) -> bool {
        self.snapshot.is_transferring
    }

    /// own received sparks come live with each balance event,
    /// channels part is as fresh as the last earnings read
    pub fn received_total(&self) -> i64 {
        self.snapshot.balance.earned.max(0)
            + self.snapshot.earnings.as_ref().map_or(0, |earnings| earnings.channels_total).max(0)
    }

    pub fn received_channels(&self) -> Vec<RiseSparksChannelEarningsDTO> {
        self.snapshot
            .earnings
            .as_ref()
            .map(|earnings| earnings.channels.iter().filter(|channel| channel.amount > 0).cloned().collect())
            .unwrap_or_default()
    }

    pub fn pending_amount(&self) -> i64 {
        self.sessions.values().map(|session| session.amount).sum()
    }

    pub fn spendable_now(&self) -> i64 {
        (self.snapshot.balance.available - self.pending_amount()).max(0)
    }

    pub fn maximum_send_amount(&self) -> i64 {
        RiseSparksLimits::tip_max_amount().min(self.spendable_now())
    }

    /// viewer own row leads the list when the server page doesnt reach it
    pub fn top_sender_rows(&self, cx: &App) -> Vec<RiseStoredSparksSender> {
        let senders = self.top.as_ref().map(|top| top.senders.clone()).unwrap_or_default();
        let mine = self.top.as_ref().map_or(0, |top| top.my_sparks).max(0);
        if mine <= 0 || senders.iter().any(|sender| sender.is_me) {
            return senders;
        }
        let Some(profile) = AuthServicesStore::try_read(cx).and_then(|auth| auth.profile()) else {
            return senders;
        };
        let me = RiseStoredSparksSender::new(
            Some(RiseStoredSparksSenderUser {
                id: profile.id.clone(),
                name: profile.name.clone(),
                tag: profile.tag.clone(),
                logo: profile.more.logo.clone(),
                is_premium: profile.more.is_premium,
                is_official: false,
            }),
            mine,
            false,
            true,
        );
        let mut rows = Vec::with_capacity(senders.len() + 1);
        rows.push(me);
        rows.extend(senders);
        rows
    }

    pub fn hidden_top_senders_count(&self, cx: &App) -> i64 {
        let rows = i64::try_from(self.top_sender_rows(cx).len()).unwrap_or(i64::MAX);
        (self.top.as_ref().map_or(0, |top| top.senders_count) - rows).max(0)
    }

    pub fn rows(&self, filter: RiseLedgerHistoryFilter) -> Vec<SparksHistoryRow> {
        self.snapshot
            .history
            .get(&filter)
            .map(|page| page.items.iter().map(SparksHistoryRow::new).collect())
            .unwrap_or_default()
    }

    pub fn has_history(&self, filter: RiseLedgerHistoryFilter) -> bool {
        self.snapshot.history.contains_key(&filter)
    }

    pub fn has_more_history(&self, filter: RiseLedgerHistoryFilter) -> bool {
        self.snapshot.history.get(&filter).is_some_and(|page| page.has_more)
    }

    pub fn history_phase(&self, filter: RiseLedgerHistoryFilter) -> RiseSparksLoadPhase {
        self.snapshot.history_phase.get(&filter).cloned().unwrap_or(RiseSparksLoadPhase::Idle)
    }

    pub fn is_loading_more(&self, filter: RiseLedgerHistoryFilter) -> bool {
        self.snapshot.loading_more.contains(&filter)
    }

    pub fn active_session(&self) -> Option<&SparksSendSession> {
        self.active_session_id.and_then(|id| self.sessions.get(&id))
    }

    pub fn burst_signal(&mut self, target: &SparksTarget, cx: &mut Context<Self>) -> Entity<SparksBurstSignal> {
        if let Some(signal) = self.burst_signals.get(target).and_then(|weak| weak.signal.upgrade()) {
            return signal;
        }

        let signal = cx.new(|_| SparksBurstSignal::new());
        self.burst_signals.insert(target.clone(), SparksWeakBurstSignal::new(&signal));
        if self.burst_signals.len() >= self.burst_signals_prune_at {
            self.burst_signals.retain(|_, weak| weak.signal.upgrade().is_some());
            self.burst_signals_prune_at = SparksBurst::SIGNALS_PRUNE_FLOOR.max(self.burst_signals.len() * 2);
        }
        signal
    }

    pub fn live_burst_signal(&self, target: &SparksTarget) -> Option<Entity<SparksBurstSignal>> {
        self.burst_signals.get(target).and_then(|weak| weak.signal.upgrade())
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.selected_filter = RiseLedgerHistoryFilter::All;
        self.direct_chats_refusing_sparks = HashSet::new();
        self.is_purchase_sheet_presented = false;
        self.needed_amount = None;
        self.offers = Vec::new();
        self.offers_phase = SparksOffersPhase::Idle;
        self.purchase_phase = SparksPurchasePhase::Idle;
        self.is_success_sheet_presented = false;
        self.success_amount = 0;
        self.is_send_sheet_presented = false;
        self.send_view_only = false;
        self.send_target = None;
        self.send_amount = 1;
        self.send_anonymous = false;
        self.top = None;
        self.top_phase = RiseSparksLoadPhase::Idle;
        self.sessions = HashMap::new();
        self.active_session_id = None;
        self.bursts = Vec::new();
        self.burst_signals = HashMap::new();
        self.burst_signals_prune_at = SparksBurst::SIGNALS_PRUNE_FLOOR;
        cx.notify();
    }
}
