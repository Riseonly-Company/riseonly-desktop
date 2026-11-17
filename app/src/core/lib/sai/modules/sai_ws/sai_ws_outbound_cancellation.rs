use std::collections::HashSet;
use std::sync::LazyLock;

use parking_lot::Mutex;

#[derive(Default)]
struct SaiWsOutboundCancellationState {
    cancelled_requests: HashSet<String>,
    cancelled_slots: HashSet<String>,
}

static STATE: LazyLock<Mutex<SaiWsOutboundCancellationState>> =
    LazyLock::new(|| Mutex::new(SaiWsOutboundCancellationState::default()));

pub enum SaiWsOutboundCancellation {}

impl SaiWsOutboundCancellation {
    pub fn cancel_request(token: &str) {
        if token.is_empty() {
            return;
        }
        STATE.lock().cancelled_requests.insert(token.to_owned());
    }

    pub fn cancel_slot(token: &str, index: usize) {
        if token.is_empty() {
            return;
        }
        STATE.lock().cancelled_slots.insert(Self::slot_key(token, index));
    }

    pub fn is_request_cancelled(token: &str) -> bool {
        !token.is_empty() && STATE.lock().cancelled_requests.contains(token)
    }

    pub fn is_slot_cancelled(token: &str, index: usize) -> bool {
        !token.is_empty() && STATE.lock().cancelled_slots.contains(&Self::slot_key(token, index))
    }

    pub fn clear(token: &str) {
        if token.is_empty() {
            return;
        }
        let prefix = format!("{token}#");
        let mut state = STATE.lock();
        state.cancelled_requests.remove(token);
        state.cancelled_slots.retain(|slot| !slot.starts_with(&prefix));
    }

    pub fn clear_all() {
        let mut state = STATE.lock();
        state.cancelled_requests.clear();
        state.cancelled_slots.clear();
    }

    fn slot_key(token: &str, index: usize) -> String {
        format!("{token}#{index}")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, thiserror::Error)]
#[error("outbound request cancelled")]
pub struct SaiWsOutboundCancelledError;
