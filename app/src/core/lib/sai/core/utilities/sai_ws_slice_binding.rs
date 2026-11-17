use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gpui::{App, AsyncApp, EntityId, Global};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::sai_payload_path::{SaiJSONCodec, SaiPayloadPath};
use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SaiWsSliceStatus {
    Idle,
    Pending,
    Fulfilled,
    Rejected,
}

impl SaiWsSliceStatus {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Pending => "pending",
            Self::Fulfilled => "fulfilled",
            Self::Rejected => "rejected",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "idle" => Some(Self::Idle),
            "pending" => Some(Self::Pending),
            "fulfilled" => Some(Self::Fulfilled),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct SaiWireEmptyDTO {}

/// the transport surface every slice shares
/// slices live on the main thread so state is behind a shared ref and owned through rc
pub trait SaiSliceState {
    fn status(&self) -> SaiWsSliceStatus;
    fn error(&self) -> Option<String>;
    /// same rejection not flattened into a string, code and params and field pointer
    /// error stays a string since hundreds of sites read it but a string cant drive focus or retry
    fn failure(&self) -> Option<RiseFailure>;
    fn is_pending(&self) -> bool;
    fn mark_pending(&self);
    fn mark_fulfilled(&self, raw: Option<&Value>, take_path: Option<&str>);
    fn mark_rejected(&self, error: &anyhow::Error);
    fn reset_to_idle(&self);
    fn callback_payload(&self, raw: Option<&Value>, take_path: Option<&str>) -> Option<Box<dyn Any>>;
}

struct SaiWeakSliceState {
    value: Weak<dyn SaiSliceState>,
}

impl SaiWeakSliceState {
    fn new(value: &Rc<dyn SaiSliceState>) -> Self {
        Self {
            value: Rc::downgrade(value),
        }
    }
}

/// shared slice registry, weak refs only so a slice dies with its store
#[derive(Default)]
pub struct SaiSliceRegistry {
    slices: HashMap<usize, SaiWeakSliceState>,
}

impl Global for SaiSliceRegistry {}

impl SaiSliceRegistry {
    pub fn shared(cx: &mut App) -> &mut Self {
        cx.default_global::<Self>()
    }

    pub fn register(slice: &Rc<dyn SaiSliceState>, cx: &mut App) {
        let this = Self::shared(cx);
        this.slices.insert(Self::object_identifier(slice), SaiWeakSliceState::new(slice));
        this.compact();
    }

    /// live slices are collected first
    /// a reset may build or drop other slices and re-enter the borrowed registry
    pub fn reset_all(cx: &mut App) {
        let live: Vec<Rc<dyn SaiSliceState>> = Self::shared(cx).slices.values().filter_map(|slot| slot.value.upgrade()).collect();
        for slice in &live {
            slice.reset_to_idle();
        }
        drop(live);
        Self::shared(cx).compact();
    }

    fn compact(&mut self) {
        self.slices.retain(|_, slot| slot.value.strong_count() > 0);
    }

    fn object_identifier(slice: &Rc<dyn SaiSliceState>) -> usize {
        Rc::as_ptr(slice).cast::<()>() as usize
    }
}

struct SaiTypedSliceState<T> {
    status: SaiWsSliceStatus,
    data: Option<T>,
    error: Option<String>,
    failure: Option<RiseFailure>,
}

/// typed slice, views read it during render so a read in a draw records the drawing view
/// a mutation notifies those views
pub struct SaiTypedSlice<T> {
    state: RefCell<SaiTypedSliceState<T>>,
    app: AsyncApp,
    refresh_scheduled: Rc<Cell<bool>>,
    readers: RefCell<Vec<EntityId>>,
    pending_readers: Rc<RefCell<Vec<EntityId>>>,
    redraws_windows: bool,
    revision: Cell<u64>,
}

impl<T: DeserializeOwned + 'static> SaiTypedSlice<T> {
    pub fn new(cx: &mut App) -> Rc<Self> {
        Self::build(true, cx)
    }

    /// a slice no view reads in render, eg a mutation whose result reaches the screen via callbacks
    /// its marks redraw nothing while a plain slice refreshes every window
    pub fn new_unobserved(cx: &mut App) -> Rc<Self> {
        Self::build(false, cx)
    }

    fn build(redraws_windows: bool, cx: &mut App) -> Rc<Self> {
        let slice = Rc::new(Self {
            state: RefCell::new(SaiTypedSliceState {
                status: SaiWsSliceStatus::Idle,
                data: None,
                error: None,
                failure: None,
            }),
            app: cx.to_async(),
            refresh_scheduled: Rc::new(Cell::new(false)),
            readers: RefCell::new(Vec::new()),
            pending_readers: Rc::new(RefCell::new(Vec::new())),
            redraws_windows,
            revision: Cell::new(0),
        });
        let registered: Rc<dyn SaiSliceState> = slice.clone();
        SaiSliceRegistry::register(&registered, cx);
        slice
    }

    fn decode(raw: Option<&Value>, take_path: Option<&str>) -> Option<T> {
        if TypeId::of::<T>() == TypeId::of::<SaiWireEmptyDTO>() {
            return serde_json::from_value::<T>(Value::Object(Map::new())).ok();
        }
        let extracted = SaiPayloadPath::extract(raw, take_path);
        SaiJSONCodec::decode::<T>(extracted)
    }

    fn did_change(&self) {
        self.revision.set(self.revision.get().wrapping_add(1));
        if !self.redraws_windows {
            return;
        }
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

impl<T> SaiTypedSlice<T> {
    fn note_read(&self) {
        if !self.redraws_windows {
            return;
        }
        if let Some(view) = gpui::rendering_view() {
            let mut readers = self.readers.borrow_mut();
            if !readers.contains(&view) {
                readers.push(view);
            }
        }
    }
}

impl<T> SaiTypedSlice<T> {
    /// desktop only, changes with every mark
    /// read in a draw it subscribes the view like data does without cloning the data
    pub fn revision(&self) -> u64 {
        self.note_read();
        self.revision.get()
    }
}

impl<T: Clone> SaiTypedSlice<T> {
    pub fn data(&self) -> Option<T> {
        self.note_read();
        self.state.borrow().data.clone()
    }
}

impl<T: DeserializeOwned + 'static> SaiSliceState for SaiTypedSlice<T> {
    fn status(&self) -> SaiWsSliceStatus {
        self.note_read();
        self.state.borrow().status
    }

    fn error(&self) -> Option<String> {
        self.note_read();
        self.state.borrow().error.clone()
    }

    fn failure(&self) -> Option<RiseFailure> {
        self.note_read();
        self.state.borrow().failure.clone()
    }

    fn is_pending(&self) -> bool {
        self.status() == SaiWsSliceStatus::Pending
    }

    fn mark_pending(&self) {
        {
            let mut state = self.state.borrow_mut();
            state.status = SaiWsSliceStatus::Pending;
            state.error = None;
            state.failure = None;
        }
        self.did_change();
    }

    fn mark_fulfilled(&self, raw: Option<&Value>, take_path: Option<&str>) {
        let data = Self::decode(raw, take_path);
        {
            let mut state = self.state.borrow_mut();
            state.status = SaiWsSliceStatus::Fulfilled;
            state.error = None;
            state.failure = None;
            state.data = data;
        }
        self.did_change();
    }

    fn mark_rejected(&self, error: &anyhow::Error) {
        {
            let mut state = self.state.borrow_mut();
            state.status = if state.data.is_none() {
                SaiWsSliceStatus::Rejected
            } else {
                SaiWsSliceStatus::Fulfilled
            };
            state.error = Some(CanonErrors::refusal_text(error));
            state.failure = CanonErrors::failure(error);
        }
        self.did_change();
    }

    fn reset_to_idle(&self) {
        {
            let mut state = self.state.borrow_mut();
            state.status = SaiWsSliceStatus::Idle;
            state.data = None;
            state.error = None;
            state.failure = None;
        }
        self.did_change();
    }

    fn callback_payload(&self, raw: Option<&Value>, take_path: Option<&str>) -> Option<Box<dyn Any>> {
        let extracted = SaiPayloadPath::extract(raw, take_path);
        SaiJSONCodec::decode::<T>(extracted).map(|decoded| Box::new(decoded) as Box<dyn Any>)
    }
}
