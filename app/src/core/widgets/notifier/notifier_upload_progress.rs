use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gpui::App;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::widgets::notifier::notifier_progress::NotifierProgressStore;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};

/// one long upload (story, post riffs track) under the same bottom banner as a download
/// ring with the real share and a progress title
/// a tap hides it and the upload goes on
pub struct NotifierUploadProgress {
    state: Rc<RefCell<NotifierUploadProgressState>>,
}

struct NotifierUploadProgressState {
    title: String,
    cancel_title: Option<String>,
    on_cancel: Option<Rc<dyn Fn(&mut App)>>,
    toast_id: Option<Uuid>,
    fraction: f64,
    is_open: bool,
}

impl NotifierUploadProgress {
    const CEILING: f64 = 0.99;

    /// on_cancel only where stopping is real, without it the banner has no button
    pub fn new(
        title: impl Into<String>,
        cancel_title: Option<String>,
        on_cancel: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut App,
    ) -> Self {
        let state = Rc::new(RefCell::new(NotifierUploadProgressState {
            title: title.into(),
            cancel_title: if on_cancel.is_none() { None } else { cancel_title },
            on_cancel,
            toast_id: None,
            fraction: 0.0,
            is_open: true,
        }));
        Self::show(&state, cx);
        Self { state }
    }

    pub fn is_open(&self) -> bool {
        self.state.borrow().is_open
    }

    /// never steps back, a step without bytes of its own (server work) keeps the last value
    pub fn update(&self, value: f64, cx: &mut App) {
        let (toast_id, next) = {
            let mut state = self.state.borrow_mut();
            if !state.is_open {
                return;
            }
            let next = Self::CEILING.min(state.fraction.max(value));
            if next <= state.fraction {
                return;
            }
            state.fraction = next;
            (state.toast_id, next)
        };
        let Some(toast_id) = toast_id else {
            return;
        };
        let store = NotifierProgressStore::shared(cx);
        if store.update(cx, |store, cx| store.update(toast_id, next, cx)) {
            return;
        }
        if store.read(cx).can_return(toast_id, cx) {
            Self::show(&self.state, cx);
        }
    }

    /// takes the progress banner place, also when the person hid it
    pub fn succeed(&self, title: impl Into<String>, content: Option<String>, cx: &mut App) {
        if self.close().is_none() {
            return;
        }
        let options = NotifierSendOptions {
            content,
            r#type: NotifierType::Success,
            position: NotifierPosition::Bottom,
            icon: NotifierIcon::System("checkmark.circle.fill".to_string()),
            ..NotifierSendOptions::default()
        };
        Notifier::send(title, options, cx);
    }

    pub fn fail(&self, title: impl Into<String>, content: Option<String>, cx: &mut App) {
        let Some(id) = self.close() else {
            return;
        };
        Self::dismiss_if_showing(id, cx);
        let options = NotifierSendOptions {
            content,
            r#type: NotifierType::Error,
            position: NotifierPosition::Bottom,
            icon: NotifierIcon::System("exclamationmark.triangle.fill".to_string()),
            ..NotifierSendOptions::default()
        };
        Notifier::send(title, options, cx);
    }

    /// removes the banner with no outcome of its own, caller explains it or the upload was cancelled
    pub fn finish(&self, cx: &mut App) {
        let Some(id) = self.close() else {
            return;
        };
        Self::dismiss_if_showing(id, cx);
    }

    fn close(&self) -> Option<Uuid> {
        let mut state = self.state.borrow_mut();
        if !state.is_open {
            return None;
        }
        state.is_open = false;
        Some(state.toast_id.take().unwrap_or_else(Uuid::new_v4))
    }

    fn dismiss_if_showing(id: Uuid, cx: &mut App) {
        let store = NotifierProgressStore::shared(cx);
        if store.read(cx).is_showing(id, cx) {
            Notifier::dismiss(id, cx);
        }
    }

    fn show(state: &Rc<RefCell<NotifierUploadProgressState>>, cx: &mut App) {
        let (title, cancel_title, fraction, on_cancel) = {
            let state = state.borrow();
            (state.title.clone(), state.cancel_title.clone(), state.fraction, state.on_cancel.clone())
        };
        let cancel: Option<Rc<dyn Fn(&mut App)>> = on_cancel.map(|on_cancel| {
            let weak: Weak<RefCell<NotifierUploadProgressState>> = Rc::downgrade(state);
            Rc::new(move |cx: &mut App| {
                if let Some(state) = weak.upgrade() {
                    let mut state = state.borrow_mut();
                    state.is_open = false;
                    state.toast_id = None;
                }
                on_cancel(cx);
            }) as Rc<dyn Fn(&mut App)>
        });
        let toast_id =
            Notifier::send_progress(title, cancel_title, fraction, NotifierPosition::Bottom, 0.0, cancel, cx);
        state.borrow_mut().toast_id = Some(toast_id);
    }
}
