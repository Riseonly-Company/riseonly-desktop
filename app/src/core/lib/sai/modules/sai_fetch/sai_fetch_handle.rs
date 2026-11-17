use std::rc::Rc;

use gpui::{App, Context, Task};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::SaiHttpTraceStore;
use crate::core::lib::sai::sai::Sai;

use super::sai_fetch_memory_cache::SaiFetchMemoryCache;
use super::sai_fetch_options::SaiFetchOptions;

const MISSING_CACHE_KEY: &str = "?";

/// async operation returning t, run on the main thread
pub type SaiFetchOperation<T> = Rc<dyn Fn(&mut App) -> Task<anyhow::Result<T>>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaiFetchHandlePhase {
    #[default]
    Idle,
    Pending,
    Fulfilled,
    Rejected,
}

enum SaiFetchHandleOutcome<T> {
    Dropped,
    Fulfilled(T),
    Rejected(Rc<anyhow::Error>),
}

/// fetch handle, an entity where each phase change notifies
pub struct SaiFetchHandle<T: 'static> {
    phase: SaiFetchHandlePhase,
    data: Option<T>,
    error: Option<Rc<anyhow::Error>>,
    pub options: SaiFetchOptions,
    pub memory_cache_trace_context: Option<SaiFetchMemoryCacheTraceContext>,
    operation: SaiFetchOperation<T>,
    run_task: Option<Task<()>>,
    did_record_memory_cache_trace: bool,
}

impl<T: Clone + 'static> SaiFetchHandle<T> {
    /// makes a fetch handle that runs the operation with these options
    pub fn new(operation: SaiFetchOperation<T>, options: SaiFetchOptions) -> Self {
        Self {
            phase: SaiFetchHandlePhase::Idle,
            data: None,
            error: None,
            options,
            memory_cache_trace_context: None,
            operation,
            run_task: None,
            did_record_memory_cache_trace: false,
        }
    }

    /// makes a handle that already holds a cached value
    pub fn cached(
        value: T,
        options: SaiFetchOptions,
        memory_cache_trace_context: Option<SaiFetchMemoryCacheTraceContext>,
    ) -> Self {
        let operation_value = value.clone();
        let operation: SaiFetchOperation<T> = Rc::new(move |_: &mut App| Task::ready(Ok(operation_value.clone())));
        Self {
            phase: SaiFetchHandlePhase::Fulfilled,
            data: Some(value),
            error: None,
            options,
            memory_cache_trace_context,
            operation,
            run_task: None,
            did_record_memory_cache_trace: false,
        }
    }

    pub fn phase(&self) -> SaiFetchHandlePhase {
        self.phase
    }

    pub fn data(&self) -> Option<&T> {
        self.data.as_ref()
    }

    pub fn error(&self) -> Option<&anyhow::Error> {
        self.error.as_deref()
    }

    pub fn start(
        &mut self,
        on_success: impl FnOnce(T, &mut App) + 'static,
        on_error: impl FnOnce(&anyhow::Error, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_account() {
            return;
        }
        match self.phase {
            SaiFetchHandlePhase::Fulfilled => {
                if Self::records_debug_traces() {
                    self.record_memory_cache_trace_if_needed(cx);
                }
                if let Some(data) = self.data.clone() {
                    on_success(data, cx);
                }
                return;
            }
            SaiFetchHandlePhase::Rejected => {
                if let Some(error) = self.error.clone() {
                    on_error(&error, cx);
                }
                return;
            }
            SaiFetchHandlePhase::Pending => {
                let key = self.options.cache_key.clone().unwrap_or_else(|| MISSING_CACHE_KEY.to_owned());
                Sai::log(
                    "SaiFetch",
                    move || format!("handler ignored: already pending ({key})"),
                    SaiLogKind::Network,
                    cx,
                );
                return;
            }
            SaiFetchHandlePhase::Idle => {}
        }

        self.phase = SaiFetchHandlePhase::Pending;
        self.error = None;
        cx.notify();

        let operation = Rc::clone(&self.operation);
        let retained = cx.entity();
        self.run_task = Some(cx.spawn(async move |this, cx| {
            let _retained = retained;
            let pending = cx.update(|cx| operation(cx));
            let result = pending.await;
            let outcome = this
                .update(cx, |handle, cx| handle.settle(result, cx))
                .unwrap_or(SaiFetchHandleOutcome::Dropped);
            match outcome {
                SaiFetchHandleOutcome::Dropped => {}
                SaiFetchHandleOutcome::Fulfilled(value) => cx.update(|cx| on_success(value, cx)),
                SaiFetchHandleOutcome::Rejected(error) => cx.update(|cx| on_error(&error, cx)),
            }
        }));
    }

    fn settle(&mut self, result: anyhow::Result<T>, cx: &mut Context<Self>) -> SaiFetchHandleOutcome<T> {
        if !self.is_current_account() {
            self.data = None;
            self.error = None;
            self.phase = SaiFetchHandlePhase::Idle;
            self.run_task = None;
            cx.notify();
            return SaiFetchHandleOutcome::Dropped;
        }
        match result {
            Ok(value) => {
                self.data = Some(value.clone());
                self.phase = SaiFetchHandlePhase::Fulfilled;
                if let Some(key) = self.options.cache_key.as_deref() {
                    SaiFetchMemoryCache::set_value(Rc::new(value.clone()), key, cx);
                }
                cx.notify();
                SaiFetchHandleOutcome::Fulfilled(value)
            }
            Err(error) => {
                let error = Rc::new(error);
                self.error = Some(Rc::clone(&error));
                self.phase = SaiFetchHandlePhase::Rejected;
                cx.notify();
                SaiFetchHandleOutcome::Rejected(error)
            }
        }
    }

    fn is_current_account(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(
            self.options.account_user_id.as_deref(),
            self.options.account_epoch.as_deref(),
        )
    }

    fn records_debug_traces() -> bool {
        cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools")
    }

    fn record_memory_cache_trace_if_needed(&mut self, cx: &mut App) {
        if self.did_record_memory_cache_trace {
            return;
        }
        let Some(ctx) = self.memory_cache_trace_context.clone() else {
            return;
        };
        self.did_record_memory_cache_trace = true;
        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| {
            store.append_memory_cache_hit_http_trace(
                &ctx.method,
                &ctx.url,
                ctx.fetch_id.as_deref(),
                &ctx.request_body_wire,
                &ctx.response_body_preview,
                cx,
            );
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiFetchMemoryCacheTraceContext {
    pub method: String,
    pub url: String,
    pub fetch_id: Option<String>,
    pub request_body_wire: String,
    pub response_body_preview: String,
}

#[cfg(test)]
#[path = "sai_fetch_handle_tests.rs"]
mod tests;
