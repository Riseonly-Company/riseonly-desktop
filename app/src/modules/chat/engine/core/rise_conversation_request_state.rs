use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use gpui::{App, AsyncApp, EntityId};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::lib::sai::core::utilities::sai_payload_path::{SaiJSONCodec, SaiPayloadPath};
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::{
    SaiSliceRegistry, SaiSliceState, SaiWsSliceStatus,
};

struct RiseConversationRequestStateFields<Data> {
    status: SaiWsSliceStatus,
    data: Option<Data>,
    error: Option<String>,
    failure: Option<RiseFailure>,
    revision: u64,
    request_generation: u64,
}

pub struct RiseConversationRequestState<Data> {
    fields: RefCell<RiseConversationRequestStateFields<Data>>,
    app: AsyncApp,
    refresh_scheduled: Rc<Cell<bool>>,
    readers: RefCell<Vec<EntityId>>,
    pending_readers: Rc<RefCell<Vec<EntityId>>>,
}

impl<Data: DeserializeOwned + 'static> RiseConversationRequestState<Data> {
    pub fn new(cx: &mut App) -> Rc<Self> {
        let state = Rc::new(Self {
            fields: RefCell::new(RiseConversationRequestStateFields {
                status: SaiWsSliceStatus::Idle,
                data: None,
                error: None,
                failure: None,
                revision: 0,
                request_generation: 0,
            }),
            app: cx.to_async(),
            refresh_scheduled: Rc::new(Cell::new(false)),
            readers: RefCell::new(Vec::new()),
            pending_readers: Rc::new(RefCell::new(Vec::new())),
        });
        let registered: Rc<dyn SaiSliceState> = state.clone();
        SaiSliceRegistry::register(&registered, cx);
        state
    }

    pub fn revision(&self) -> u64 {
        self.note_read();
        self.fields.borrow().revision
    }

    pub fn has_value(&self) -> bool {
        self.note_read();
        self.fields.borrow().data.is_some()
    }

    /// returns a token to reject stale responses after a refresh or account change
    /// a background refresh may keep the current value visible
    pub fn begin(&self, reflect_pending_in_ui: bool) -> u64 {
        let token = {
            let mut fields = self.fields.borrow_mut();
            fields.request_generation = fields.request_generation.wrapping_add(1);
            fields.error = None;
            fields.failure = None;
            if reflect_pending_in_ui || fields.data.is_none() {
                fields.status = SaiWsSliceStatus::Pending;
            }
            fields.request_generation
        };
        self.did_change();
        token
    }

    pub fn publish(&self, value: Data, token: Option<u64>) {
        {
            let mut fields = self.fields.borrow_mut();
            if token.is_some_and(|token| token != fields.request_generation) {
                return;
            }
            fields.data = Some(value);
            fields.error = None;
            fields.failure = None;
            fields.status = SaiWsSliceStatus::Fulfilled;
            fields.revision = fields.revision.wrapping_add(1);
        }
        self.did_change();
    }

    pub fn publish_error(&self, error: &anyhow::Error, token: Option<u64>, keep_current_value: bool) {
        {
            let mut fields = self.fields.borrow_mut();
            if token.is_some_and(|token| token != fields.request_generation) {
                return;
            }
            fields.error = Some(error.to_string());
            fields.failure = CanonErrors::failure(error);
            fields.status = if keep_current_value && fields.data.is_some() {
                SaiWsSliceStatus::Fulfilled
            } else {
                SaiWsSliceStatus::Rejected
            };
            fields.revision = fields.revision.wrapping_add(1);
        }
        self.did_change();
    }

    pub fn invalidate_pending_requests(&self) {
        {
            let mut fields = self.fields.borrow_mut();
            fields.request_generation = fields.request_generation.wrapping_add(1);
            if fields.status == SaiWsSliceStatus::Pending {
                fields.status = if fields.data.is_none() {
                    SaiWsSliceStatus::Idle
                } else {
                    SaiWsSliceStatus::Fulfilled
                };
            }
        }
        self.did_change();
    }

    fn did_change(&self) {
        let readers = std::mem::take(&mut *self.readers.borrow_mut());
        if readers.is_empty() {
            return;
        }
        {
            let mut pending = self.pending_readers.borrow_mut();
            for view in readers {
                if !pending.contains(&view) {
                    pending.push(view);
                }
            }
        }
        if self.refresh_scheduled.replace(true) {
            return;
        }
        let app = self.app.clone();
        let scheduled = Rc::clone(&self.refresh_scheduled);
        let pending = Rc::clone(&self.pending_readers);
        self.app
            .foreground_executor()
            .spawn(async move {
                scheduled.set(false);
                let views = std::mem::take(&mut *pending.borrow_mut());
                app.update(|cx| {
                    for view in views {
                        cx.notify(view);
                    }
                });
            })
            .detach();
    }
}

impl<Data> RiseConversationRequestState<Data> {
    fn note_read(&self) {
        if let Some(view) = gpui::rendering_view() {
            let mut readers = self.readers.borrow_mut();
            if !readers.contains(&view) {
                readers.push(view);
            }
        }
    }
}

impl<Data: Clone> RiseConversationRequestState<Data> {
    pub fn data(&self) -> Option<Data> {
        self.note_read();
        self.fields.borrow().data.clone()
    }
}

impl<Data: DeserializeOwned + 'static> SaiSliceState for RiseConversationRequestState<Data> {
    fn status(&self) -> SaiWsSliceStatus {
        self.note_read();
        self.fields.borrow().status
    }

    fn error(&self) -> Option<String> {
        self.note_read();
        self.fields.borrow().error.clone()
    }

    fn failure(&self) -> Option<RiseFailure> {
        self.note_read();
        self.fields.borrow().failure.clone()
    }

    fn is_pending(&self) -> bool {
        self.status() == SaiWsSliceStatus::Pending
    }

    fn mark_pending(&self) {
        let _ = self.begin(true);
    }

    fn mark_fulfilled(&self, raw: Option<&Value>, take_path: Option<&str>) {
        let extracted = SaiPayloadPath::extract(raw, take_path);
        match SaiJSONCodec::decode::<Data>(extracted) {
            Some(value) => self.publish(value, None),
            None => {
                let error = anyhow::Error::from(RiseConversationStateError::InvalidPresentationValue);
                self.publish_error(&error, None, true);
            }
        }
    }

    fn mark_rejected(&self, error: &anyhow::Error) {
        self.publish_error(error, None, true);
    }

    fn reset_to_idle(&self) {
        {
            let mut fields = self.fields.borrow_mut();
            fields.request_generation = fields.request_generation.wrapping_add(1);
            fields.status = SaiWsSliceStatus::Idle;
            fields.data = None;
            fields.error = None;
            fields.revision = fields.revision.wrapping_add(1);
        }
        self.did_change();
    }

    fn callback_payload(&self, raw: Option<&Value>, take_path: Option<&str>) -> Option<Box<dyn Any>> {
        let extracted = SaiPayloadPath::extract(raw, take_path);
        SaiJSONCodec::decode::<Data>(extracted).map(|decoded| Box::new(decoded) as Box<dyn Any>)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseConversationStateError {
    InvalidPresentationValue,
    MissingActiveSession,
    StaleAccount,
}

impl RiseConversationStateError {
    pub fn error_description(&self) -> &'static str {
        match self {
            Self::InvalidPresentationValue => "Conversation data could not be decoded",
            Self::MissingActiveSession => "Conversation engine is not ready",
            Self::StaleAccount => "Conversation request belongs to a previous account",
        }
    }
}

impl fmt::Display for RiseConversationStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.error_description())
    }
}

impl std::error::Error for RiseConversationStateError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseConversationPageState {
    pub cursor: Option<String>,
    pub has_more: bool,
    pub is_loading: bool,
    pub failure: Option<String>,
    pub generation: u64,
}

impl RiseConversationPageState {
    pub const INITIAL: Self = Self {
        cursor: None,
        has_more: true,
        is_loading: false,
        failure: None,
        generation: 0,
    };

    pub fn begin(&mut self) -> Option<u64> {
        if !self.has_more || self.is_loading {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.is_loading = true;
        self.failure = None;
        Some(self.generation)
    }

    pub fn finish(&mut self, cursor: Option<String>, has_more: bool, generation: u64) {
        if self.generation != generation {
            return;
        }
        self.cursor = cursor;
        self.has_more = has_more;
        self.is_loading = false;
        self.failure = None;
    }

    pub fn fail(&mut self, error: &anyhow::Error, generation: u64) {
        if self.generation != generation {
            return;
        }
        self.is_loading = false;
        self.failure = Some(error.to_string());
    }

    pub fn reset(&mut self) {
        *self = Self::INITIAL;
    }
}
