use std::cell::Cell;
use std::rc::{Rc, Weak};

use gpui::{App, Global};

/// phase of one drag or scroll session
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BottomSheetGestureState {
    #[default]
    Possible,
    Began,
    Changed,
    Ended,
    Cancelled,
    Failed,
}

/// desktop stand in for the gesture recognizer a gate claims an axis with
/// the gate keeps it while it lives and moves its state as the session goes
#[derive(Debug, Default)]
pub struct BottomSheetGestureRecognizer {
    state: Rc<Cell<BottomSheetGestureState>>,
}

impl BottomSheetGestureRecognizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> BottomSheetGestureState {
        self.state.get()
    }

    pub fn set_state(&self, state: BottomSheetGestureState) {
        self.state.set(state);
    }

    fn is_same(&self, owner: &Weak<Cell<BottomSheetGestureState>>) -> bool {
        Weak::ptr_eq(&Rc::downgrade(&self.state), owner)
    }
}

#[derive(Default)]
struct AxisOwners {
    horizontal_owner: Weak<Cell<BottomSheetGestureState>>,
    vertical_owner: Weak<Cell<BottomSheetGestureState>>,
}

impl Global for AxisOwners {}

/// namespace of the arbiter
/// owners live in an app global
pub enum BottomSheetAxisOwnership {}

impl BottomSheetAxisOwnership {
    pub fn horizontal_is_driving(cx: &App) -> bool {
        cx.try_global::<AxisOwners>()
            .is_some_and(|owners| Self::is_driving(&owners.horizontal_owner))
    }

    pub fn vertical_is_driving(cx: &App) -> bool {
        cx.try_global::<AxisOwners>()
            .is_some_and(|owners| Self::is_driving(&owners.vertical_owner))
    }

    pub fn claim_horizontal(recognizer: &BottomSheetGestureRecognizer, cx: &mut App) {
        cx.default_global::<AxisOwners>().horizontal_owner = Rc::downgrade(&recognizer.state);
    }

    pub fn release_horizontal(recognizer: &BottomSheetGestureRecognizer, cx: &mut App) {
        let owners = cx.default_global::<AxisOwners>();
        if !recognizer.is_same(&owners.horizontal_owner) {
            return;
        }
        owners.horizontal_owner = Weak::new();
    }

    pub fn claim_vertical(recognizer: &BottomSheetGestureRecognizer, cx: &mut App) {
        cx.default_global::<AxisOwners>().vertical_owner = Rc::downgrade(&recognizer.state);
    }

    pub fn release_vertical(recognizer: &BottomSheetGestureRecognizer, cx: &mut App) {
        let owners = cx.default_global::<AxisOwners>();
        if !recognizer.is_same(&owners.vertical_owner) {
            return;
        }
        owners.vertical_owner = Weak::new();
    }

    fn is_driving(recognizer: &Weak<Cell<BottomSheetGestureState>>) -> bool {
        let Some(recognizer) = recognizer.upgrade() else {
            return false;
        };
        match recognizer.get() {
            BottomSheetGestureState::Began | BottomSheetGestureState::Changed => true,
            BottomSheetGestureState::Possible
            | BottomSheetGestureState::Ended
            | BottomSheetGestureState::Cancelled
            | BottomSheetGestureState::Failed => false,
        }
    }
}
