use std::any::Any;

use crate::core::feature_hooks::FeatureHookSlot;

/// opaque operation token, the installer downcasts it back
pub type ChatPerformanceTraceOperationToken = Box<dyn Any>;

pub struct ChatPerformanceTraceHooks {
    pub record: fn(event: &str, fields: Vec<(String, String)>),
    pub begin_operation: fn(name: &str) -> Option<ChatPerformanceTraceOperationToken>,
    pub end_operation:
        fn(token: Option<ChatPerformanceTraceOperationToken>, threshold_ms: f64, fields: Vec<(String, String)>),
}

pub(crate) static CHAT_PERFORMANCE_TRACE_HOOKS: FeatureHookSlot<ChatPerformanceTraceHooks> = FeatureHookSlot::new("CHAT_PERFORMANCE_TRACE_HOOKS");

/// installed once at startup, a second call keeps the first hooks
pub fn set_chat_performance_trace_hooks(hooks: ChatPerformanceTraceHooks) {
    let _ = CHAT_PERFORMANCE_TRACE_HOOKS.set(hooks);
}

pub fn chat_performance_trace_hooks() -> Option<&'static ChatPerformanceTraceHooks> {
    CHAT_PERFORMANCE_TRACE_HOOKS.get()
}

/// records a trace event, the fields are built only when a trace listens
pub fn chat_performance_trace_record(event: &str, fields: impl FnOnce() -> Vec<(String, String)>) {
    if let Some(hooks) = chat_performance_trace_hooks() {
        (hooks.record)(event, fields());
    }
}

/// starts a trace operation, returns the token
pub fn chat_performance_trace_begin_operation(name: &str) -> Option<ChatPerformanceTraceOperationToken> {
    chat_performance_trace_hooks().and_then(|hooks| (hooks.begin_operation)(name))
}

pub fn chat_performance_trace_end_operation(
    token: Option<ChatPerformanceTraceOperationToken>,
    threshold_ms: f64,
    fields: impl FnOnce() -> Vec<(String, String)>,
) {
    if token.is_none() {
        return;
    }
    if let Some(hooks) = chat_performance_trace_hooks() {
        (hooks.end_operation)(token, threshold_ms, fields());
    }
}
