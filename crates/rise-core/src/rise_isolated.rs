use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

static NEXT_THREAD_TOKEN: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static THREAD_TOKEN: u64 = NEXT_THREAD_TOKEN.fetch_add(1, Ordering::Relaxed);
}

fn current_thread_token() -> u64 {
    THREAD_TOKEN.with(|token| *token)
}

/// actor like wrapper, state is only reachable through with
/// one with closure is one sync stretch so it cant await
/// lock never lives across a suspension
pub struct RiseIsolated<State> {
    state: Mutex<State>,
    owner: AtomicU64,
}

struct OwnerReset<'a> {
    owner: &'a AtomicU64,
}

impl Drop for OwnerReset<'_> {
    fn drop(&mut self) {
        self.owner.store(0, Ordering::Release);
    }
}

impl<State> RiseIsolated<State> {
    pub fn new(state: State) -> Self {
        Self {
            state: Mutex::new(state),
            owner: AtomicU64::new(0),
        }
    }

    /// runs body with exclusive access to the state
    /// calling with again from inside body deadlocks (debug asserts), so does blocking io in it
    pub fn with<R>(&self, body: impl FnOnce(&mut State) -> R) -> R {
        let token = current_thread_token();
        debug_assert_ne!(
            self.owner.load(Ordering::Acquire),
            token,
            "RiseIsolated::with re-entered from its own closure"
        );
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        self.owner.store(token, Ordering::Release);
        let _reset = OwnerReset { owner: &self.owner };
        body(&mut state)
    }
}

impl<State: Default> Default for RiseIsolated<State> {
    fn default() -> Self {
        Self::new(State::default())
    }
}

impl<State> fmt::Debug for RiseIsolated<State> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RiseIsolated").finish_non_exhaustive()
    }
}
