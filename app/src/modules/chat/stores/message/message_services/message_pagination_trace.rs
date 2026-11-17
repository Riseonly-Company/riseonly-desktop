use std::collections::HashMap;

use gpui::{App, Global};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::stores::message::message_actions::message_types::MessageHistoryDirection;

struct DiagnosticState {
    signature: String,
    recorded_at: f64,
}

#[derive(Default)]
struct DiagnosticStates(HashMap<String, DiagnosticState>);

impl Global for DiagnosticStates {}

pub enum MessagePaginationTrace {}

impl MessagePaginationTrace {
    pub const IS_ENABLED: bool = option_env!("SAI_PAGINATION_TRACE").is_some();

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        event: &str,
        scope: &str,
        direction: Option<MessageHistoryDirection>,
        token: Option<u64>,
        cursor: Option<&str>,
        anchor_id: Option<&str>,
        cached_count: Option<i64>,
        incoming_count: Option<i64>,
        result_count: Option<i64>,
        has_more: Option<bool>,
        detail: Option<&str>,
        cx: &mut App,
    ) {
        if !Self::IS_ENABLED {
            return;
        }
        let mut fields = vec![format!("event={event}"), format!("scope={scope}")];
        if let Some(direction) = direction {
            fields.push(format!("edge={}", direction.raw_value()));
        }
        if let Some(token) = token {
            fields.push(format!("token={token}"));
        }
        if let Some(cursor) = cursor {
            fields.push(format!("cursor={}", Self::short(cursor)));
        }
        if let Some(anchor_id) = anchor_id {
            fields.push(format!("anchor={}", Self::short(anchor_id)));
        }
        if let Some(cached_count) = cached_count {
            fields.push(format!("cached={cached_count}"));
        }
        if let Some(incoming_count) = incoming_count {
            fields.push(format!("incoming={incoming_count}"));
        }
        if let Some(result_count) = result_count {
            fields.push(format!("result={result_count}"));
        }
        if let Some(has_more) = has_more {
            fields.push(format!("hasMore={has_more}"));
        }
        if let Some(detail) = detail
            && !detail.is_empty()
        {
            fields.push(detail.to_owned());
        }
        Sai::log_channel("MessagePagination", move || fields.join(" | "), "pagination", cx);
    }

    pub fn diagnostic(
        event: &str,
        scope: &str,
        direction: Option<MessageHistoryDirection>,
        signature: Option<&str>,
        detail: &str,
        minimum_interval: f64,
        cx: &mut App,
    ) {
        if !cfg!(any(debug_assertions, feature = "sai_debug_tools")) {
            return;
        }
        let signature = signature.unwrap_or(detail);
        let now = RiseClock::now().time_interval_since_1970();
        let key = format!("{scope}|{event}|{}", direction.map_or("none", MessageHistoryDirection::raw_value));
        let states = &mut cx.default_global::<DiagnosticStates>().0;
        if let Some(previous) = states.get(&key)
            && previous.signature == signature
            && now - previous.recorded_at < minimum_interval
        {
            return;
        }
        if states.len() >= 128
            && !states.contains_key(&key)
            && let Some(first) = states.keys().next().cloned()
        {
            states.remove(&first);
        }
        states.insert(key, DiagnosticState { signature: signature.to_owned(), recorded_at: now });
        let mut fields = vec![format!("event={event}"), format!("scope={scope}")];
        if let Some(direction) = direction {
            fields.push(format!("edge={}", direction.raw_value()));
        }
        fields.push(detail.to_owned());
        Sai::log_channel("MessagePagination", move || fields.join(" | "), "pagination", cx);
    }

    fn short(value: &str) -> String {
        let normalized = trimming_whitespaces_and_newlines(value);
        let count = normalized.chars().count();
        if count <= 72 {
            return normalized.to_owned();
        }
        let prefix: String = normalized.chars().take(44).collect();
        let suffix: String = normalized.chars().skip(count - 20).collect();
        format!("{prefix}\u{2026}{suffix}")
    }
}
