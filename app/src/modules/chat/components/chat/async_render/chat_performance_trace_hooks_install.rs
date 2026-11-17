use std::collections::HashMap;

use crate::core::ui::lottie_sticker::chat_performance_trace_hooks::{
    ChatPerformanceTraceHooks, ChatPerformanceTraceOperationToken, set_chat_performance_trace_hooks,
};
use crate::modules::chat::components::chat::async_render::chat_performance_trace::{
    ChatPerformanceTrace, OperationToken,
};

pub(crate) fn install_chat_performance_trace_hooks() {
    set_chat_performance_trace_hooks(ChatPerformanceTraceHooks {
        record: record_event,
        begin_operation,
        end_operation,
    });
}

fn record_event(event: &str, fields: Vec<(String, String)>) {
    ChatPerformanceTrace::shared().record(event, fields.into_iter().collect());
}

fn begin_operation(name: &str) -> Option<ChatPerformanceTraceOperationToken> {
    ChatPerformanceTrace::shared()
        .begin_operation(name, HashMap::new())
        .map(|token| Box::new(token) as ChatPerformanceTraceOperationToken)
}

fn end_operation(token: Option<ChatPerformanceTraceOperationToken>, threshold_ms: f64, fields: Vec<(String, String)>) {
    let token = token.and_then(|token| token.downcast::<OperationToken>().ok()).map(|token| *token);
    ChatPerformanceTrace::shared().end_operation(token, threshold_ms, fields.into_iter().collect());
}
