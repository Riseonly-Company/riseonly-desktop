use std::rc::Rc;

use gpui::{App, Context};

pub(crate) type ExportProgressCancelAction = Rc<dyn Fn(&mut App)>;

/// one entity per export made by its owner, the overlay observes it
#[derive(Default)]
pub(crate) struct ExportProgressViewState {
    pub(crate) fraction_completed: f64,
    pub(crate) current_label: String,
    pub(crate) total_items: i64,
    pub(crate) processed_items: i64,
    pub(crate) is_cancelled: bool,
    pub(crate) failure_description: Option<String>,
    pub(crate) on_cancel: Option<ExportProgressCancelAction>,
}

impl ExportProgressViewState {
    pub(crate) fn new(_cx: &mut Context<Self>) -> Self {
        Self::default()
    }

    pub(crate) fn reset(&mut self, total: i64, cx: &mut Context<Self>) {
        self.fraction_completed = 0.0;
        self.current_label = String::new();
        self.total_items = total;
        self.processed_items = 0;
        self.is_cancelled = false;
        self.failure_description = None;
        cx.notify();
    }

    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        self.is_cancelled = true;
        cx.notify();
        if let Some(on_cancel) = self.on_cancel.clone() {
            cx.defer(move |cx| on_cancel(cx));
        }
    }
}
