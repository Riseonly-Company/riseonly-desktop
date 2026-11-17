use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{Bounds, Pixels};

use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;

const SCROLL_LOCK_OPERATION: &str = "hold_menu.scroll_lock";
const SCROLL_UNLOCK_OPERATION: &str = "hold_menu.scroll_unlock";
const SCROLL_LOCK_THRESHOLD_MS: f64 = 4.0;

thread_local! {
    static ENTRIES: RefCell<Vec<Bounds<Pixels>>> = const { RefCell::new(Vec::new()) };
}

pub enum HoldContextMenuScrollLock {}

impl HoldContextMenuScrollLock {
    /// locks scrolling under the source at from, its frame in window coordinates
    pub fn lock(from: Bounds<Pixels>) {
        let trace = ChatPerformanceTrace::shared();
        let token = trace.begin_operation(SCROLL_LOCK_OPERATION, HashMap::new());
        Self::unlock();
        let count = ENTRIES.with(|entries| {
            let mut entries = entries.borrow_mut();
            entries.push(from);
            entries.len()
        });
        trace.end_operation(
            token,
            SCROLL_LOCK_THRESHOLD_MS,
            HashMap::from([("count".to_owned(), count.to_string())]),
        );
    }

    pub fn unlock() {
        let trace = ChatPerformanceTrace::shared();
        let count = ENTRIES.with(|entries| entries.borrow().len());
        let token = trace.begin_operation(
            SCROLL_UNLOCK_OPERATION,
            HashMap::from([("count".to_owned(), count.to_string())]),
        );
        ENTRIES.with(|entries| entries.borrow_mut().clear());
        trace.end_operation(token, SCROLL_LOCK_THRESHOLD_MS, HashMap::new());
    }

    /// desktop, true when the overlay layer eats wheel and trackpad scrolling under the menu
    pub fn is_locked() -> bool {
        ENTRIES.with(|entries| !entries.borrow().is_empty())
    }
}
