use std::rc::Rc;

use gpui::{App, Pixels};
use rise_core::foundation::ns_range::NSRange;

use crate::input_ui::input_ui_state::InputUiState;

pub type ShouldChangeTextIn = Rc<dyn Fn(NSRange, &str, &mut App) -> bool>;
pub type DidChangeText = Rc<dyn Fn(&InputUiState, &mut App)>;
pub type DidChangeSelection = Rc<dyn Fn(NSRange, &mut App)>;
/// callbacks for begin and end of editing
pub type DidChangeEditing = Rc<dyn Fn(&mut App)>;
pub type DidChangeHeight = Rc<dyn Fn(Pixels, &mut App)>;

/// the delegate slots
/// calls firing while the field lays out (focus and height) wait until the end of the frame
#[derive(Clone, Default)]
pub struct InputUiTextViewDelegate {
    pub should_change_text_in: Option<ShouldChangeTextIn>,
    pub did_change: Option<DidChangeText>,
    pub did_change_selection: Option<DidChangeSelection>,
    pub did_begin_editing: Option<DidChangeEditing>,
    pub did_end_editing: Option<DidChangeEditing>,
    pub did_change_height: Option<DidChangeHeight>,
}

impl InputUiTextViewDelegate {
    pub fn should_change_text_in(
        mut self,
        f: impl Fn(NSRange, &str, &mut App) -> bool + 'static,
    ) -> Self {
        self.should_change_text_in = Some(Rc::new(f));
        self
    }

    pub fn did_change(mut self, f: impl Fn(&InputUiState, &mut App) + 'static) -> Self {
        self.did_change = Some(Rc::new(f));
        self
    }

    pub fn did_change_selection(mut self, f: impl Fn(NSRange, &mut App) + 'static) -> Self {
        self.did_change_selection = Some(Rc::new(f));
        self
    }

    pub fn did_begin_editing(mut self, f: impl Fn(&mut App) + 'static) -> Self {
        self.did_begin_editing = Some(Rc::new(f));
        self
    }

    pub fn did_end_editing(mut self, f: impl Fn(&mut App) + 'static) -> Self {
        self.did_end_editing = Some(Rc::new(f));
        self
    }

    pub fn did_change_height(mut self, f: impl Fn(Pixels, &mut App) + 'static) -> Self {
        self.did_change_height = Some(Rc::new(f));
        self
    }
}
