use std::fmt;

use serde::{Deserialize, Serialize};

use crate::generation::{Generation, GenerationCounter};
use crate::identifiers::AccountId;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountEpoch(u64);

impl AccountEpoch {
    pub const FIRST: Self = Self(0);

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeId(String);

impl ScopeId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Captured identity of the work that started an async operation.
///
/// Account, epoch, scope and generation are all checked, so a scope that closes
/// and reopens (ABA) does not revive a result from the earlier one.
#[derive(Clone, Debug)]
pub struct StaleGuard {
    account: AccountId,
    epoch: AccountEpoch,
    scope: ScopeId,
    generation: Generation,
    counter: GenerationCounter,
}

impl StaleGuard {
    pub fn capture(
        account: AccountId,
        epoch: AccountEpoch,
        scope: ScopeId,
        counter: GenerationCounter,
    ) -> Self {
        let generation = counter.current();
        Self {
            account,
            epoch,
            scope,
            generation,
            counter,
        }
    }

    pub fn account(&self) -> &AccountId {
        &self.account
    }

    pub fn scope(&self) -> &ScopeId {
        &self.scope
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    pub fn is_valid_for(&self, account: &AccountId, epoch: AccountEpoch, scope: &ScopeId) -> bool {
        self.account == *account
            && self.epoch == epoch
            && self.scope == *scope
            && self.counter.is_current(self.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> (StaleGuard, AccountId, ScopeId, GenerationCounter) {
        let account = AccountId::new("u1").unwrap();
        let scope = ScopeId::new("chat:42");
        let counter = GenerationCounter::new();
        let guard = StaleGuard::capture(
            account.clone(),
            AccountEpoch::FIRST,
            scope.clone(),
            counter.clone(),
        );
        (guard, account, scope, counter)
    }

    #[test]
    fn a_fresh_capture_is_valid() {
        let (guard, account, scope, _) = guard();
        assert!(guard.is_valid_for(&account, AccountEpoch::FIRST, &scope));
    }

    #[test]
    fn a_new_generation_invalidates_it() {
        let (guard, account, scope, counter) = guard();
        counter.bump();
        assert!(!guard.is_valid_for(&account, AccountEpoch::FIRST, &scope));
    }

    #[test]
    fn an_account_switch_invalidates_it() {
        let (guard, _, scope, _) = guard();
        let other = AccountId::new("u2").unwrap();
        assert!(!guard.is_valid_for(&other, AccountEpoch::FIRST, &scope));
    }

    #[test]
    fn a_new_epoch_on_the_same_account_invalidates_it() {
        let (guard, account, scope, _) = guard();
        assert!(!guard.is_valid_for(&account, AccountEpoch::FIRST.next(), &scope));
    }

    #[test]
    fn a_different_scope_invalidates_it() {
        let (guard, account, _, _) = guard();
        assert!(!guard.is_valid_for(&account, AccountEpoch::FIRST, &ScopeId::new("chat:43")));
    }

    #[test]
    fn reopening_the_same_scope_does_not_revive_a_stale_guard() {
        let (guard, account, scope, counter) = guard();

        counter.bump();
        let reopened = StaleGuard::capture(
            account.clone(),
            AccountEpoch::FIRST,
            scope.clone(),
            counter.clone(),
        );

        assert!(!guard.is_valid_for(&account, AccountEpoch::FIRST, &scope));
        assert!(reopened.is_valid_for(&account, AccountEpoch::FIRST, &scope));
    }
}
