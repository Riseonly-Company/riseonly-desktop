use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_canon::rise_ledger::HistoryFilter as RiseLedgerHistoryFilter;

use crate::modules::sparks::engine::rise_sparks_engine_models::{
    RiseSparksBalanceDTO, RiseSparksEarningsDTO, RiseSparksHistoryPage, RiseSparksLoadPhase, SparksBalanceExt,
};
use crate::modules::sparks::engine::rise_sparks_repository::RiseSparksRepositorySnapshot;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseSparksPresentationSnapshot {
    pub account_id: Option<String>,
    pub account_epoch: Option<String>,
    pub session_id: Option<Uuid>,
    pub generation: u64,
    pub balance: RiseSparksBalanceDTO,
    pub has_balance: bool,
    pub balance_phase: RiseSparksLoadPhase,
    pub earnings: Option<RiseSparksEarningsDTO>,
    pub earnings_phase: RiseSparksLoadPhase,
    pub is_transferring: bool,
    pub history: HashMap<RiseLedgerHistoryFilter, RiseSparksHistoryPage>,
    pub history_phase: HashMap<RiseLedgerHistoryFilter, RiseSparksLoadPhase>,
    pub loading_more: HashSet<RiseLedgerHistoryFilter>,
}

impl RiseSparksPresentationSnapshot {
    pub fn empty() -> Self {
        Self {
            account_id: None,
            account_epoch: None,
            session_id: None,
            generation: 0,
            balance: RiseSparksBalanceDTO::zero(),
            has_balance: false,
            balance_phase: RiseSparksLoadPhase::Idle,
            earnings: None,
            earnings_phase: RiseSparksLoadPhase::Idle,
            is_transferring: false,
            history: HashMap::new(),
            history_phase: HashMap::new(),
            loading_more: HashSet::new(),
        }
    }

    pub fn cleared(&self, account_id: Option<String>, account_epoch: Option<String>, session_id: Option<Uuid>) -> Self {
        Self {
            account_id,
            account_epoch,
            session_id,
            generation: self.generation.wrapping_add(1),
            ..Self::empty()
        }
    }
}

pub struct RiseSparksPresentation {
    snapshot: RiseSparksPresentationSnapshot,
}

impl RiseSparksPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { snapshot: RiseSparksPresentationSnapshot::empty() })
    }

    pub fn snapshot(&self) -> &RiseSparksPresentationSnapshot {
        &self.snapshot
    }

    pub fn activate(&mut self, account_id: &str, account_epoch: &str, session_id: Uuid, cx: &mut Context<Self>) {
        self.snapshot =
            self.snapshot.cleared(Some(account_id.to_owned()), Some(account_epoch.to_owned()), Some(session_id));
        cx.notify();
    }

    pub fn publish(&mut self, value: RiseSparksRepositorySnapshot, cx: &mut Context<Self>) -> bool {
        if self.snapshot.account_id.as_deref() != Some(value.account_id.as_str())
            || self.snapshot.account_epoch.as_deref() != Some(value.account_epoch.as_str())
            || self.snapshot.session_id != Some(value.session_id)
        {
            return false;
        }
        let next = RiseSparksPresentationSnapshot {
            account_id: Some(value.account_id),
            account_epoch: Some(value.account_epoch),
            session_id: Some(value.session_id),
            generation: self.snapshot.generation.wrapping_add(1),
            balance: value.balance,
            has_balance: value.has_balance,
            balance_phase: value.balance_phase,
            earnings: value.earnings,
            earnings_phase: value.earnings_phase,
            is_transferring: value.is_transferring,
            history: value.history,
            history_phase: value.history_phase,
            loading_more: value.loading_more,
        };
        if next.balance == self.snapshot.balance
            && next.has_balance == self.snapshot.has_balance
            && next.balance_phase == self.snapshot.balance_phase
            && next.earnings == self.snapshot.earnings
            && next.earnings_phase == self.snapshot.earnings_phase
            && next.is_transferring == self.snapshot.is_transferring
            && next.history == self.snapshot.history
            && next.history_phase == self.snapshot.history_phase
            && next.loading_more == self.snapshot.loading_more
        {
            return true;
        }
        self.snapshot = next;
        cx.notify();
        true
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.snapshot.account_id.as_deref() != Some(account_id) || self.snapshot.session_id != Some(session_id) {
            return;
        }
        self.snapshot = self.snapshot.cleared(None, None, None);
        cx.notify();
    }
}
