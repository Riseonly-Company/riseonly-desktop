use std::ops::Range;
use std::time::Duration;

use gpui::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    Pixels, Point, SharedString, UTF16Selection, Window, point, px,
};

use crate::input_ui::caret_blink::{BLINK_INTERVAL, CaretBlink, PAUSE_AFTER_EDIT};
use crate::input_ui::display_text::DisplayText;
use crate::input_ui::edit_history::{Edit, EditHistory, EditKind};
use crate::input_ui::input_actions::{self, install_key_bindings};
use crate::input_ui::shaped_input::ShapedInput;
use crate::input_ui::text_boundaries;

/// Whether the field is one line or many.
///
/// This is not only a layout choice. It decides what Enter does, whether the
/// arrow keys are consumed or left to the list behind the field, and what
/// happens to the newlines in a paste.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InputMode {
    /// A field. Enter emits [`InputUiEvent::Submitted`], pasted line breaks
    /// collapse to spaces, and the vertical arrows are propagated.
    #[default]
    SingleLine,
    /// A composer. Enter inserts a line break and the arrows move by line.
    MultiLine,
}

impl InputMode {
    pub fn is_multiline(self) -> bool {
        matches!(self, Self::MultiLine)
    }
}

/// An anchor and a head, never a sorted pair.
///
/// `start`/`end` are derived on demand. Storing them instead would lose which
/// end the keyboard is moving, and shift+left followed by shift+right would grow
/// the selection from the wrong side.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    /// An empty selection — a caret — at a byte offset.
    pub fn cursor(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    /// A selection from where it was dropped to where it is being dragged.
    /// Offsets are bytes into the document and need not be ordered.
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// The lower of the two offsets.
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// The higher of the two offsets.
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// The selected bytes, ordered — safe to slice the document with.
    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }

    /// Whether this is a bare caret rather than a range.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Whether the moving end is before the fixed one, i.e. the selection was
    /// grown leftwards. Needed by the IME, which is told which way it points.
    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }
}

/// What the field reports to whoever is listening.
///
/// Subscribe with `cx.subscribe(&field, ..)`. The field never calls back into a
/// parent directly — a search box that filtered its own list would only work for
/// the first screen that used it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputUiEvent {
    /// The document changed: typing, paste, undo, a programmatic
    /// [`InputUiState::set_text`]. Fires per edit, so debounce anything
    /// expensive hanging off it.
    Changed,
    /// Enter in a single-line field. A composer emits a line break instead.
    Submitted,
    /// Escape, with no composition to cancel first.
    Cancelled,
}

/// What one selection gesture extends by.
///
/// Fixed at mouse-down from the click count and held for the whole drag: a drag
/// that began as a double click keeps taking whole words, which is what makes
/// dragging back over the first word feel right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectGranularity {
    /// Single click.
    Character,
    /// Double click.
    Word,
    /// Triple click.
    Line,
}

#[derive(Clone, Debug)]
struct Drag {
    granularity: SelectGranularity,
    anchor: Range<usize>,
}

/// The document as it was when the input method started composing.
///
/// The IME sends one `replace_and_mark_text_in_range` per keystroke of a
/// composition; none of those is an undo step. This is what lets the commit be
/// recorded as a single edit spanning the whole composition, and what lets a
/// cancelled composition leave no trace at all.
#[derive(Clone, Debug)]
struct Composition {
    at: usize,
    removed: String,
    selection_before: Range<usize>,
}

/// Everything a text field is, apart from how it is drawn.
///
/// The document, the selection, the undo history, the IME composition, the caret
/// blink and the scroll offset live here; [`InputUi`](crate::InputUi) is the
/// element that paints them. Hold it as a `gpui::Entity<InputUiState>` and it
/// outlives the frame — a field rebuilt on every render would lose its selection
/// and its history on the first keystroke.
///
/// It implements `gpui::EntityInputHandler`, so the platform IME talks to it
/// directly. That is what makes dead keys, Cyrillic, CJK composition and the
/// character palette work without this crate parsing key events itself.
///
/// Offsets in the public API are byte offsets into the UTF-8 document and are
/// always on a char boundary; the UTF-16 offsets the IME speaks are converted at
/// the boundary. Motion is by grapheme, so one press of the left arrow crosses a
/// whole emoji rather than one of its code points.
///
/// It never reads the theme and never touches the clipboard except through the
/// action methods, so it can be driven headlessly in tests.
pub struct InputUiState {
    mode: InputMode,
    text: String,
    selection: Selection,
    marked: Option<Range<usize>>,
    composition: Option<Composition>,
    placeholder: SharedString,
    secure: bool,
    disabled: bool,
    height: Option<Pixels>,
    max_length: Option<usize>,
    max_visible_lines: usize,
    history: EditHistory,
    blink: CaretBlink,
    focus_handle: FocusHandle,
    focused: bool,
    revision: u64,
    display: DisplayText,
    drag: Option<Drag>,
    goal_x: Option<Pixels>,
    scroll: Point<Pixels>,
    line_height: Pixels,
    bounds: Option<Bounds<Pixels>>,
    shaped: Option<ShapedInput>,
}

impl EventEmitter<InputUiEvent> for InputUiState {}

impl Focusable for InputUiState {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl InputUiState {
    /// An empty field.
    ///
    /// Installs this crate's key bindings on first use, so a field works in an
    /// app that has never configured a keymap.
    pub fn new(mode: InputMode, cx: &mut Context<Self>) -> Self {
        install_key_bindings(cx);

        let placeholder = SharedString::default();
        Self {
            mode,
            text: String::new(),
            selection: Selection::default(),
            marked: None,
            composition: None,
            display: DisplayText::new("", &placeholder, false, mode.is_multiline()),
            placeholder,
            secure: false,
            disabled: false,
            height: None,
            max_length: None,
            max_visible_lines: 8,
            history: EditHistory::default(),
            blink: CaretBlink::default(),
            focus_handle: cx.focus_handle(),
            focused: false,
            revision: 0,
            drag: None,
            goal_x: None,
            scroll: point(px(0.), px(0.)),
            line_height: px(0.),
            bounds: None,
            shaped: None,
        }
    }

    /// An empty [`InputMode::SingleLine`] field.
    pub fn single_line(cx: &mut Context<Self>) -> Self {
        Self::new(InputMode::SingleLine, cx)
    }

    /// An empty [`InputMode::MultiLine`] composer.
    pub fn multi_line(cx: &mut Context<Self>) -> Self {
        Self::new(InputMode::MultiLine, cx)
    }

    pub fn mode(&self) -> InputMode {
        self.mode
    }

    /// The document as typed — the real characters even in a secure field, and
    /// without the placeholder. What is drawn is [`InputUiState::display`].
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// The range the IME is still composing in, if any.
    ///
    /// Its presence means the text inside is provisional: Escape takes it back
    /// out, and it is not yet an undo step of its own.
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn placeholder(&self) -> &SharedString {
        &self.placeholder
    }

    pub fn is_secure(&self) -> bool {
        self.secure
    }

    /// A height this field alone uses, or `None` for the theme's.
    ///
    /// Exists so a screen can make its field and the button under it one size —
    /// the button takes the same token, so the two cannot drift apart.
    pub fn height(&self) -> Option<Pixels> {
        self.height
    }

    pub fn set_height(&mut self, height: Option<Pixels>, cx: &mut Context<Self>) {
        if self.height == height {
            return;
        }
        self.height = height;
        cx.notify();
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn max_length(&self) -> Option<usize> {
        self.max_length
    }

    pub fn max_visible_lines(&self) -> usize {
        self.max_visible_lines
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// The text shown while the document is empty.
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        self.invalidate_display(cx);
    }

    /// Turns the field into a password field: bullets on screen, one per
    /// grapheme, and the contents stop leaving through copy, cut, the IME's
    /// `text_for_range` and the character palette.
    pub fn set_secure(&mut self, secure: bool, cx: &mut Context<Self>) {
        self.secure = secure;
        self.invalidate_display(cx);
    }

    /// Refuses every edit — typing, IME, paste, undo — while keeping the text
    /// on screen and selectable.
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        cx.notify();
    }

    /// A limit in characters, not bytes — a limit of 5 holds five Cyrillic
    /// letters, not two and a half.
    ///
    /// Enforced on every insertion, so a paste is truncated rather than
    /// rejected, and replacing a full selection with the same length still fits.
    pub fn set_max_length(&mut self, max_length: Option<usize>, cx: &mut Context<Self>) {
        self.max_length = max_length;
        cx.notify();
    }

    /// How tall a composer is allowed to grow before it scrolls instead.
    /// Clamped to at least one line.
    pub fn set_max_visible_lines(&mut self, lines: usize, cx: &mut Context<Self>) {
        self.max_visible_lines = lines.max(1);
        cx.notify();
    }

    /// Replaces the whole document as one undoable step.
    pub fn set_text(&mut self, text: impl AsRef<str>, cx: &mut Context<Self>) {
        let whole = 0..self.text.len();
        self.replace(whole, text.as_ref(), EditKind::Atomic, cx);
    }

    /// Replaces the whole document and forgets that anything came before it.
    pub fn reset(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let incoming = text.into();
        self.text = self.fit(&incoming, &(0..self.text.len()));
        self.selection = Selection::cursor(self.text.len());
        self.marked = None;
        self.composition = None;
        self.history.clear();
        self.scroll = point(px(0.), px(0.));
        self.after_edit(cx);
    }

    /// Empties the field as one undoable step. To empty it unundoably, use
    /// [`InputUiState::reset`].
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    /// Moves the caret or selection. Offsets are clamped to char boundaries, so
    /// an offset computed from a byte length is safe to pass.
    pub fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selection = Selection::new(
            text_boundaries::clamp_to_char_boundary(&self.text, selection.anchor),
            text_boundaries::clamp_to_char_boundary(&self.text, selection.head),
        );
        self.history.break_coalescing();
        cx.notify();
    }

    /// Selects the whole document. The programmatic form of
    /// [`InputUiState::select_all`], which is the bound action.
    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.selection = Selection::new(0, self.text.len());
        self.history.break_coalescing();
        cx.notify();
    }

    /// Types text at the selection, replacing it — an emoji picker, a mention
    /// completion. One undo step, and subject to the char limit.
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        self.replace(self.selection.range(), text, EditKind::Atomic, cx);
    }

    /// Whether the caret should be painted this frame. Off during the blink's
    /// dark half, and permanently off while disabled.
    pub fn caret_is_visible(&self) -> bool {
        self.blink.is_visible() && !self.disabled
    }

    /// What is actually on screen: the placeholder when empty, bullets when
    /// secure, split into display lines. Never the raw document.
    pub fn display(&self) -> &DisplayText {
        &self.display
    }

    pub fn scroll(&self) -> Point<Pixels> {
        self.scroll
    }

    pub fn line_height(&self) -> Pixels {
        self.line_height
    }

    /// The shaped rows from the last prepaint, or `None` before the first one.
    /// Everything that maps between pixels and offsets needs it.
    pub fn shaped(&self) -> Option<&ShapedInput> {
        self.shaped.as_ref()
    }

    /// Bumped on every change to the document or to what is displayed.
    ///
    /// Shaping is keyed on it, so an edit invalidates the shaped rows while a
    /// caret move or a repaint reuses them.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    // ---- element plumbing -------------------------------------------------

    /// Hands back the rows shaped during prepaint. Called by
    /// [`InputUi`](crate::InputUi); a call site has no reason to.
    pub fn set_shaped(&mut self, shaped: ShapedInput) {
        self.shaped = Some(shaped);
    }

    /// Records where the field landed and how tall a line is, which is what
    /// turns a window-space click into a document offset.
    pub fn set_viewport(&mut self, bounds: Bounds<Pixels>, line_height: Pixels) {
        self.bounds = Some(bounds);
        self.line_height = line_height;
    }

    /// Stores the scroll offset the element computed to keep the caret in view.
    pub fn set_scroll(&mut self, scroll: Point<Pixels>) {
        self.scroll = scroll;
    }

    /// Called from prepaint, which is why it returns early on no change: a
    /// `notify` on every frame would keep the window permanently dirty.
    pub fn sync_focus(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if focused {
            let epoch = self.blink.focus();
            self.schedule_resume(epoch, cx);
        } else {
            self.blink.blur();
            self.drag = None;
        }
        cx.notify();
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// The display line the caret is on — the row a composer scrolls to.
    pub fn caret_line(&self) -> usize {
        self.display.line_index_for_source(self.selection.head)
    }

    /// The document offset under a window-space point.
    ///
    /// Clamped to the shaped window: a pointer dragged above or below the field
    /// resolves to the nearest shaped row, the selection extends there, and the
    /// next frame scrolls one row further. That is a row per pointer event
    /// rather than continuous autoscroll, and it is deliberate — no timer runs
    /// while the pointer sits still outside the field.
    pub fn offset_at(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(shaped)) = (self.bounds, self.shaped.as_ref()) else {
            return self.selection.head;
        };
        if self.display.is_placeholder {
            return 0;
        }

        let visible = shaped.visible();
        let local_y = position.y - bounds.top() + self.scroll.y;
        let row = if self.line_height > px(0.) {
            (f32::from(local_y) / f32::from(self.line_height)).floor()
        } else {
            0.0
        };
        let row = if row < 0.0 { 0 } else { row as usize };
        let line_index = row
            .min(self.display.line_count().saturating_sub(1))
            .clamp(visible.start, visible.end.saturating_sub(1));

        let x = position.x - bounds.left() + self.scroll.x;
        let offset_in_line = shaped.offset_for_x(line_index, x);
        self.display
            .source_offset_within_line(line_index, offset_in_line)
    }

    // ---- mouse ------------------------------------------------------------

    /// Places the caret, or selects the word or line under the pointer, and
    /// starts a drag at that granularity.
    ///
    /// `click_count` comes from gpui's `MouseDownEvent`; anything above three
    /// keeps selecting by line. `extend` is the shift-click case: the anchor
    /// stays put and only the head moves.
    pub fn on_mouse_down(
        &mut self,
        position: Point<Pixels>,
        click_count: usize,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let offset = self.offset_at(position);
        self.history.break_coalescing();

        match click_count {
            0 | 1 => {
                if extend {
                    self.selection.head = offset;
                } else {
                    self.selection = Selection::cursor(offset);
                }
                self.drag = Some(Drag {
                    granularity: SelectGranularity::Character,
                    anchor: offset..offset,
                });
            }
            2 => {
                let word = text_boundaries::word_range_at(&self.text, offset);
                self.selection = Selection::new(word.start, word.end);
                self.drag = Some(Drag {
                    granularity: SelectGranularity::Word,
                    anchor: word,
                });
            }
            _ => {
                let line = text_boundaries::line_range_at(&self.text, offset);
                self.selection = Selection::new(line.start, line.end);
                self.drag = Some(Drag {
                    granularity: SelectGranularity::Line,
                    anchor: line,
                });
            }
        }

        self.goal_x = None;
        let epoch = self.blink.interrupt();
        self.schedule_resume(epoch, cx);
        cx.notify();
    }

    /// Extends the live selection to the pointer, in whole words or lines when
    /// the drag started as a double or triple click.
    ///
    /// A no-op when no drag is in flight, and silent when the selection has not
    /// actually changed — a repaint per mouse-move event would be a repaint too
    /// many.
    pub fn on_mouse_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let offset = self.offset_at(position);
        let extent = match drag.granularity {
            SelectGranularity::Character => offset..offset,
            SelectGranularity::Word => text_boundaries::word_range_at(&self.text, offset),
            SelectGranularity::Line => text_boundaries::line_range_at(&self.text, offset),
        };

        let selection = if extent.start < drag.anchor.start {
            Selection::new(drag.anchor.end, extent.start)
        } else {
            Selection::new(drag.anchor.start, extent.end.max(drag.anchor.end))
        };
        if selection == self.selection {
            return;
        }
        self.selection = selection;
        cx.notify();
    }

    /// Ends the drag. The selection stays as it was left.
    pub fn on_mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    /// Whether a selection drag is in flight — the element captures the mouse
    /// while it is.
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    // ---- actions ----------------------------------------------------------
    //
    // Every method below is a gpui action handler: it takes the action, the
    // window and the context, and is wired up with `.on_action(..)`. They are
    // public so a toolbar button or a menu item can invoke the same behaviour
    // the keystroke does — `state.undo(&Undo, window, cx)`.

    /// Caret one grapheme left, collapsing a selection to its start.
    pub fn move_left(
        &mut self,
        _: &input_actions::MoveLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.horizontal(false, false, cx);
    }

    /// Caret one grapheme right, collapsing a selection to its end.
    pub fn move_right(
        &mut self,
        _: &input_actions::MoveRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.horizontal(true, false, cx);
    }

    /// Grows the selection one grapheme left, keeping the anchor.
    pub fn select_left(
        &mut self,
        _: &input_actions::SelectLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.horizontal(false, true, cx);
    }

    /// Grows the selection one grapheme right, keeping the anchor.
    pub fn select_right(
        &mut self,
        _: &input_actions::SelectRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.horizontal(true, true, cx);
    }

    /// Caret to the previous word boundary.
    pub fn move_word_left(
        &mut self,
        _: &input_actions::MoveWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::previous_word_boundary(&self.text, self.selection.head);
        self.move_caret(target, false, cx);
    }

    /// Caret to the next word boundary.
    pub fn move_word_right(
        &mut self,
        _: &input_actions::MoveWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::next_word_boundary(&self.text, self.selection.head);
        self.move_caret(target, false, cx);
    }

    /// Grows the selection to the previous word boundary.
    pub fn select_word_left(
        &mut self,
        _: &input_actions::SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::previous_word_boundary(&self.text, self.selection.head);
        self.move_caret(target, true, cx);
    }

    /// Grows the selection to the next word boundary.
    pub fn select_word_right(
        &mut self,
        _: &input_actions::SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::next_word_boundary(&self.text, self.selection.head);
        self.move_caret(target, true, cx);
    }

    /// Caret to the start of the line it is on.
    pub fn move_line_start(
        &mut self,
        _: &input_actions::MoveLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::line_start(&self.text, self.selection.head);
        self.move_caret(target, false, cx);
    }

    /// Caret to the end of the line it is on.
    pub fn move_line_end(
        &mut self,
        _: &input_actions::MoveLineEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::line_end(&self.text, self.selection.head);
        self.move_caret(target, false, cx);
    }

    /// Grows the selection to the start of the line.
    pub fn select_line_start(
        &mut self,
        _: &input_actions::SelectLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::line_start(&self.text, self.selection.head);
        self.move_caret(target, true, cx);
    }

    /// Grows the selection to the end of the line.
    pub fn select_line_end(
        &mut self,
        _: &input_actions::SelectLineEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::line_end(&self.text, self.selection.head);
        self.move_caret(target, true, cx);
    }

    /// Caret to the very beginning of the document.
    pub fn move_document_start(
        &mut self,
        _: &input_actions::MoveDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_caret(0, false, cx);
    }

    /// Caret to the very end of the document.
    pub fn move_document_end(
        &mut self,
        _: &input_actions::MoveDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_caret(self.text.len(), false, cx);
    }

    /// Selects from the caret back to the beginning of the document.
    pub fn select_document_start(
        &mut self,
        _: &input_actions::SelectDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_caret(0, true, cx);
    }

    /// Selects from the caret forward to the end of the document.
    pub fn select_document_end(
        &mut self,
        _: &input_actions::SelectDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_caret(self.text.len(), true, cx);
    }

    /// Caret one display line up, keeping its horizontal goal column across a
    /// run of presses.
    ///
    /// In a single-line field this propagates instead, leaving the arrow to the
    /// list or picker the field sits in front of.
    pub fn move_up(&mut self, _: &input_actions::MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, false, cx);
    }

    /// Caret one display line down. Propagates in a single-line field, as
    /// [`InputUiState::move_up`] does.
    pub fn move_down(
        &mut self,
        _: &input_actions::MoveDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(1, false, cx);
    }

    /// Grows the selection one display line up.
    pub fn select_up(
        &mut self,
        _: &input_actions::SelectUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(-1, true, cx);
    }

    /// Grows the selection one display line down.
    pub fn select_down(
        &mut self,
        _: &input_actions::SelectDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(1, true, cx);
    }

    /// Selects the whole document.
    pub fn select_all(
        &mut self,
        _: &input_actions::SelectAll,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_all_text(cx);
    }

    /// Deletes the selection, or one grapheme before the caret — one press
    /// takes a whole emoji or a combining sequence.
    pub fn backspace(
        &mut self,
        _: &input_actions::Backspace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_backwards(text_boundaries::previous_grapheme_boundary, cx);
    }

    /// Deletes the selection, or back to the previous word boundary.
    pub fn backspace_word(
        &mut self,
        _: &input_actions::BackspaceWord,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_backwards(text_boundaries::previous_word_boundary, cx);
    }

    /// Deletes the selection, or one grapheme after the caret.
    pub fn delete(&mut self, _: &input_actions::Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_forwards(text_boundaries::next_grapheme_boundary, cx);
    }

    /// Deletes the selection, or forward to the next word boundary.
    pub fn delete_word(
        &mut self,
        _: &input_actions::DeleteWord,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_forwards(text_boundaries::next_word_boundary, cx);
    }

    /// Copies the selection. A secure field copies nothing.
    pub fn copy(&mut self, _: &input_actions::Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    /// Copies the selection and deletes it. In a secure field neither half
    /// happens — a refused copy must not still delete.
    pub fn cut(&mut self, _: &input_actions::Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.copy_selection(cx) {
            return;
        }
        self.replace(self.selection.range(), "", EditKind::Atomic, cx);
    }

    /// Pastes text over the selection, as one undo step.
    ///
    /// Line breaks are normalised — collapsed to spaces in a field, CRLF
    /// flattened in a composer — and the char limit truncates rather than
    /// rejects.
    pub fn paste(&mut self, _: &input_actions::Paste, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.replace(self.selection.range(), &text, EditKind::Atomic, cx);
    }

    /// Undoes one step and restores the selection that preceded it.
    ///
    /// A run of typing is one step, and so is a whole IME composition; moving
    /// the caret ends the run.
    pub fn undo(&mut self, _: &input_actions::Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let Some(edit) = self.history.undo() else {
            return;
        };
        self.text.replace_range(edit.undone_range(), &edit.removed);
        self.selection = Selection::new(edit.selection_before.start, edit.selection_before.end);
        self.after_history(cx);
    }

    /// Reapplies the last undone step.
    pub fn redo(&mut self, _: &input_actions::Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let Some(edit) = self.history.redo() else {
            return;
        };
        self.text.replace_range(edit.redone_range(), &edit.inserted);
        self.selection = Selection::new(edit.selection_after.start, edit.selection_after.end);
        self.after_history(cx);
    }

    /// Enter: emits [`InputUiEvent::Submitted`] in a field, inserts a line
    /// break in a composer.
    pub fn confirm(&mut self, _: &input_actions::Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if self.mode.is_multiline() {
            self.replace(self.selection.range(), "\n", EditKind::Typing, cx);
        } else {
            cx.emit(InputUiEvent::Submitted);
        }
    }

    /// Escape. Takes back a composition in progress if there is one; only
    /// otherwise does it emit [`InputUiEvent::Cancelled`], so the first Escape
    /// during CJK input does not also close the sheet the field is in.
    pub fn cancel(&mut self, _: &input_actions::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() {
            self.cancel_composition(cx);
            return;
        }
        cx.emit(InputUiEvent::Cancelled);
    }

    /// Opens the system emoji and symbol palette. Refused for a secure field,
    /// which the palette would otherwise read back.
    pub fn show_character_palette(
        &mut self,
        _: &input_actions::ShowCharacterPalette,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if !self.secure {
            window.show_character_palette();
        }
    }

    // ---- internals --------------------------------------------------------

    fn horizontal(&mut self, forward: bool, extend: bool, cx: &mut Context<Self>) {
        let target = if !extend && !self.selection.is_empty() {
            if forward {
                self.selection.end()
            } else {
                self.selection.start()
            }
        } else if forward {
            text_boundaries::next_grapheme_boundary(&self.text, self.selection.head)
        } else {
            text_boundaries::previous_grapheme_boundary(&self.text, self.selection.head)
        };
        self.move_caret(target, extend, cx);
    }

    fn vertical(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        // A single-line field has no line to move to, and an action listener
        // stops propagation by default — so consuming the keystroke here would
        // silently take the arrow keys away from every list this field sits in
        // front of. The command palette and every picker after it depend on
        // getting them.
        if !self.mode.is_multiline() {
            cx.propagate();
            return;
        }

        let line_index = self.display.line_index_for_source(self.selection.head);
        let target_line = line_index.saturating_add_signed(delta);
        if target_line >= self.display.line_count() {
            let target = if delta < 0 { 0 } else { self.text.len() };
            self.move_caret(target, extend, cx);
            return;
        }

        let goal_x = self.goal_x.or_else(|| {
            let offset_in_line = self
                .display
                .offset_within_line(line_index, self.selection.head);
            self.shaped
                .as_ref()
                .map(|shaped| shaped.x_for(line_index, offset_in_line))
        });

        let target = match (goal_x, self.shaped.as_ref()) {
            (Some(x), Some(shaped)) if shaped.line(target_line).is_some() => {
                let offset_in_line = shaped.offset_for_x(target_line, x);
                self.display
                    .source_offset_within_line(target_line, offset_in_line)
            }
            _ => self.column_preserving_offset(line_index, target_line),
        };

        self.move_caret(target, extend, cx);
        self.goal_x = goal_x;
    }

    /// The fallback when the target row has not been shaped: the same number of
    /// graphemes in from the start of the line. Correct for a monospaced run,
    /// approximate for a proportional one.
    fn column_preserving_offset(&self, from_line: usize, to_line: usize) -> usize {
        let Some(from) = self.display.lines.get(from_line) else {
            return self.selection.head;
        };
        let Some(to) = self.display.lines.get(to_line) else {
            return self.selection.head;
        };
        let column = text_boundaries::grapheme_index(
            &self.text[from.source.clone()],
            self.selection.head.saturating_sub(from.source.start),
        );
        to.source.start + text_boundaries::grapheme_offset(&self.text[to.source.clone()], column)
    }

    fn move_caret(&mut self, offset: usize, extend: bool, cx: &mut Context<Self>) {
        let offset = text_boundaries::clamp_to_char_boundary(&self.text, offset);
        if extend {
            self.selection.head = offset;
        } else {
            self.selection = Selection::cursor(offset);
        }
        self.goal_x = None;
        self.history.break_coalescing();
        let epoch = self.blink.interrupt();
        self.schedule_resume(epoch, cx);
        cx.notify();
    }

    fn delete_backwards(&mut self, boundary: fn(&str, usize) -> usize, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            self.replace(self.selection.range(), "", EditKind::Deleting, cx);
            return;
        }
        let head = self.selection.head;
        let start = boundary(&self.text, head);
        if start == head {
            return;
        }
        self.replace(start..head, "", EditKind::Deleting, cx);
    }

    fn delete_forwards(&mut self, boundary: fn(&str, usize) -> usize, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            self.replace(self.selection.range(), "", EditKind::Deleting, cx);
            return;
        }
        let head = self.selection.head;
        let end = boundary(&self.text, head);
        if end == head {
            return;
        }
        self.replace(head..end, "", EditKind::Deleting, cx);
    }

    /// `false` when nothing was put on the clipboard, which is also the answer
    /// for a secure field: a password must not leave through Cmd-C, and cut
    /// therefore must not delete either.
    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        if self.secure || self.selection.is_empty() {
            return false;
        }
        let selected = self.text[self.selection.range()].to_owned();
        cx.write_to_clipboard(ClipboardItem::new_string(selected));
        true
    }

    fn replace(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        kind: EditKind,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let range = self.clamp_range(range);
        let insertion = self.fit(new_text, &range);
        if insertion.is_empty() && range.is_empty() {
            return;
        }

        let removed = self.text[range.clone()].to_owned();
        let selection_before = self.selection.range();
        let caret = range.start + insertion.len();

        self.text.replace_range(range.clone(), &insertion);
        self.selection = Selection::cursor(caret);
        self.marked = None;
        self.composition = None;
        self.history.push(
            Edit {
                at: range.start,
                removed,
                inserted: insertion,
                selection_before,
                selection_after: caret..caret,
            },
            kind,
        );
        self.after_edit(cx);
    }

    fn clamp_range(&self, range: Range<usize>) -> Range<usize> {
        let start = text_boundaries::clamp_to_char_boundary(&self.text, range.start);
        let end = text_boundaries::clamp_to_char_boundary(&self.text, range.end.max(range.start));
        start..end.max(start)
    }

    /// The text as this field will actually store it: newlines collapsed in a
    /// single-line field, and truncated to whatever the char limit still allows.
    fn fit(&self, new_text: &str, replacing: &Range<usize>) -> String {
        let sanitised = if self.mode.is_multiline() {
            rewrite_line_breaks(new_text, '\n')
        } else {
            rewrite_line_breaks(new_text, ' ')
        };

        let Some(limit) = self.max_length else {
            return sanitised;
        };
        let removed = text_boundaries::char_count(&self.text[replacing.clone()]);
        let kept = text_boundaries::char_count(&self.text) - removed;
        let room = limit.saturating_sub(kept);
        let cut = text_boundaries::truncate_to_char_limit(&sanitised, room);
        sanitised[..cut].to_owned()
    }

    fn after_edit(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        self.rebuild_display();
        self.goal_x = None;
        let epoch = self.blink.interrupt();
        self.schedule_resume(epoch, cx);
        cx.emit(InputUiEvent::Changed);
        cx.notify();
    }

    fn after_history(&mut self, cx: &mut Context<Self>) {
        self.marked = None;
        self.composition = None;
        self.after_edit(cx);
    }

    fn invalidate_display(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        self.rebuild_display();
        cx.notify();
    }

    fn rebuild_display(&mut self) {
        self.display = DisplayText::new(
            &self.text,
            &self.placeholder,
            self.secure,
            self.mode.is_multiline(),
        );
    }

    fn schedule_resume(&self, epoch: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAUSE_AFTER_EDIT).await;
            this.update(cx, |this, cx| {
                if this.blink.resume(epoch) {
                    this.schedule_tick(epoch, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn schedule_tick(&self, epoch: u64, cx: &mut Context<Self>) {
        schedule_tick_after(BLINK_INTERVAL, epoch, cx);
    }

    fn resolve_ime_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        let range = range_utf16
            .map(|range| text_boundaries::utf8_range_from_utf16(&self.text, &range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.range());
        self.clamp_range(range)
    }

    fn cancel_composition(&mut self, cx: &mut Context<Self>) {
        let Some(marked) = self.marked.clone() else {
            return;
        };
        let composition = self.composition.take();
        self.text.replace_range(marked.clone(), "");
        self.marked = None;
        self.selection = Selection::cursor(marked.start);

        if let Some(composition) = composition
            && !composition.removed.is_empty()
        {
            let caret = self.selection.range();
            self.history.push(
                Edit {
                    at: composition.at,
                    removed: composition.removed,
                    inserted: String::new(),
                    selection_before: composition.selection_before,
                    selection_after: caret,
                },
                EditKind::Atomic,
            );
        }
        self.after_edit(cx);
    }
}

fn schedule_tick_after(delay: Duration, epoch: u64, cx: &mut Context<InputUiState>) {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(delay).await;
        this.update(cx, |this, cx| {
            if this.blink.tick(epoch) {
                cx.notify();
                schedule_tick_after(BLINK_INTERVAL, epoch, cx);
            }
        })
        .ok();
    })
    .detach();
}

/// Every line break becomes `replacement`, and a CRLF becomes one of it.
///
/// A carriage return never survives either branch: a lone `\r` inside the
/// document would split no line and render as a control glyph.
fn rewrite_line_breaks(text: &str, replacement: char) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                out.push(replacement);
            }
            '\n' => out.push(replacement),
            _ => out.push(character),
        }
    }
    out
}

impl EntityInputHandler for InputUiState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        if self.secure {
            return None;
        }
        let range = text_boundaries::utf8_range_from_utf16(&self.text, &range_utf16);
        adjusted_range.replace(text_boundaries::utf16_range_from_utf8(&self.text, &range));
        Some(self.text[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if self.disabled && !ignore_disabled_input {
            return None;
        }
        Some(UTF16Selection {
            range: text_boundaries::utf16_range_from_utf8(&self.text, &self.selection.range()),
            reversed: self.selection.is_reversed(),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|marked| text_boundaries::utf16_range_from_utf8(&self.text, marked))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(composition) = self.composition.take() else {
            if self.marked.take().is_some() {
                cx.notify();
            }
            return;
        };
        let marked = self.marked.take();
        let end = marked.map_or(composition.at, |marked| marked.end);
        let at = composition.at.min(end);
        let inserted = self.text[at..end.max(at)].to_owned();

        if inserted != composition.removed {
            let caret = self.selection.range();
            self.history.push(
                Edit {
                    at,
                    removed: composition.removed,
                    inserted,
                    selection_before: composition.selection_before,
                    selection_after: caret,
                },
                EditKind::Atomic,
            );
        }
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let range = self.resolve_ime_range(range_utf16);

        let Some(composition) = self.composition.take() else {
            self.replace(range, new_text, EditKind::Typing, cx);
            return;
        };

        // The composition commits as one step: `at` is where it began, `removed`
        // is what it displaced, and everything between is the result.
        let insertion = self.fit(new_text, &range);
        let at = composition.at.min(range.start);
        self.text.replace_range(range.clone(), &insertion);
        let end = range.start + insertion.len();
        let inserted = self.text[at..end.max(at)].to_owned();

        self.selection = Selection::cursor(end);
        self.marked = None;
        self.history.push(
            Edit {
                at,
                removed: composition.removed,
                inserted,
                selection_before: composition.selection_before,
                selection_after: end..end,
            },
            EditKind::Atomic,
        );
        self.after_edit(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let range = self.resolve_ime_range(range_utf16);

        if self.composition.is_none() {
            let removed = self.text[range.clone()].to_owned();
            self.composition = Some(Composition {
                at: range.start,
                removed,
                selection_before: self.selection.range(),
            });
        }

        let insertion = self.fit(new_text, &range);
        self.text.replace_range(range.clone(), &insertion);
        let marked = range.start..range.start + insertion.len();

        // `new_selected_range_utf16` is relative to the marked text, not to the
        // document. Treating it as a document range is the classic IME bug: the
        // caret lands somewhere near the start of the line during composition.
        self.selection = match new_selected_range_utf16 {
            Some(selected) => {
                let start = marked.start
                    + text_boundaries::utf8_offset_from_utf16(&insertion, selected.start);
                let end = marked.start
                    + text_boundaries::utf8_offset_from_utf16(
                        &insertion,
                        selected.end.max(selected.start),
                    );
                Selection::new(start, end)
            }
            None => Selection::cursor(marked.end),
        };

        if insertion.is_empty() {
            let composition = self.composition.take();
            self.marked = None;
            if let Some(composition) = composition
                && !composition.removed.is_empty()
            {
                let caret = self.selection.range();
                self.history.push(
                    Edit {
                        at: composition.at,
                        removed: composition.removed,
                        inserted: String::new(),
                        selection_before: composition.selection_before,
                        selection_after: caret,
                    },
                    EditKind::Atomic,
                );
            }
        } else {
            self.marked = Some(marked);
        }

        self.after_edit(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let shaped = self.shaped.as_ref()?;
        let range = text_boundaries::utf8_range_from_utf16(&self.text, &range_utf16);
        let line_index = self.display.line_index_for_source(range.start);

        let top = element_bounds.top() + self.line_height * (line_index as f32) - self.scroll.y;
        let left = element_bounds.left() - self.scroll.x
            + shaped.x_for(
                line_index,
                self.display.offset_within_line(line_index, range.start),
            );
        let right = element_bounds.left() - self.scroll.x
            + shaped.x_for(
                line_index,
                self.display.offset_within_line(line_index, range.end),
            );

        Some(Bounds::from_corners(
            point(left, top),
            point(right, top + self.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let offset = self.offset_at(position);
        Some(text_boundaries::utf16_offset_from_utf8(&self.text, offset))
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(text_boundaries::utf16_len(&self.text))
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        !self.disabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_ui::input_actions::{
        Backspace, BackspaceWord, Cancel, Confirm, Copy, Cut, Delete, DeleteWord, Paste, SelectAll,
        SelectLeft, SelectRight, Undo,
    };
    use gpui::{TestAppContext, WindowHandle};
    use rise_theme::AppTheme;

    const CYRILLIC: &str = "Привет";
    const EMOJI: &str = "👍🏻";

    fn open(mode: InputMode, cx: &mut TestAppContext) -> WindowHandle<InputUiState> {
        cx.update(|cx| {
            if !cx.has_global::<AppTheme>() {
                crate::install_theme(AppTheme::dark(), cx);
            }
        });
        cx.add_window(|_window, cx| InputUiState::new(mode, cx))
    }

    fn edit<R>(
        handle: &WindowHandle<InputUiState>,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut InputUiState, &mut Window, &mut Context<InputUiState>) -> R,
    ) -> R {
        handle.update(cx, f).expect("window is open")
    }

    fn text_of(handle: &WindowHandle<InputUiState>, cx: &mut TestAppContext) -> String {
        edit(handle, cx, |state, _, _| state.text().to_owned())
    }

    fn type_text(handle: &WindowHandle<InputUiState>, cx: &mut TestAppContext, text: &str) {
        for character in text.chars() {
            let piece = character.to_string();
            edit(handle, cx, |state, window, cx| {
                state.replace_text_in_range(None, &piece, window, cx);
            });
        }
    }

    fn assert_consistent(handle: &WindowHandle<InputUiState>, cx: &mut TestAppContext) {
        edit(handle, cx, |state, _, _| {
            let text = state.text().to_owned();
            let selection = state.selection();
            assert!(
                text.is_char_boundary(selection.anchor),
                "anchor {} is inside a char of {text:?}",
                selection.anchor
            );
            assert!(
                text.is_char_boundary(selection.head),
                "head {} is inside a char of {text:?}",
                selection.head
            );
            if let Some(marked) = state.marked_range() {
                assert!(text.is_char_boundary(marked.start));
                assert!(text.is_char_boundary(marked.end));
            }
        });
    }

    #[gpui::test]
    fn typing_and_deleting_multibyte_text_never_lands_off_a_char_boundary(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);

        for source in [CYRILLIC, "日本語", EMOJI, "e\u{0301}", "🇷🇺"] {
            edit(&field, cx, |state, window, cx| {
                state.replace_text_in_range(None, source, window, cx);
            });
            assert_consistent(&field, cx);
        }

        while !text_of(&field, cx).is_empty() {
            edit(&field, cx, |state, window, cx| {
                state.backspace(&Backspace, window, cx);
            });
            assert_consistent(&field, cx);
        }

        edit(&field, cx, |state, window, cx| {
            state.backspace(&Backspace, window, cx);
            state.delete(&Delete, window, cx);
        });
        assert_eq!(text_of(&field, cx), "");
    }

    #[gpui::test]
    fn a_single_line_field_leaves_the_arrow_keys_to_the_list_in_front_of_it(
        cx: &mut TestAppContext,
    ) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, "riseonly", window, cx);
            state.set_selection(Selection::cursor(4), cx);

            state.move_up(&input_actions::MoveUp, window, cx);
            assert_eq!(state.selection().head, 4);
            state.move_down(&input_actions::MoveDown, window, cx);
            assert_eq!(state.selection().head, 4);
            state.select_up(&input_actions::SelectUp, window, cx);
            assert!(state.selection().is_empty());
        });
    }

    #[gpui::test]
    fn a_multi_line_field_still_moves_by_line(cx: &mut TestAppContext) {
        let composer = open(InputMode::MultiLine, cx);
        edit(&composer, cx, |state, window, cx| {
            state.replace_text_in_range(None, "one\ntwo", window, cx);
            state.move_up(&input_actions::MoveUp, window, cx);
            assert!(state.selection().head < 4, "the caret stayed on line two");
        });
    }

    #[gpui::test]
    fn a_left_arrow_crosses_a_whole_grapheme(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, EMOJI, window, cx);
            assert_eq!(state.selection().head, EMOJI.len());
            state.move_left(&input_actions::MoveLeft, window, cx);
            assert_eq!(state.selection().head, 0, "one press, not four");
            state.move_right(&input_actions::MoveRight, window, cx);
            assert_eq!(state.selection().head, EMOJI.len());
        });
    }

    #[gpui::test]
    fn a_selection_survives_shift_left_then_shift_right(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, CYRILLIC, window, cx);

            state.select_left(&SelectLeft, window, cx);
            let after_left = state.selection();
            assert!(after_left.is_reversed());
            assert_eq!(after_left.range(), 10..12);

            state.select_right(&SelectRight, window, cx);
            let after_right = state.selection();
            assert!(
                after_right.is_empty(),
                "the selection must collapse, not flip"
            );
            assert_eq!(after_right.head, 12);
        });
    }

    #[gpui::test]
    fn word_wise_delete_works_at_both_ends(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, "Привет мир", window, cx);
            state.backspace_word(&BackspaceWord, window, cx);
            assert_eq!(state.text(), "Привет ");
            state.backspace_word(&BackspaceWord, window, cx);
            assert_eq!(state.text(), "");
            state.backspace_word(&BackspaceWord, window, cx);
            assert_eq!(state.text(), "");

            state.replace_text_in_range(None, "Привет мир", window, cx);
            state.move_document_start(&input_actions::MoveDocumentStart, window, cx);
            state.delete_word(&DeleteWord, window, cx);
            assert_eq!(state.text(), " мир");
            state.delete_word(&DeleteWord, window, cx);
            assert_eq!(state.text(), "");
            state.delete_word(&DeleteWord, window, cx);
            assert_eq!(state.text(), "");
        });
    }

    #[gpui::test]
    fn replace_text_in_range_speaks_utf16(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, "日本語", window, cx);

            // Three UTF-16 units, nine UTF-8 bytes.
            let selection = state
                .selected_text_range(true, window, cx)
                .expect("a selection");
            assert_eq!(selection.range, 3..3);

            state.replace_text_in_range(Some(1..2), "X", window, cx);
            assert_eq!(state.text(), "日X語");

            let mut adjusted = None;
            let read = state.text_for_range(0..2, &mut adjusted, window, cx);
            assert_eq!(read.as_deref(), Some("日X"));
            assert_eq!(adjusted, Some(0..2));
        });
    }

    #[gpui::test]
    fn a_composition_commits_as_one_atomic_step(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_and_mark_text_in_range(None, "に", Some(0..1), window, cx);
            assert_eq!(state.text(), "に");
            assert_eq!(state.marked_range(), Some(0..3));
            assert_eq!(state.selection().range(), 0..3);

            state.replace_and_mark_text_in_range(None, "にほん", Some(0..3), window, cx);
            assert_eq!(state.text(), "にほん");
            assert_eq!(state.marked_range(), Some(0..9));

            state.replace_text_in_range(None, "日本", window, cx);
            assert_eq!(state.text(), "日本");
            assert_eq!(state.marked_range(), None);
            assert_eq!(state.selection().head, "日本".len());

            state.undo(&Undo, window, cx);
            assert_eq!(
                state.text(),
                "",
                "a whole composition is one undo step, not one per keystroke"
            );
            assert!(!state.can_undo());
        });
    }

    #[gpui::test]
    fn a_cancelled_composition_leaves_nothing_behind(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, "ab", window, cx);
            let before = state.text().to_owned();
            let undo_depth = state.can_undo();

            state.replace_and_mark_text_in_range(None, "に", Some(0..1), window, cx);
            assert_eq!(state.text(), "abに");

            state.replace_and_mark_text_in_range(None, "", None, window, cx);
            assert_eq!(state.text(), before);
            assert_eq!(state.marked_range(), None);
            assert_eq!(state.selection().head, before.len());
            assert_eq!(state.can_undo(), undo_depth);

            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "", "the cancelled mark left no extra step");
        });
    }

    #[gpui::test]
    fn unmarking_keeps_the_composed_text_as_one_step(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_and_mark_text_in_range(None, "ほん", Some(0..2), window, cx);
            state.unmark_text(window, cx);
            assert_eq!(state.text(), "ほん");
            assert_eq!(state.marked_range(), None);

            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "");
        });
    }

    #[gpui::test]
    fn a_marked_range_replaces_the_selection_it_displaced(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, "Привет", window, cx);
            state.select_all(&SelectAll, window, cx);

            state.replace_and_mark_text_in_range(None, "に", Some(0..1), window, cx);
            assert_eq!(state.text(), "に");

            state.replace_text_in_range(None, "日", window, cx);
            assert_eq!(state.text(), "日");

            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "Привет");
        });
    }

    #[gpui::test]
    fn a_multiline_paste_into_a_single_line_field_collapses(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo\r\nthree".into()));
        edit(&field, cx, |state, window, cx| {
            state.paste(&Paste, window, cx);
            assert_eq!(state.text(), "one two three");
            assert!(!state.text().contains('\n'));
        });

        let composer = open(InputMode::MultiLine, cx);
        edit(&composer, cx, |state, window, cx| {
            state.paste(&Paste, window, cx);
            assert_eq!(
                state.text(),
                "one\ntwo\nthree",
                "a Windows clipboard must not leave a carriage return in the document"
            );
            assert_eq!(state.display().line_count(), 3);
        });
    }

    #[gpui::test]
    fn a_run_of_typed_characters_undoes_in_one_step(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        type_text(&field, cx, "Привет");
        assert_eq!(text_of(&field, cx), "Привет");

        edit(&field, cx, |state, window, cx| {
            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "");
            assert!(!state.can_undo());

            state.redo(&input_actions::Redo, window, cx);
            assert_eq!(state.text(), "Привет");
        });
    }

    #[gpui::test]
    fn moving_the_caret_ends_the_undo_run(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        type_text(&field, cx, "ab");
        edit(&field, cx, |state, window, cx| {
            state.move_left(&input_actions::MoveLeft, window, cx);
            state.move_right(&input_actions::MoveRight, window, cx);
        });
        type_text(&field, cx, "cd");

        edit(&field, cx, |state, window, cx| {
            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "ab");
            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "");
        });
    }

    #[gpui::test]
    fn the_char_limit_holds_on_typing_and_on_paste(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, _window, cx| {
            state.set_max_length(Some(5), cx);
        });

        type_text(&field, cx, "Приветмир");
        assert_eq!(text_of(&field, cx), "Приве");

        edit(&field, cx, |state, window, cx| {
            state.select_all(&SelectAll, window, cx);
            state.backspace(&Backspace, window, cx);
        });
        cx.write_to_clipboard(ClipboardItem::new_string("日本語のテキスト".into()));
        edit(&field, cx, |state, window, cx| {
            state.paste(&Paste, window, cx);
            assert_eq!(state.text(), "日本語のテ");
            assert_eq!(state.text().chars().count(), 5);
        });
    }

    #[gpui::test]
    fn the_char_limit_still_allows_replacing_a_full_selection(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.set_max_length(Some(3), cx);
            state.replace_text_in_range(None, "abc", window, cx);
            state.select_all(&SelectAll, window, cx);
            state.replace_text_in_range(None, "xyz", window, cx);
            assert_eq!(state.text(), "xyz");
        });
    }

    #[gpui::test]
    fn a_secure_field_refuses_to_hand_over_its_contents(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        cx.write_to_clipboard(ClipboardItem::new_string("untouched".into()));

        edit(&field, cx, |state, window, cx| {
            state.set_secure(true, cx);
            state.replace_text_in_range(None, "hunter2", window, cx);
            state.select_all(&SelectAll, window, cx);
            state.copy(&Copy, window, cx);
        });

        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("untouched".to_owned())
        );

        edit(&field, cx, |state, window, cx| {
            state.cut(&Cut, window, cx);
            assert_eq!(
                state.text(),
                "hunter2",
                "a refused copy must not delete either"
            );

            let mut adjusted = None;
            assert_eq!(state.text_for_range(0..3, &mut adjusted, window, cx), None);
        });

        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("untouched".to_owned())
        );
    }

    #[gpui::test]
    fn a_plain_field_still_copies_and_cuts(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, "Привет", window, cx);
            state.select_all(&SelectAll, window, cx);
            state.cut(&Cut, window, cx);
            assert_eq!(state.text(), "");
        });
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("Привет".to_owned())
        );
    }

    #[gpui::test]
    fn a_secure_field_shows_one_bullet_per_grapheme(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.set_secure(true, cx);
            state.replace_text_in_range(None, "a👍🏻б", window, cx);
            assert_eq!(state.display().text.as_ref(), "•••");
        });
    }

    #[gpui::test]
    fn enter_submits_a_field_and_breaks_a_line_in_a_composer(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, "hi", window, cx);
            state.confirm(&Confirm, window, cx);
            assert_eq!(state.text(), "hi", "a single-line field never grows a line");
        });

        let composer = open(InputMode::MultiLine, cx);
        edit(&composer, cx, |state, window, cx| {
            state.replace_text_in_range(None, "hi", window, cx);
            state.confirm(&Confirm, window, cx);
            assert_eq!(state.text(), "hi\n");
            assert_eq!(state.display().line_count(), 2);
        });
    }

    #[gpui::test]
    fn escape_cancels_a_composition_before_it_reaches_the_caller(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_and_mark_text_in_range(None, "に", Some(0..1), window, cx);
            state.cancel(&Cancel, window, cx);
            assert_eq!(state.text(), "");
            assert_eq!(state.marked_range(), None);
        });
    }

    #[gpui::test]
    fn a_disabled_field_accepts_nothing(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, "before", window, cx);
            state.set_disabled(true, cx);

            state.replace_text_in_range(None, "after", window, cx);
            state.replace_and_mark_text_in_range(None, "に", None, window, cx);
            state.backspace(&Backspace, window, cx);
            assert_eq!(state.text(), "before");
            assert!(!state.accepts_text_input(window, cx));
            assert!(state.selected_text_range(false, window, cx).is_none());
            assert!(state.selected_text_range(true, window, cx).is_some());
        });
    }

    #[gpui::test]
    fn set_text_is_one_undoable_step(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.set_text("Привет", cx);
            assert_eq!(state.text(), "Привет");
            state.set_text("мир", cx);
            assert_eq!(state.text(), "мир");
            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "Привет");
        });
    }

    #[gpui::test]
    fn reset_forgets_the_history(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, _window, cx| {
            state.set_text("Привет", cx);
            state.reset("мир", cx);
            assert_eq!(state.text(), "мир");
            assert!(!state.can_undo());

            state.reset("one\r\ntwo", cx);
            assert_eq!(
                state.text(),
                "one two",
                "reset goes through the same sanitiser as typing"
            );
        });
    }

    #[gpui::test]
    fn drawing_the_element_shapes_the_visible_rows_and_answers_a_click(cx: &mut TestAppContext) {
        let composer = open(InputMode::MultiLine, cx);
        edit(&composer, cx, |state, window, cx| {
            let handle = state.focus_handle.clone();
            window.focus(&handle, cx);
            state.replace_text_in_range(None, "Привет\nмир\n日本語", window, cx);
        });
        cx.run_until_parked();

        edit(&composer, cx, |state, _window, _cx| {
            let shaped = state.shaped().expect("prepaint shaped the visible rows");
            assert!(shaped.visible().start == 0);
            assert!(state.line_height() > px(0.));
            assert_eq!(state.display().line_count(), 3);
        });

        let (bounds, line_height) = edit(&composer, cx, |state, _window, _cx| {
            (state.bounds.expect("laid out"), state.line_height())
        });

        edit(&composer, cx, |state, _window, cx| {
            let middle_of_second_row = point(bounds.left(), bounds.top() + line_height * 1.5);
            state.on_mouse_down(middle_of_second_row, 1, false, cx);
            assert_eq!(state.selection().head, "Привет\n".len());

            state.on_mouse_down(middle_of_second_row, 2, false, cx);
            assert_eq!(
                state.selection().range(),
                "Привет\n".len().."Привет\nмир".len(),
                "a double click takes the word under it"
            );

            state.on_mouse_down(middle_of_second_row, 3, false, cx);
            assert_eq!(
                state.selection().range(),
                "Привет\n".len().."Привет\nмир".len()
            );
            state.on_mouse_up(cx);
            assert!(!state.is_dragging());
        });
    }

    #[gpui::test]
    fn the_caret_blinks_when_idle_and_stops_while_typing(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            let handle = state.focus_handle.clone();
            window.focus(&handle, cx);
        });
        cx.run_until_parked();

        assert!(edit(&field, cx, |state, _, _| state.caret_is_visible()));

        cx.executor()
            .advance_clock(PAUSE_AFTER_EDIT + BLINK_INTERVAL);
        cx.run_until_parked();
        assert!(
            !edit(&field, cx, |state, _, _| state.caret_is_visible()),
            "an idle caret blinks"
        );

        for character in "Привет".chars() {
            let piece = character.to_string();
            edit(&field, cx, |state, window, cx| {
                state.replace_text_in_range(None, &piece, window, cx);
            });
            cx.executor().advance_clock(BLINK_INTERVAL);
            cx.run_until_parked();
            assert!(
                edit(&field, cx, |state, _, _| state.caret_is_visible()),
                "typing must hold the caret solid"
            );
        }
    }

    #[test]
    fn a_selection_keeps_its_direction() {
        let mut selection = Selection::cursor(5);
        selection.head = 3;
        assert!(selection.is_reversed());
        assert_eq!(selection.range(), 3..5);

        selection.head = 7;
        assert!(!selection.is_reversed());
        assert_eq!(selection.range(), 5..7);
    }

    #[test]
    fn a_single_line_field_collapses_pasted_newlines() {
        assert_eq!(rewrite_line_breaks("one\ntwo", ' '), "one two");
        assert_eq!(rewrite_line_breaks("one\r\ntwo", ' '), "one two");
        assert_eq!(rewrite_line_breaks("one\rtwo", ' '), "one two");
        assert_eq!(rewrite_line_breaks("Привет\nмир", ' '), "Привет мир");
    }

    #[test]
    fn a_composer_keeps_the_breaks_but_never_a_carriage_return() {
        assert_eq!(rewrite_line_breaks("one\r\ntwo", '\n'), "one\ntwo");
        assert_eq!(rewrite_line_breaks("one\rtwo", '\n'), "one\ntwo");
        assert_eq!(rewrite_line_breaks("one\ntwo", '\n'), "one\ntwo");
    }
}
