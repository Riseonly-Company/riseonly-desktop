use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Div, ElementId, Entity, Focusable, IntoElement, ParentElement, Pixels, RenderOnce, Styled,
    Window, div, px,
};
use rise_core::foundation::ns_range::NSRange;

use super::rise_document::RiseDocumentRun;
use super::rise_document_attributed_text::{
    RiseDocumentAttributedRun, RiseDocumentAttributedString, RiseDocumentAttributedText, RiseDocumentAttributes,
    RiseDocumentTextStyle,
};
use super::rise_document_editor_row::RiseDocumentEditorRowKind;
use crate::input_ui::{InputUiState, InputUiTextViewDelegate, RunAttributes};
use crate::swiftui::rich_text::AttributeContainer;

type RunsChange = Rc<dyn Fn(Vec<RiseDocumentRun>, &mut App)>;
type VerbatimChange = Rc<dyn Fn(String, &mut App)>;
type SelectionChange = Rc<dyn Fn(NSRange, &mut App)>;
type Split = Rc<dyn Fn(usize, &mut App)>;
type BackspaceAtStart = Rc<dyn Fn(&mut App)>;
type Focus = Rc<dyn Fn(&mut App)>;
type HeightChange = Rc<dyn Fn(f32, &mut App)>;

/// text view of one editor row, one view per block
/// one view for the document would reassign its string on every keystroke
/// and fight the input system, selection cant span two blocks
#[derive(Clone, IntoElement)]
pub struct RiseBlockTextView {
    id: ElementId,
    pub identity: String,
    pub kind: RiseDocumentEditorRowKind,
    pub runs: Vec<RiseDocumentRun>,
    pub verbatim: String,
    pub style: RiseDocumentTextStyle,
    pub placeholder: String,
    pub is_focused: bool,

    on_runs_change: Option<RunsChange>,
    on_verbatim_change: Option<VerbatimChange>,
    on_selection_change: Option<SelectionChange>,
    on_split: Option<Split>,
    on_backspace_at_start: Option<BackspaceAtStart>,
    on_focus: Option<Focus>,
    on_height_change: Option<HeightChange>,
}

impl RiseBlockTextView {
    /// id keys the text view across renders, identity only decides when to reinstall
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        identity: impl Into<String>,
        kind: RiseDocumentEditorRowKind,
        runs: Vec<RiseDocumentRun>,
        verbatim: impl Into<String>,
        style: RiseDocumentTextStyle,
        placeholder: impl Into<String>,
        is_focused: bool,
    ) -> Self {
        Self {
            id: id.into(),
            identity: identity.into(),
            kind,
            runs,
            verbatim: verbatim.into(),
            style,
            placeholder: placeholder.into(),
            is_focused,
            on_runs_change: None,
            on_verbatim_change: None,
            on_selection_change: None,
            on_split: None,
            on_backspace_at_start: None,
            on_focus: None,
            on_height_change: None,
        }
    }

    pub fn on_runs_change(mut self, f: impl Fn(Vec<RiseDocumentRun>, &mut App) + 'static) -> Self {
        self.on_runs_change = Some(Rc::new(f));
        self
    }

    pub fn on_verbatim_change(mut self, f: impl Fn(String, &mut App) + 'static) -> Self {
        self.on_verbatim_change = Some(Rc::new(f));
        self
    }

    pub fn on_selection_change(mut self, f: impl Fn(NSRange, &mut App) + 'static) -> Self {
        self.on_selection_change = Some(Rc::new(f));
        self
    }

    /// utf16 location the row splits at when Return is typed
    pub fn on_split(mut self, f: impl Fn(usize, &mut App) + 'static) -> Self {
        self.on_split = Some(Rc::new(f));
        self
    }

    pub fn on_backspace_at_start(mut self, f: impl Fn(&mut App) + 'static) -> Self {
        self.on_backspace_at_start = Some(Rc::new(f));
        self
    }

    pub fn on_focus(mut self, f: impl Fn(&mut App) + 'static) -> Self {
        self.on_focus = Some(Rc::new(f));
        self
    }

    pub fn on_height_change(mut self, f: impl Fn(f32, &mut App) + 'static) -> Self {
        self.on_height_change = Some(Rc::new(f));
        self
    }
}

/// coordinator memory
/// kept outside the entity because the field calls its delegate synchronously while updating
/// and the delegate reads the latest parent from here
struct RiseBlockTextViewCoordinatorState {
    parent: RiseBlockTextView,
    installed_identity: Option<String>,
    installed_text: String,
    installed_kind: Option<RiseDocumentEditorRowKind>,
}

/// owns the field and its delegate for the life of the row
#[derive(Clone)]
pub struct RiseBlockTextViewCoordinator {
    field: Entity<InputUiState>,
    state: Rc<RefCell<RiseBlockTextViewCoordinatorState>>,
}

impl RiseBlockTextView {
    fn make_coordinator(&self) -> Rc<RefCell<RiseBlockTextViewCoordinatorState>> {
        Rc::new(RefCell::new(RiseBlockTextViewCoordinatorState {
            parent: self.clone(),
            installed_identity: None,
            installed_text: String::new(),
            installed_kind: None,
        }))
    }

    fn make_ui_view(&self, cx: &mut App) -> RiseBlockTextViewCoordinator {
        let state = self.make_coordinator();
        let delegate = RiseBlockTextViewCoordinator::delegate(&state);
        let field = cx.new(|cx| {
            let mut field = InputUiState::multi_line(cx);
            field.set_chromeless(true, cx);
            field.set_delegate(delegate);
            field
        });
        let coordinator = RiseBlockTextViewCoordinator { field, state };
        coordinator.install(self, cx);
        coordinator
    }
}

impl RiseBlockTextViewCoordinator {
    fn install(&self, parent: &RiseBlockTextView, cx: &mut App) {
        let attributed = if parent.kind.is_verbatim() {
            RiseDocumentAttributedString::with_attributes(
                parent.verbatim.clone(),
                AttributeContainer {
                    font: Some(parent.style.mono),
                    foreground_color: Some(parent.style.text_color),
                    ..AttributeContainer::default()
                },
            )
        } else {
            RiseDocumentAttributedText::attributed(&parent.runs, &parent.kind, &parent.style)
        };
        let pieces: Vec<(String, RunAttributes)> = attributed
            .runs
            .iter()
            .filter_map(|run| {
                let piece = attributed.string.get(run.range.clone())?.to_owned();
                Some((piece, run_attributes(run)))
            })
            .collect();
        let typing = RunAttributes::new(AttributeContainer {
            font: Some(parent.style.font(&parent.kind)),
            foreground_color: Some(parent.style.text_color),
            ..AttributeContainer::default()
        });
        {
            let mut state = self.state.borrow_mut();
            state.installed_identity = Some(parent.identity.clone());
            state.installed_text = plain_text(parent);
            state.installed_kind = Some(parent.kind.clone());
        }
        self.field
            .update(cx, |field, cx| field.set_attributed_text(pieces, typing, cx));
    }

    fn sync_if_needed(&self, parent: &RiseBlockTextView, cx: &mut App) {
        let same = {
            let state = self.state.borrow();
            state.installed_identity.as_deref() == Some(parent.identity.as_str())
                && state.installed_text == plain_text(parent)
                && state.installed_kind.as_ref() == Some(&parent.kind)
        };
        if same {
            return;
        }
        self.install(parent, cx);
    }

    fn update_ui_view(&self, parent: &RiseBlockTextView, window: &mut Window, cx: &mut App) {
        self.state.borrow_mut().parent = parent.clone();
        if self.field.read(cx).is_composing() {
            return;
        }
        self.sync_if_needed(parent, cx);
        let handle = self.field.read(cx).focus_handle(cx);
        if parent.is_focused && !handle.is_focused(window) {
            window.defer(cx, move |window, cx| handle.focus(window, cx));
        }
    }

    fn install_placeholder(text: &str, style: &RiseDocumentTextStyle) -> Div {
        let body = style.body;
        let label = div()
            .absolute()
            .top_0()
            .left_0()
            .whitespace_nowrap()
            .truncate()
            .text_size(px(body.size))
            .font_weight(body.weight)
            .text_color(style.secondary_color)
            .child(text.to_owned());
        if body.italic { label.italic() } else { label }
    }

    fn refresh_placeholder(field: &InputUiState, text: &str) -> bool {
        field.text().is_empty() && !text.is_empty()
    }

    /// reports the fitted height rounded up, once it moved
    /// the field measures and dedupes
    fn publish_height(state: &Rc<RefCell<RiseBlockTextViewCoordinatorState>>, height: Pixels, cx: &mut App) {
        let on_height_change = state.borrow().parent.on_height_change.clone();
        if let Some(on_height_change) = on_height_change {
            on_height_change(f32::from(height).ceil(), cx);
        }
    }

    /// should change text in range with replacement hook
    fn text_view(
        state: &Rc<RefCell<RiseBlockTextViewCoordinatorState>>,
        range: NSRange,
        text: &str,
        cx: &mut App,
    ) -> bool {
        if text == "\n" {
            let on_split = state.borrow().parent.on_split.clone();
            if let Some(on_split) = on_split {
                on_split(range.location, cx);
            }
            return false;
        }
        if text.is_empty() && range.length == 0 && range.location == 0 {
            let on_backspace_at_start = state.borrow().parent.on_backspace_at_start.clone();
            if let Some(on_backspace_at_start) = on_backspace_at_start {
                on_backspace_at_start(cx);
            }
            return false;
        }
        true
    }

    fn text_view_did_change(
        state: &Rc<RefCell<RiseBlockTextViewCoordinatorState>>,
        field: &InputUiState,
        cx: &mut App,
    ) {
        if field.is_composing() {
            return;
        }
        let (is_verbatim, on_runs_change, on_verbatim_change) = {
            let mut state = state.borrow_mut();
            state.installed_text = field.text().to_owned();
            (
                state.parent.kind.is_verbatim(),
                state.parent.on_runs_change.clone(),
                state.parent.on_verbatim_change.clone(),
            )
        };
        if is_verbatim {
            if let Some(on_verbatim_change) = on_verbatim_change {
                on_verbatim_change(field.text().to_owned(), cx);
            }
        } else if let Some(on_runs_change) = on_runs_change {
            on_runs_change(RiseDocumentAttributedText::runs(&attributed_text(field)), cx);
        }
    }

    fn text_view_did_change_selection(
        state: &Rc<RefCell<RiseBlockTextViewCoordinatorState>>,
        range: NSRange,
        cx: &mut App,
    ) {
        let on_selection_change = state.borrow().parent.on_selection_change.clone();
        if let Some(on_selection_change) = on_selection_change {
            cx.defer(move |cx| on_selection_change(range, cx));
        }
    }

    fn text_view_did_begin_editing(state: &Rc<RefCell<RiseBlockTextViewCoordinatorState>>, cx: &mut App) {
        let on_focus = state.borrow().parent.on_focus.clone();
        if let Some(on_focus) = on_focus {
            on_focus(cx);
        }
    }

    /// the delegate slots, each forwards to the method of the same name
    fn delegate(state: &Rc<RefCell<RiseBlockTextViewCoordinatorState>>) -> InputUiTextViewDelegate {
        let should_change = Rc::clone(state);
        let did_change = Rc::clone(state);
        let did_change_selection = Rc::clone(state);
        let did_begin_editing = Rc::clone(state);
        let did_change_height = Rc::clone(state);
        InputUiTextViewDelegate::default()
            .should_change_text_in(move |range, text, cx| Self::text_view(&should_change, range, text, cx))
            .did_change(move |field, cx| Self::text_view_did_change(&did_change, field, cx))
            .did_change_selection(move |range, cx| {
                Self::text_view_did_change_selection(&did_change_selection, range, cx)
            })
            .did_begin_editing(move |cx| Self::text_view_did_begin_editing(&did_begin_editing, cx))
            .did_change_height(move |height, cx| Self::publish_height(&did_change_height, height, cx))
    }
}

impl RenderOnce for RiseBlockTextView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let parent = self.clone();
        let coordinator = window.use_keyed_state(self.id.clone(), cx, move |_, cx| parent.make_ui_view(cx));
        let coordinator = coordinator.read(cx).clone();
        coordinator.update_ui_view(&self, window, cx);
        let shows_placeholder =
            RiseBlockTextViewCoordinator::refresh_placeholder(coordinator.field.read(cx), &self.placeholder);
        let placeholder = RiseBlockTextViewCoordinator::install_placeholder(&self.placeholder, &self.style);
        div()
            .relative()
            .w_full()
            .min_w_0()
            .child(coordinator.field)
            .when(shows_placeholder, |view| view.child(placeholder))
    }
}
fn plain_text(parent: &RiseBlockTextView) -> String {
    if parent.kind.is_verbatim() {
        parent.verbatim.clone()
    } else {
        parent.runs.iter().map(|run| run.text.as_str()).collect()
    }
}

fn run_attributes(run: &RiseDocumentAttributedRun) -> RunAttributes {
    if run.document == RiseDocumentAttributes::default() {
        RunAttributes::new(run.attributes.clone())
    } else {
        RunAttributes::tagged(run.attributes.clone(), run.document.clone())
    }
}

/// the field runs read back with their document attributes
fn attributed_text(field: &InputUiState) -> RiseDocumentAttributedString {
    let mut out = RiseDocumentAttributedString::default();
    for (piece, attributes) in field.attributed_pieces() {
        let start = out.string.len();
        out.string.push_str(piece);
        out.runs.push(RiseDocumentAttributedRun {
            range: start..out.string.len(),
            attributes: attributes.attributes.clone(),
            document: attributes.tag::<RiseDocumentAttributes>().cloned().unwrap_or_default(),
        });
    }
    out
}
