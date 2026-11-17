use std::collections::HashMap;

use gpui::{App, Global};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SaiWsCoalescingStrategy {
    #[serde(rename = "toggleParity")]
    ToggleParity,
}

impl SaiWsCoalescingStrategy {
    const TOGGLE_PARITY_RAW: &'static str = "toggleParity";

    pub fn raw_value(self) -> &'static str {
        match self {
            Self::ToggleParity => Self::TOGGLE_PARITY_RAW,
        }
    }

    /// parses from the raw string, none if unknown
    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            Self::TOGGLE_PARITY_RAW => Some(Self::ToggleParity),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaiWsCoalescingOptions {
    pub key: String,
    pub strategy: SaiWsCoalescingStrategy,
}

impl SaiWsCoalescingOptions {
    /// new one for this key with the default toggle parity strategy
    pub fn new(key: String) -> Self {
        Self::with_strategy(key, SaiWsCoalescingStrategy::ToggleParity)
    }

    pub fn with_strategy(key: String, strategy: SaiWsCoalescingStrategy) -> Self {
        Self { key, strategy }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaiWsCoalescingConsumeResult {
    Send { tap_count: i64 },
    SkippedEvenParity { tap_count: i64 },
}

/// tap counts per key for the coalescing registry, main thread only
#[derive(Default)]
struct SaiWsCoalescingTapCounts(HashMap<String, i64>);

impl Global for SaiWsCoalescingTapCounts {}

pub enum SaiWsCoalescingRegistry {}

impl SaiWsCoalescingRegistry {
    pub fn bump(options: &SaiWsCoalescingOptions, cx: &mut App) {
        let k = Self::normalize_key(&options.key);
        let counts = &mut cx.default_global::<SaiWsCoalescingTapCounts>().0;
        let count = counts.entry(k).or_insert(0);
        *count = count.wrapping_add(1);
    }

    pub fn consume(options: &SaiWsCoalescingOptions, cx: &mut App) -> SaiWsCoalescingConsumeResult {
        let k = Self::normalize_key(&options.key);
        let n = cx
            .default_global::<SaiWsCoalescingTapCounts>()
            .0
            .remove(&k)
            .unwrap_or(0);
        match options.strategy {
            SaiWsCoalescingStrategy::ToggleParity => {
                if n == 0 {
                    return SaiWsCoalescingConsumeResult::Send { tap_count: 0 };
                }
                if n % 2 == 0 {
                    return SaiWsCoalescingConsumeResult::SkippedEvenParity { tap_count: n };
                }
                SaiWsCoalescingConsumeResult::Send { tap_count: n }
            }
        }
    }

    pub fn discard(options: &SaiWsCoalescingOptions, cx: &mut App) {
        let k = Self::normalize_key(&options.key);
        cx.default_global::<SaiWsCoalescingTapCounts>().0.remove(&k);
    }

    pub fn clear_all(cx: &mut App) {
        cx.default_global::<SaiWsCoalescingTapCounts>().0.clear();
    }

    fn normalize_key(key: &str) -> String {
        trimming_whitespaces_and_newlines(key).to_owned()
    }
}
