use std::any::Any;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Bounds, ClipboardItem, Context, EntityId, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, FontWeight, Global, Hsla, KeystrokeEvent, Pixels, Point, SharedString, Subscription, TextAlign,
    UTF16Selection, Window, point, px, transparent_black,
};

use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::ns_range::{self, NSRange};
use rise_core::rise_app_events::{RiseAppEvents, UIResponder};

use crate::keyboard_dismiss::ScreenFocus;
use crate::input_ui::attributed_runs::{AttributedRuns, RunAttributes};
use crate::input_ui::caret_blink::{BLINK_INTERVAL, CaretBlink, CaretSwap, PAUSE_AFTER_EDIT, caret_path};
use crate::input_ui::display_text::DisplayText;
use crate::input_ui::edit_history::{Edit, EditHistory, EditKind};
use crate::input_ui::input_actions::{self, install_key_bindings};
use crate::input_ui::shaped_input::ShapedInput;
use crate::input_ui::text_boundaries;
use crate::input_ui::text_view_delegate::InputUiTextViewDelegate;
use crate::context_menu_ui::ContextMenuItem;
use crate::swiftui::rich_text::AttributeContainer;

const HEIGHT_CHANGE_THRESHOLD: f32 = 0.5;

/// whether the field is one line or many
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InputMode {
    #[default]
    SingleLine,
    MultiLine,
}

impl InputMode {
    pub fn is_multiline(self) -> bool {
        matches!(self, Self::MultiLine)
    }
}

/// an anchor and a head, never a sorted pair
/// start and end are derived
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    /// empty selection (a caret) at a byte offset
    pub fn cursor(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
        }
    }

    /// offsets are bytes into the document and need not be ordered
    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head }
    }

    /// the lower of the two offsets
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    /// the higher of the two offsets
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    /// selected bytes in order, safe to slice the document with
    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }

    /// true for a bare caret and not a range
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// true when the moving end is before the fixed one
    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }
}

/// what the field reports to listeners, subscribe with cx.subscribe
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputUiEvent {
    Changed,
    Submitted,
    Cancelled,
}

/// what one selection gesture extends by, fixed at mouse down and held for the whole drag
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectGranularity {
    Character,
    Word,
    Line,
}

#[derive(Clone, Debug)]
struct Drag {
    granularity: SelectGranularity,
    anchor: Range<usize>,
}

#[derive(Clone, Debug)]
struct Composition {
    at: usize,
    removed: String,
    selection_before: Range<usize>,
}

/// everything a text field is apart from how it is drawn
/// keep it in an Entity, a field rebuilt every render loses selection and history
/// offsets are utf8 bytes on a char boundary
pub struct InputUiState {
    mode: InputMode,
    text: String,
    selection: Selection,
    marked: Option<Range<usize>>,
    composition: Option<Composition>,
    placeholder: SharedString,
    secure: bool,
    disabled: bool,
    chromeless: bool,
    height: Option<Pixels>,
    max_length: Option<usize>,
    max_visible_lines: usize,
    wraps: bool,
    history: EditHistory,
    blink: CaretBlink,
    caret_swap: Option<CaretSwap>,
    caret_swap_missed: bool,
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
    runs: Option<AttributedRuns>,
    base_typing_attributes: RunAttributes,
    typing_attributes: Option<RunAttributes>,
    delegate: InputUiTextViewDelegate,
    published_selection: Option<NSRange>,
    published_height: Option<Pixels>,
    text_color: Option<Hsla>,
    placeholder_color: Option<Hsla>,
    caret_color: Option<Hsla>,
    font_size: Option<Pixels>,
    font_weight: Option<FontWeight>,
    minimum_line_height: Option<Pixels>,
    baseline_lift: Pixels,
    text_align: TextAlign,
    caret_reserve: Pixels,
    scroll_enabled: Option<bool>,
    wheel_anchor: Option<(u64, Selection)>,
    wheel_latch_until: Option<Duration>,
    edit_submenu: Option<InputEditSubmenu>,
    edit_menu: Option<InputEditMenuPresentation>,
    on_edit_menu_presented_change: Option<InputEditMenuPresentedChange>,
    edit_menu_escape: Option<Subscription>,
    on_paste: Option<InputPasteHandler>,
    escape_to_layer: bool,
    escape_clears_text: bool,
}

/// submenu the field edit menu carries next to cut copy and paste
/// shown only while something is selected
#[derive(Clone)]
pub struct InputEditSubmenu {
    pub title: String,
    pub icon: Option<String>,
    pub items: Vec<ContextMenuItem>,
}

/// called with true when the edit menu opens and false when it closes
pub type InputEditMenuPresentedChange = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// asked before a paste edits the field, true means the owner took the clipboard
/// runs while the field is borrowed so read the clipboard only and defer touching the field
pub type InputPasteHandler = Rc<dyn Fn(&mut Window, &mut App) -> bool>;

/// where the open edit menu stands and whether it shows the submenu items
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputEditMenuPresentation {
    pub anchor: Point<Pixels>,
    pub in_submenu: bool,
}

/// one row of the edit menu the right click opens
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEditMenuCommand {
    Cut,
    Copy,
    Paste,
    SelectAll,
    Submenu,
}

impl InputEditMenuCommand {
    /// label key, same as the app Edit menu uses for these commands
    /// the submenu row shows its own title
    pub fn label_key(self) -> Option<&'static str> {
        match self {
            Self::Cut => Some("desktop_menu_cut"),
            Self::Copy => Some("copy"),
            Self::Paste => Some("desktop_menu_paste"),
            Self::SelectAll => Some("desktop_menu_select_all"),
            Self::Submenu => None,
        }
    }
}

impl EventEmitter<InputUiEvent> for InputUiState {}

impl Focusable for InputUiState {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// text fields of the app that hold focus, desktop stand in for the keyboard
/// one count for all fields so moving focus between fields posts no hide
#[derive(Default)]
struct KeyboardFocusedFields {
    fields: HashMap<EntityId, Subscription>,
    announced: bool,
}

impl Global for KeyboardFocusedFields {}

impl KeyboardFocusedFields {
    fn field_focused(cx: &mut Context<InputUiState>) {
        let id = cx.entity_id();
        let release = cx.on_release(move |_, cx: &mut App| Self::field_left(id, cx));
        let fields = cx.default_global::<Self>();
        fields.fields.insert(id, release);
        if !fields.announced {
            fields.announced = true;
            RiseAppEvents::default_center().post(UIResponder::KEYBOARD_WILL_SHOW_NOTIFICATION);
        }
    }

    fn field_left(id: EntityId, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }
        cx.global_mut::<Self>().fields.remove(&id);
        cx.defer(|cx| {
            let fields = cx.default_global::<Self>();
            if fields.announced && fields.fields.is_empty() {
                fields.announced = false;
                RiseAppEvents::default_center().post(UIResponder::KEYBOARD_WILL_HIDE_NOTIFICATION);
            }
        });
    }
}

impl InputUiState {
    /// empty field, installs the key bindings on first use
    pub fn new(mode: InputMode, cx: &mut Context<Self>) -> Self {
        install_key_bindings(cx);

        let focus_handle = cx.focus_handle().tab_stop(true);
        ScreenFocus::register_text_field(&focus_handle, cx);
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
            chromeless: false,
            height: None,
            max_length: None,
            max_visible_lines: 8,
            wraps: true,
            history: EditHistory::default(),
            blink: CaretBlink::default(),
            caret_swap: None,
            caret_swap_missed: false,
            focus_handle,
            focused: false,
            revision: 0,
            drag: None,
            goal_x: None,
            scroll: point(px(0.), px(0.)),
            line_height: px(0.),
            bounds: None,
            shaped: None,
            runs: None,
            base_typing_attributes: RunAttributes::default(),
            typing_attributes: None,
            delegate: InputUiTextViewDelegate::default(),
            published_selection: None,
            published_height: None,
            text_color: None,
            placeholder_color: None,
            caret_color: None,
            font_size: None,
            font_weight: None,
            minimum_line_height: None,
            baseline_lift: px(0.),
            text_align: TextAlign::Left,
            caret_reserve: px(0.),
            scroll_enabled: None,
            wheel_anchor: None,
            wheel_latch_until: None,
            edit_submenu: None,
            edit_menu: None,
            on_edit_menu_presented_change: None,
            edit_menu_escape: None,
            on_paste: None,
            escape_to_layer: true,
            escape_clears_text: false,
        }
    }

    /// empty single line field
    pub fn single_line(cx: &mut Context<Self>) -> Self {
        Self::new(InputMode::SingleLine, cx)
    }

    /// empty multi line composer
    pub fn multi_line(cx: &mut Context<Self>) -> Self {
        Self::new(InputMode::MultiLine, cx)
    }

    pub fn mode(&self) -> InputMode {
        self.mode
    }

    /// document as typed, real characters even in a secure field and no placeholder
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// range the ime is still composing in if any
    /// provisional, not an undo step yet
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn placeholder(&self) -> &SharedString {
        &self.placeholder
    }

    pub fn is_secure(&self) -> bool {
        self.secure
    }

    /// height this field alone uses or none for the theme one
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

    /// text shown while the document is empty
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        self.invalidate_display(cx);
    }

    /// one bullet per grapheme, contents stop leaving through copy cut the ime and palette
    pub fn set_secure(&mut self, secure: bool, cx: &mut Context<Self>) {
        self.secure = secure;
        self.invalidate_display(cx);
    }

    pub fn is_chromeless(&self) -> bool {
        self.chromeless
    }

    /// for a screen that draws its own field shape around the editor
    pub fn set_chromeless(&mut self, chromeless: bool, cx: &mut Context<Self>) {
        self.chromeless = chromeless;
        cx.notify();
    }

    /// text color this field alone uses or none for the theme one
    pub fn text_color(&self) -> Option<Hsla> {
        self.text_color
    }

    pub fn set_text_color(&mut self, color: Option<Hsla>, cx: &mut Context<Self>) {
        if self.text_color == color {
            return;
        }
        self.text_color = color;
        cx.notify();
    }

    /// placeholder color this field alone uses or none for the theme one
    pub fn placeholder_color(&self) -> Option<Hsla> {
        self.placeholder_color
    }

    pub fn set_placeholder_color(&mut self, color: Option<Hsla>, cx: &mut Context<Self>) {
        if self.placeholder_color == color {
            return;
        }
        self.placeholder_color = color;
        cx.notify();
    }

    /// caret color this field alone uses or none for the theme one
    pub fn caret_color(&self) -> Option<Hsla> {
        self.caret_color
    }

    pub fn set_caret_color(&mut self, color: Option<Hsla>, cx: &mut Context<Self>) {
        if self.caret_color == color {
            return;
        }
        self.caret_color = color;
        cx.notify();
    }

    /// font size this field alone uses or none for the theme body size
    /// line height follows in the theme proportion
    pub fn font_size(&self) -> Option<Pixels> {
        self.font_size
    }

    pub fn set_font_size(&mut self, size: Option<Pixels>, cx: &mut Context<Self>) {
        if self.font_size == size {
            return;
        }
        self.font_size = size;
        cx.notify();
    }

    /// minimum line height of every paragraph, none keeps the font line
    pub fn minimum_line_height(&self) -> Option<Pixels> {
        self.minimum_line_height
    }

    pub fn set_minimum_line_height(&mut self, height: Option<Pixels>, cx: &mut Context<Self>) {
        let height = height.filter(|height| *height > px(0.));
        if self.minimum_line_height == height {
            return;
        }
        self.minimum_line_height = height;
        cx.notify();
    }

    /// row height for a font line of line_height, never below the minimum line height
    pub fn held_line_height(&self, line_height: Pixels) -> Pixels {
        self.minimum_line_height
            .map_or(line_height, |minimum| line_height.max(minimum))
    }

    /// how far the baseline sits below where the font line alone would put it
    /// the height the minimum line height added above the glyphs in the last render
    pub fn baseline_lift(&self) -> Pixels {
        self.baseline_lift
    }

    pub(crate) fn set_baseline_lift(&mut self, lift: Pixels) {
        self.baseline_lift = lift.max(px(0.));
    }

    /// weight override for the field, none uses the theme
    pub fn font_weight(&self) -> Option<FontWeight> {
        self.font_weight
    }

    pub fn set_font_weight(&mut self, weight: Option<FontWeight>, cx: &mut Context<Self>) {
        if self.font_weight == weight {
            return;
        }
        self.font_weight = weight;
        cx.notify();
    }

    /// text alignment of the field
    pub fn text_align(&self) -> TextAlign {
        self.text_align
    }

    pub fn set_text_align(&mut self, align: TextAlign, cx: &mut Context<Self>) {
        if self.text_align == align {
            return;
        }
        self.text_align = align;
        cx.notify();
    }

    /// how far a row starts from the field leading edge under the text alignment
    pub fn row_alignment_shift(&self, row_index: usize, line_width: Pixels) -> Pixels {
        let row_width = self.shaped.as_ref().map_or(px(0.), |shaped| shaped.row_width(row_index));
        let free = (line_width - row_width).max(px(0.));
        let right_to_left = self.shaped.as_ref().is_some_and(|shaped| shaped.row_is_right_to_left(row_index));
        match self.text_align {
            TextAlign::Left if right_to_left => free,
            TextAlign::Left => px(0.),
            TextAlign::Center => px(f32::from(free) / 2.),
            TextAlign::Right => free,
        }
    }

    fn alignment_shift(&self, row_index: usize) -> Pixels {
        let line_width = self.bounds.map_or(px(0.), |bounds| bounds.size.width) - self.caret_reserve;
        self.row_alignment_shift(row_index, line_width)
    }

    pub fn scroll_enabled(&self) -> Option<bool> {
        self.scroll_enabled
    }

    pub fn set_scroll_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.scroll_enabled == Some(enabled) {
            return;
        }
        self.scroll_enabled = Some(enabled);
        cx.notify();
    }

    /// rows the element asks layout for, before the parent height applies
    pub fn layout_rows(&self) -> usize {
        if !self.mode.is_multiline() {
            return 1;
        }
        let rows = self.visual_row_count().max(1);
        match self.scroll_enabled {
            Some(false) => rows,
            Some(true) => 1,
            None => rows.min(self.max_visible_lines),
        }
    }

    /// true when the element takes all the height its parent gives
    pub fn fills_parent_height(&self) -> bool {
        self.mode.is_multiline() && self.scroll_enabled == Some(true)
    }

    /// submenu the edit menu carries while something is selected
    pub fn edit_submenu(&self) -> Option<&InputEditSubmenu> {
        self.edit_submenu.as_ref()
    }

    pub fn set_edit_submenu(&mut self, submenu: Option<InputEditSubmenu>, cx: &mut Context<Self>) {
        self.edit_submenu = submenu;
        if let Some(menu) = self.edit_menu.as_mut() {
            menu.in_submenu = false;
        }
        cx.notify();
    }

    /// watches the edit menu opening and closing
    pub fn set_on_edit_menu_presented_change(&mut self, callback: Option<InputEditMenuPresentedChange>) {
        self.on_edit_menu_presented_change = callback;
    }

    /// desktop only, lets the owner take a clipboard the field cant use
    /// eg files or an image in a composer
    pub fn set_on_paste(&mut self, handler: Option<InputPasteHandler>) {
        self.on_paste = handler;
    }

    pub fn edit_menu(&self) -> Option<InputEditMenuPresentation> {
        self.edit_menu
    }

    /// rows of the edit menu in order, system commands this field can do now
    /// submenu sits at index min(1, count) while the selection is not empty
    pub fn edit_menu_commands(&self) -> Vec<InputEditMenuCommand> {
        let has_selection = !self.selection.is_empty() && !self.display.is_placeholder;
        let can_copy = has_selection && !self.secure;
        let mut commands = Vec::with_capacity(5);
        if can_copy && !self.disabled {
            commands.push(InputEditMenuCommand::Cut);
        }
        if can_copy {
            commands.push(InputEditMenuCommand::Copy);
        }
        if !self.disabled {
            commands.push(InputEditMenuCommand::Paste);
        }
        if !self.text.is_empty() {
            commands.push(InputEditMenuCommand::SelectAll);
        }
        if has_selection && self.edit_submenu.is_some() {
            let index = commands.len().min(1);
            commands.insert(index, InputEditMenuCommand::Submenu);
        }
        commands
    }

    /// right click at a window point
    /// a click outside the selection moves the caret there first and then the menu opens
    pub fn open_edit_menu(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.offset_at(position);
        let selected = self.selection.range();
        if self.selection.is_empty() || offset < selected.start || offset > selected.end {
            self.set_selection(Selection::cursor(offset), cx);
        }
        let was_open = self.edit_menu.is_some();
        self.edit_menu = Some(InputEditMenuPresentation {
            anchor: position,
            in_submenu: false,
        });
        cx.notify();
        if !was_open {
            self.edit_menu_escape = Some(Self::intercept_edit_menu_escape(window, cx));
            self.notify_edit_menu_presented(true, window, cx);
        }
    }

    fn intercept_edit_menu_escape(window: &Window, cx: &mut Context<Self>) -> Subscription {
        let menu_window = window.window_handle();
        let field = cx.weak_entity();
        cx.intercept_keystrokes(move |event: &KeystrokeEvent, window: &mut Window, cx: &mut App| {
            let keystroke = &event.keystroke;
            if window.window_handle() != menu_window
                || keystroke.key != "escape"
                || keystroke.modifiers.modified()
            {
                return;
            }
            let Some(field) = field.upgrade() else {
                return;
            };
            if field.read(cx).edit_menu.is_none() {
                return;
            }
            cx.stop_propagation();
            field.update(cx, |field, cx| field.close_edit_menu(window, cx));
        })
    }

    pub fn open_edit_submenu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.edit_menu.as_mut() {
            menu.in_submenu = true;
            cx.notify();
        }
    }

    pub fn close_edit_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit_menu.take().is_none() {
            return;
        }
        self.edit_menu_escape = None;
        cx.notify();
        self.notify_edit_menu_presented(false, window, cx);
    }

    /// runs one system row and closes the menu
    pub fn perform_edit_menu_command(
        &mut self,
        command: InputEditMenuCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if command == InputEditMenuCommand::Submenu {
            self.open_edit_submenu(cx);
            return;
        }
        self.close_edit_menu(window, cx);
        match command {
            InputEditMenuCommand::Cut => self.cut(&input_actions::Cut, window, cx),
            InputEditMenuCommand::Copy => self.copy(&input_actions::Copy, window, cx),
            InputEditMenuCommand::Paste => self.paste(&input_actions::Paste, window, cx),
            InputEditMenuCommand::SelectAll => self.select_all(&input_actions::SelectAll, window, cx),
            InputEditMenuCommand::Submenu => {}
        }
    }

    fn notify_edit_menu_presented(&self, presented: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(callback) = self.on_edit_menu_presented_change.clone() {
            window.defer(cx, move |window, cx| callback(presented, window, cx));
        }
    }

    /// refuses every edit but keeps the text on screen and selectable
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        cx.notify();
    }

    /// limit in characters and not bytes, a paste over it is truncated and not rejected
    pub fn set_max_length(&mut self, max_length: Option<usize>, cx: &mut Context<Self>) {
        self.max_length = max_length;
        cx.notify();
    }

    /// how tall a composer may grow before it scrolls, at least one line
    pub fn set_max_visible_lines(&mut self, lines: usize, cx: &mut Context<Self>) {
        self.max_visible_lines = lines.max(1);
        cx.notify();
    }

    /// true when a multi line field breaks long lines at its width
    /// on by default, single line never wraps
    pub fn wraps(&self) -> bool {
        self.wraps && self.mode.is_multiline()
    }

    pub fn set_wraps(&mut self, wraps: bool, cx: &mut Context<Self>) {
        if self.wraps == wraps {
            return;
        }
        self.wraps = wraps;
        self.shaped = None;
        cx.notify();
    }

    /// rows on screen for the whole document, wrapped rows once shaped or hard lines otherwise
    pub fn visual_row_count(&self) -> usize {
        let lines = self.display.line_count();
        self.shaped
            .as_ref()
            .filter(|_| self.wraps())
            .and_then(ShapedInput::wrapped_row_count)
            .map_or(lines, |rows| rows.max(lines))
    }

    /// replaces the whole document as one undoable step
    pub fn set_text(&mut self, text: impl AsRef<str>, cx: &mut Context<Self>) {
        let whole = 0..self.text.len();
        self.replace(whole, text.as_ref(), EditKind::Atomic, cx);
    }

    /// replaces the whole document and forgets all history
    pub fn reset(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        let incoming = text.into();
        self.text = self.fit(&incoming, &(0..self.text.len()));
        if self.runs.is_some() {
            self.runs = Some(AttributedRuns::new(
                self.text.len(),
                self.base_typing_attributes.clone(),
            ));
        }
        self.typing_attributes = None;
        self.selection = Selection::cursor(self.text.len());
        self.marked = None;
        self.composition = None;
        self.history.clear();
        self.scroll = point(px(0.), px(0.));
        self.after_edit(cx);
    }

    /// empties the field as one undoable step, reset is the version you cant undo
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    /// moves the caret or selection, offsets are clamped to char boundaries
    pub fn set_selection(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.selection = Selection::new(
            text_boundaries::clamp_to_char_boundary(&self.text, selection.anchor),
            text_boundaries::clamp_to_char_boundary(&self.text, selection.head),
        );
        self.history.break_coalescing();
        self.publish_selection(cx);
        cx.notify();
    }

    /// selects the whole document from code
    pub fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.selection = Selection::new(0, self.text.len());
        self.history.break_coalescing();
        self.publish_selection(cx);
        cx.notify();
    }

    /// types text over the selection as one undo step, respects the char limit
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        self.replace(self.selection.range(), text, EditKind::Atomic, cx);
    }

    pub fn set_delegate(&mut self, delegate: InputUiTextViewDelegate) {
        self.delegate = delegate;
        self.published_selection = None;
        self.published_height = None;
    }

    /// true while an input method is composing
    /// nothing may reinstall the text meanwhile or the syllable being built is lost
    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    pub fn selected_range(&self) -> NSRange {
        ns_range::byte_to_utf16_range(&self.text, self.selection.range())
    }

    /// sets the selected range, clamped to the text
    pub fn set_selected_range(&mut self, range: NSRange, cx: &mut Context<Self>) {
        let utf16 = range.location..range.location.saturating_add(range.length);
        let bytes = text_boundaries::utf8_range_from_utf16(&self.text, &utf16);
        self.set_selection(Selection::new(bytes.start, bytes.end), cx);
    }

    /// sets the text from pieces joined together, each with its attributes
    /// plus the typing attributes
    /// keeps the caret clamped to the new text and starts a fresh undo history
    pub fn set_attributed_text(
        &mut self,
        pieces: impl IntoIterator<Item = (String, RunAttributes)>,
        typing: RunAttributes,
        cx: &mut Context<Self>,
    ) {
        if self.is_composing() {
            return;
        }
        let mut text = String::new();
        let mut lengths = Vec::new();
        for (piece, attributes) in pieces {
            text.push_str(&piece);
            lengths.push((piece.len(), attributes));
        }
        let selected = self.selected_range();
        self.text = text;
        self.runs = Some(AttributedRuns::from_pieces(lengths));
        self.base_typing_attributes = typing;
        self.typing_attributes = None;
        self.marked = None;
        self.composition = None;
        self.history.clear();
        let utf16 = selected.location..selected.location.saturating_add(selected.length);
        let bytes = text_boundaries::utf8_range_from_utf16(&self.text, &utf16);
        self.selection = Selection::new(bytes.start, bytes.end);
        self.after_edit(cx);
    }

    /// the runs over the text, none for a plain field
    pub fn attributed_runs(&self) -> Option<&AttributedRuns> {
        self.runs.as_ref()
    }

    /// tallest attachment image in the text, a row grows to hold it
    /// none without attachments or in a secure field
    pub fn tallest_attachment(&self) -> Option<Pixels> {
        if self.is_secure() {
            return None;
        }
        self.runs.as_ref()?.tallest_attachment(&self.text)
    }

    /// the text cut at its runs
    /// a plain field reports its whole text as one piece of the typing attributes
    pub fn attributed_pieces(&self) -> Vec<(&str, &RunAttributes)> {
        match self.runs.as_ref() {
            Some(runs) => runs
                .runs()
                .filter_map(|(range, attributes)| {
                    self.text.get(range).map(|piece| (piece, attributes))
                })
                .collect(),
            None if self.text.is_empty() => Vec::new(),
            None => vec![(self.text.as_str(), &self.base_typing_attributes)],
        }
    }

    pub fn typed_pieces<T: Any>(&self) -> Vec<(&str, &AttributeContainer, Option<&T>)> {
        self.attributed_pieces()
            .into_iter()
            .map(|(piece, attributes)| (piece, &attributes.attributes, attributes.tag::<T>()))
            .collect()
    }

    /// what the next typed character takes, never an attachment
    pub fn typing_attributes(&self) -> RunAttributes {
        if let Some(typing) = self.typing_attributes.clone() {
            return typing.without_attachment();
        }
        self.runs
            .as_ref()
            .and_then(|runs| runs.inherited_for(&self.selection.range()))
            .unwrap_or_else(|| self.base_typing_attributes.clone())
    }

    /// sets typing attributes until the selection next moves
    pub fn set_typing_attributes(&mut self, attributes: RunAttributes) {
        if self.runs.is_none() {
            self.runs = Some(AttributedRuns::new(
                self.text.len(),
                self.base_typing_attributes.clone(),
            ));
        }
        self.typing_attributes = Some(attributes);
        self.published_selection = Some(self.selected_range());
    }

    pub fn update_attributes(
        &mut self,
        range: Range<usize>,
        f: impl Fn(&mut RunAttributes),
        cx: &mut Context<Self>,
    ) {
        let range = self.clamp_range(range);
        let runs = self.runs.get_or_insert_with(|| {
            AttributedRuns::new(self.text.len(), self.base_typing_attributes.clone())
        });
        runs.update(range, f);
        self.invalidate_display(cx);
    }

    /// height that fits, one line height per row (wrapped rows when wrapping) and at least one line
    /// zero before the first layout when line height is unknown
    pub fn fitted_height(&self) -> Pixels {
        self.line_height * (self.visual_row_count().max(1) as f32)
    }

    pub fn publish_fitted_height(&mut self, cx: &mut Context<Self>) {
        if self.line_height <= px(0.) {
            return;
        }
        let height = self.fitted_height();
        let moved = self.published_height.is_none_or(|published| {
            (f32::from(height) - f32::from(published)).abs() > HEIGHT_CHANGE_THRESHOLD
        });
        if !moved {
            return;
        }
        self.published_height = Some(height);
        if let Some(callback) = self.delegate.did_change_height.clone() {
            cx.defer(move |cx| callback(height, cx));
        }
    }

    /// true when the caret paints this frame, off on the blink dark half and while disabled
    pub fn caret_is_visible(&self) -> bool {
        self.blink.is_visible() && !self.disabled
    }

    pub fn caret_is_shown(&self) -> bool {
        self.blink.is_focused() && !self.disabled
    }

    pub(crate) fn note_caret_painted(&mut self, swap: Option<CaretSwap>) {
        self.caret_swap = swap;
    }

    fn show_blink_phase(&mut self, cx: &mut Context<Self>) {
        let Some(swap) = self.caret_swap else {
            return;
        };
        let color = if self.caret_is_visible() { swap.color } else { transparent_black() };
        let path = caret_path(Bounds::new(Point::default(), swap.size));
        let swapped = swap
            .window
            .update(cx, |_, window, _| window.swap_paths(vec![(swap.key, path, color.into())]))
            .unwrap_or(false);
        if swapped {
            self.caret_swap_missed = false;
            return;
        }
        self.caret_swap = None;
        if !self.caret_swap_missed {
            self.caret_swap_missed = true;
            cx.notify();
            return;
        }
        let frame_shows_dark = self.blink.is_visible();
        self.blink.interrupt();
        if frame_shows_dark {
            cx.notify();
        }
    }

    /// what is on screen, placeholder when empty and bullets when secure split into lines
    pub fn display(&self) -> &DisplayText {
        &self.display
    }

    pub fn scroll(&self) -> Point<Pixels> {
        self.scroll
    }

    pub fn line_height(&self) -> Pixels {
        self.line_height
    }

    /// shaped rows from the last prepaint or none before the first one
    pub fn shaped(&self) -> Option<&ShapedInput> {
        self.shaped.as_ref()
    }

    /// bumped on every change to the document or to what is displayed, shaping keys on it
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn set_shaped(&mut self, shaped: ShapedInput) {
        self.shaped = Some(shaped);
    }

    pub fn wheel_scroll_holds(&self) -> bool {
        self.wheel_anchor == Some((self.revision, self.selection))
    }

    pub fn scroll_by_wheel(&mut self, delta_y: Pixels, cx: &mut Context<Self>) -> bool {
        const WHEEL_LATCH: Duration = Duration::from_millis(300);
        let now = RiseClock::monotonic();
        let latched = self.wheel_latch_until.is_some_and(|until| now < until);
        let Some(bounds) = self.bounds else {
            return false;
        };
        if !self.mode.is_multiline() || self.line_height <= px(0.) {
            return false;
        }
        let content = self.line_height * (self.visual_row_count() as f32);
        let overflow = (content - bounds.size.height).max(px(0.));
        if overflow <= px(0.) {
            return false;
        }
        let next = (self.scroll.y - delta_y).clamp(px(0.), overflow);
        if next == self.scroll.y {
            if latched {
                self.wheel_latch_until = Some(now + WHEEL_LATCH);
            }
            return latched;
        }
        self.scroll.y = next;
        self.wheel_anchor = Some((self.revision, self.selection));
        self.wheel_latch_until = Some(now + WHEEL_LATCH);
        cx.notify();
        true
    }

    pub fn set_viewport(&mut self, bounds: Bounds<Pixels>, line_height: Pixels) {
        self.bounds = Some(bounds);
        self.line_height = line_height;
    }

    pub fn set_caret_reserve(&mut self, caret_width: Pixels) {
        self.caret_reserve = caret_width;
    }

    pub fn set_scroll(&mut self, scroll: Point<Pixels>) {
        self.scroll = scroll;
    }

    /// called from prepaint, returns early on no change since a notify per frame never settles
    pub fn sync_focus(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if focused {
            KeyboardFocusedFields::field_focused(cx);
        } else {
            let id = cx.entity_id();
            KeyboardFocusedFields::field_left(id, cx);
        }
        if focused {
            let epoch = self.blink.focus();
            self.schedule_resume(epoch, cx);
        } else {
            self.blink.blur();
            self.drag = None;
        }
        let editing = if focused {
            self.delegate.did_begin_editing.clone()
        } else {
            self.delegate.did_end_editing.clone()
        };
        if let Some(callback) = editing {
            cx.defer(move |cx| callback(cx));
        }
        if focused && let Some(callback) = self.delegate.did_change_selection.clone() {
            let selected = self.selected_range();
            cx.defer(move |cx| callback(selected, cx));
        }
        cx.notify();
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// hard display line the caret is on
    pub fn caret_line(&self) -> usize {
        self.display.line_index_for_source(self.selection.head)
    }

    /// painted row the caret is on, the row a composer scrolls to
    pub fn caret_row(&self) -> usize {
        self.row_of(self.selection.head)
    }

    fn row_of(&self, source_offset: usize) -> usize {
        let line_index = self.display.line_index_for_source(source_offset);
        let offset_in_line = self.display.offset_within_line(line_index, source_offset);
        self.shaped
            .as_ref()
            .and_then(|shaped| shaped.row_for(line_index, offset_in_line))
            .unwrap_or(line_index)
    }

    /// document offset under a window point, clamped to the shaped text
    pub fn offset_at(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(shaped)) = (self.bounds, self.shaped.as_ref()) else {
            return self.selection.head;
        };
        if self.display.is_placeholder {
            return 0;
        }

        let rows = shaped.row_range();
        let local_y = position.y - bounds.top() + self.scroll.y;
        let row = if self.line_height > px(0.) {
            (f32::from(local_y) / f32::from(self.line_height)).floor()
        } else {
            0.0
        };
        let row = if row < 0.0 { 0 } else { row as usize };
        let row = row.clamp(rows.start, rows.end.saturating_sub(1));

        let x = position.x - bounds.left() + self.scroll.x - self.alignment_shift(row);
        let Some((line_index, offset_in_line)) = shaped.offset_in_row(row, x) else {
            return self.selection.head;
        };
        self.display
            .source_offset_within_line(line_index, offset_in_line)
    }

    /// places the caret and starts a drag at the click granularity
    /// click_count above three keeps selecting by line, extend is shift click and only the head moves
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
        self.publish_selection(cx);
        cx.notify();
    }

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
        self.publish_selection(cx);
        cx.notify();
    }

    /// ends the drag, selection stays as it was left
    pub fn on_mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    /// true while a selection drag is in flight
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// caret one grapheme left, a selection collapses to its start
    pub fn move_left(
        &mut self,
        _: &input_actions::MoveLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.horizontal(false, false, cx);
    }

    /// caret one grapheme right, a selection collapses to its end
    pub fn move_right(
        &mut self,
        _: &input_actions::MoveRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.horizontal(true, false, cx);
    }

    /// grows the selection one grapheme left, anchor stays
    pub fn select_left(
        &mut self,
        _: &input_actions::SelectLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.horizontal(false, true, cx);
    }

    /// grows the selection one grapheme right, anchor stays
    pub fn select_right(
        &mut self,
        _: &input_actions::SelectRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.horizontal(true, true, cx);
    }

    pub fn move_word_left(
        &mut self,
        _: &input_actions::MoveWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::previous_word_boundary(&self.text, self.selection.head);
        self.move_caret(target, false, cx);
    }

    pub fn move_word_right(
        &mut self,
        _: &input_actions::MoveWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::next_word_boundary(&self.text, self.selection.head);
        self.move_caret(target, false, cx);
    }

    pub fn select_word_left(
        &mut self,
        _: &input_actions::SelectWordLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::previous_word_boundary(&self.text, self.selection.head);
        self.move_caret(target, true, cx);
    }

    pub fn select_word_right(
        &mut self,
        _: &input_actions::SelectWordRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = text_boundaries::next_word_boundary(&self.text, self.selection.head);
        self.move_caret(target, true, cx);
    }

    pub fn move_line_start(
        &mut self,
        _: &input_actions::MoveLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.line_edge(false);
        self.move_caret(target, false, cx);
    }

    pub fn move_line_end(
        &mut self,
        _: &input_actions::MoveLineEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.line_edge(true);
        self.move_caret(target, false, cx);
    }

    pub fn select_line_start(
        &mut self,
        _: &input_actions::SelectLineStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.line_edge(false);
        self.move_caret(target, true, cx);
    }

    pub fn select_line_end(
        &mut self,
        _: &input_actions::SelectLineEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.line_edge(true);
        self.move_caret(target, true, cx);
    }

    /// caret to the very start of the document
    pub fn move_document_start(
        &mut self,
        _: &input_actions::MoveDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_caret(0, false, cx);
    }

    pub fn move_document_end(
        &mut self,
        _: &input_actions::MoveDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_caret(self.text.len(), false, cx);
    }

    pub fn select_document_start(
        &mut self,
        _: &input_actions::SelectDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_caret(0, true, cx);
    }

    pub fn select_document_end(
        &mut self,
        _: &input_actions::SelectDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_caret(self.text.len(), true, cx);
    }

    /// caret one display line up keeping its goal column, propagates in a single line field
    pub fn move_up(&mut self, _: &input_actions::MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(-1, false, cx);
    }

    /// caret one display line down, propagates in a single line field
    pub fn move_down(
        &mut self,
        _: &input_actions::MoveDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(1, false, cx);
    }

    /// grows the selection one display line up
    pub fn select_up(
        &mut self,
        _: &input_actions::SelectUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(-1, true, cx);
    }

    pub fn select_down(
        &mut self,
        _: &input_actions::SelectDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vertical(1, true, cx);
    }

    /// selects the whole document
    pub fn select_all(
        &mut self,
        _: &input_actions::SelectAll,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_all_text(cx);
    }

    /// deletes the selection or one grapheme before the caret, a whole emoji per press
    pub fn backspace(
        &mut self,
        _: &input_actions::Backspace,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_backwards(text_boundaries::previous_grapheme_boundary, cx);
    }

    /// deletes the selection or back to the previous word boundary
    pub fn backspace_word(
        &mut self,
        _: &input_actions::BackspaceWord,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_backwards(text_boundaries::previous_word_boundary, cx);
    }

    /// deletes the selection or one grapheme after the caret
    pub fn delete(&mut self, _: &input_actions::Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.delete_forwards(text_boundaries::next_grapheme_boundary, cx);
    }

    /// deletes the selection or back to the start of the caret line (cmd backspace)
    pub fn backspace_line(
        &mut self,
        _: &input_actions::BackspaceLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_backwards(text_boundaries::line_start, cx);
    }

    /// deletes the selection or forward to the end of the caret line (cmd delete)
    pub fn delete_line(
        &mut self,
        _: &input_actions::DeleteLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_forwards(text_boundaries::line_end, cx);
    }

    /// deletes the selection or forward to the next word boundary
    pub fn delete_word(
        &mut self,
        _: &input_actions::DeleteWord,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_forwards(text_boundaries::next_word_boundary, cx);
    }

    /// copies the selection, a secure field copies nothing
    pub fn copy(&mut self, _: &input_actions::Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    /// copies the selection and deletes it, a secure field does neither
    pub fn cut(&mut self, _: &input_actions::Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.copy_selection(cx) {
            return;
        }
        self.replace_by_user(self.selection.range(), "", EditKind::Atomic, cx);
    }

    /// pastes over the selection as one undo step
    /// line breaks become spaces in a field and stay breaks in a composer
    pub fn paste(&mut self, _: &input_actions::Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if let Some(handler) = self.on_paste.clone() {
            if handler(window, cx) {
                return;
            }
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.replace_by_user(self.selection.range(), &text, EditKind::Atomic, cx);
    }

    /// undoes one step and restores its selection, a typing run or ime composition is one step
    pub fn undo(&mut self, _: &input_actions::Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let Some(edit) = self.history.undo() else {
            return;
        };
        self.typing_attributes = None;
        self.splice_text(edit.undone_range(), &edit.removed);
        self.selection = Selection::new(edit.selection_before.start, edit.selection_before.end);
        self.after_history(cx);
        self.notify_did_change(cx);
    }

    /// reapplies the last undone step
    pub fn redo(&mut self, _: &input_actions::Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let Some(edit) = self.history.redo() else {
            return;
        };
        self.typing_attributes = None;
        self.splice_text(edit.redone_range(), &edit.inserted);
        self.selection = Selection::new(edit.selection_after.start, edit.selection_after.end);
        self.after_history(cx);
        self.notify_did_change(cx);
    }

    /// enter, emits Submitted in a field and a line break in a composer
    pub fn confirm(&mut self, _: &input_actions::Confirm, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if self.mode.is_multiline() {
            self.replace_by_user(self.selection.range(), "\n", EditKind::Typing, cx);
        } else {
            cx.emit(InputUiEvent::Submitted);
        }
    }

    /// escape, takes back a composition first and only otherwise emits Cancelled
    /// a field with no escape of its own passes the key to the layer around it, so escape closes that
    pub fn cancel(&mut self, _: &input_actions::Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.is_some() {
            self.cancel_composition(cx);
            return;
        }
        let clears_first = self.escape_clears_text && !self.text.is_empty();
        cx.emit(InputUiEvent::Cancelled);
        if !clears_first && self.escape_to_layer && crate::layer_keys::focus_is_inside_layer(window) {
            cx.propagate();
        }
    }

    /// turn off for a field whose owner answers escape itself, so the layer around doesnt close too
    pub fn set_escape_to_layer(&mut self, passes: bool) {
        self.escape_to_layer = passes;
    }

    pub fn set_escape_clears_text(&mut self, clears: bool) {
        self.escape_clears_text = clears;
    }

    /// tab, focus moves to the next tab stop of the window
    pub fn focus_next(&mut self, _: &input_actions::FocusNext, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_next(cx);
    }

    pub fn focus_previous(&mut self, _: &input_actions::FocusPrevious, window: &mut Window, cx: &mut Context<Self>) {
        window.focus_prev(cx);
    }

    /// opens the system emoji and symbol palette, refused for a secure field
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
        if !self.mode.is_multiline() {
            cx.propagate();
            return;
        }

        let line_index = self.display.line_index_for_source(self.selection.head);
        let row = self.caret_row();
        let target_row = row.saturating_add_signed(delta);
        if target_row >= self.visual_row_count() {
            let target = if delta < 0 { 0 } else { self.text.len() };
            self.move_caret(target, extend, cx);
            return;
        }

        let goal_x = self.goal_x.or_else(|| {
            let offset_in_line = self
                .display
                .offset_within_line(line_index, self.selection.head);
            let source_row = self.row_of(self.selection.head);
            self.shaped
                .as_ref()
                .map(|shaped| shaped.x_for(line_index, offset_in_line) + self.alignment_shift(source_row))
        });

        let landed = match (goal_x, self.shaped.as_ref()) {
            (Some(x), Some(shaped)) => shaped.offset_in_row(target_row, x - self.alignment_shift(target_row)),
            _ => None,
        };
        let target = match landed {
            Some((target_line, offset_in_line)) => self
                .display
                .source_offset_within_line(target_line, offset_in_line),
            None => self.column_preserving_offset(line_index, target_row),
        };

        self.move_caret(target, extend, cx);
        self.goal_x = goal_x;
    }

    fn line_edge(&self, end: bool) -> usize {
        let hard = if end {
            text_boundaries::line_end(&self.text, self.selection.head)
        } else {
            text_boundaries::line_start(&self.text, self.selection.head)
        };
        let Some(shaped) = self.shaped.as_ref().filter(|shaped| shaped.wraps()) else {
            return hard;
        };
        let row_index = self.caret_row();
        let Some(row) = shaped.row(row_index) else {
            return hard;
        };
        if !end {
            return self.display.source_offset_within_line(row.line, row.start);
        }
        let continues = shaped
            .row(row_index + 1)
            .is_some_and(|next| next.line == row.line);
        if !continues {
            return hard;
        }
        shaped
            .offset_in_row(row_index, Pixels::MAX)
            .map_or(hard, |(line_index, offset_in_line)| {
                self.display
                    .source_offset_within_line(line_index, offset_in_line)
            })
    }

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
        self.publish_selection(cx);
        cx.notify();
    }

    fn delete_backwards(&mut self, boundary: fn(&str, usize) -> usize, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            self.replace_by_user(self.selection.range(), "", EditKind::Deleting, cx);
            return;
        }
        let head = self.selection.head;
        let start = boundary(&self.text, head);
        if start == head {
            if !self.disabled {
                self.should_change_text(&(head..head), "", cx);
            }
            return;
        }
        self.replace_by_user(start..head, "", EditKind::Deleting, cx);
    }

    fn delete_forwards(&mut self, boundary: fn(&str, usize) -> usize, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            self.replace_by_user(self.selection.range(), "", EditKind::Deleting, cx);
            return;
        }
        let head = self.selection.head;
        let end = boundary(&self.text, head);
        if end == head {
            return;
        }
        self.replace_by_user(head..end, "", EditKind::Deleting, cx);
    }

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

        self.splice_text(range.clone(), &insertion);
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

    fn replace_by_user(
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
        if !self.should_change_text(&range, new_text, cx) {
            return;
        }
        let revision = self.revision;
        self.replace(range, new_text, kind, cx);
        if self.revision != revision {
            self.notify_did_change(cx);
        }
    }

    fn should_change_text(
        &mut self,
        range: &Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(callback) = self.delegate.should_change_text_in.clone() else {
            return true;
        };
        let range = ns_range::byte_to_utf16_range(&self.text, range.clone());
        callback(range, text, cx)
    }

    fn notify_did_change(&mut self, cx: &mut Context<Self>) {
        if let Some(callback) = self.delegate.did_change.clone() {
            callback(self, cx);
        }
    }

    fn publish_selection(&mut self, cx: &mut Context<Self>) {
        if self.marked.is_some() {
            return;
        }
        if self.delegate.did_change_selection.is_none() && self.typing_attributes.is_none() {
            return;
        }
        let selected = self.selected_range();
        if self.published_selection == Some(selected) {
            return;
        }
        self.published_selection = Some(selected);
        self.typing_attributes = None;
        if let Some(callback) = self.delegate.did_change_selection.clone() {
            callback(selected, cx);
        }
    }

    fn splice_text(&mut self, range: Range<usize>, insertion: &str) {
        if let Some(runs) = self.runs.as_mut() {
            let attributes = self
                .typing_attributes
                .clone()
                .or_else(|| runs.inherited_for(&range))
                .unwrap_or_else(|| self.base_typing_attributes.clone())
                .without_attachment();
            runs.splice(range.clone(), insertion.len(), attributes);
        }
        self.text.replace_range(range, insertion);
    }

    fn clamp_range(&self, range: Range<usize>) -> Range<usize> {
        let start = text_boundaries::clamp_to_char_boundary(&self.text, range.start);
        let end = text_boundaries::clamp_to_char_boundary(&self.text, range.end.max(range.start));
        start..end.max(start)
    }

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
        self.publish_selection(cx);
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
                    this.caret_swap_missed = false;
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
        self.splice_text(marked.clone(), "");
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
        self.notify_did_change(cx);
    }
}

fn schedule_tick_after(delay: Duration, epoch: u64, cx: &mut Context<InputUiState>) {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(delay).await;
        this.update(cx, |this, cx| {
            if this.blink.tick(epoch) {
                this.show_blink_phase(cx);
                schedule_tick_after(BLINK_INTERVAL, epoch, cx);
            }
        })
        .ok();
    })
    .detach();
}

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
        self.publish_selection(cx);
        self.notify_did_change(cx);
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
            self.replace_by_user(range, new_text, EditKind::Typing, cx);
            return;
        };

        let insertion = self.fit(new_text, &range);
        let at = composition.at.min(range.start);
        self.splice_text(range.clone(), &insertion);
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
        self.notify_did_change(cx);
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
        self.splice_text(range.clone(), &insertion);
        let marked = range.start..range.start + insertion.len();

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
        self.notify_did_change(cx);
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
        let row = self.row_of(range.start);

        let top = element_bounds.top() + self.line_height * (row as f32) - self.scroll.y;
        let shift = self.alignment_shift(row);
        let left = element_bounds.left() - self.scroll.x
            + shift
            + shaped.x_for(
                line_index,
                self.display.offset_within_line(line_index, range.start),
            );
        let right_x = if self.row_of(range.end) == row {
            shaped.x_for(
                line_index,
                self.display.offset_within_line(line_index, range.end),
            )
        } else {
            shaped.row_width(row)
        };
        let right = element_bounds.left() - self.scroll.x + shift + right_x;

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

    const CYRILLIC: &str = "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}";
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
    fn line_wise_delete_stops_at_the_line_break(cx: &mut TestAppContext) {
        let composer = open(InputMode::MultiLine, cx);
        edit(&composer, cx, |state, window, cx| {
            state.replace_text_in_range(None, "ab\ncd", window, cx);
            state.backspace_line(&input_actions::BackspaceLine, window, cx);
            assert_eq!(state.text(), "ab\n");
            state.backspace_line(&input_actions::BackspaceLine, window, cx);
            assert_eq!(state.text(), "ab\n", "at a line start there is nothing of the line to delete");

            state.move_document_start(&input_actions::MoveDocumentStart, window, cx);
            state.delete_line(&input_actions::DeleteLine, window, cx);
            assert_eq!(state.text(), "\n");
        });
    }

    #[gpui::test]
    fn word_wise_delete_works_at_both_ends(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.replace_text_in_range(None, "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{43c}\u{438}\u{440}", window, cx);
            state.backspace_word(&BackspaceWord, window, cx);
            assert_eq!(state.text(), "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} ");
            state.backspace_word(&BackspaceWord, window, cx);
            assert_eq!(state.text(), "");
            state.backspace_word(&BackspaceWord, window, cx);
            assert_eq!(state.text(), "");

            state.replace_text_in_range(None, "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{43c}\u{438}\u{440}", window, cx);
            state.move_document_start(&input_actions::MoveDocumentStart, window, cx);
            state.delete_word(&DeleteWord, window, cx);
            assert_eq!(state.text(), " \u{43c}\u{438}\u{440}");
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
            state.replace_text_in_range(None, "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}", window, cx);
            state.select_all(&SelectAll, window, cx);

            state.replace_and_mark_text_in_range(None, "に", Some(0..1), window, cx);
            assert_eq!(state.text(), "に");

            state.replace_text_in_range(None, "日", window, cx);
            assert_eq!(state.text(), "日");

            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}");
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
        type_text(&field, cx, "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}");
        assert_eq!(text_of(&field, cx), "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}");

        edit(&field, cx, |state, window, cx| {
            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "");
            assert!(!state.can_undo());

            state.redo(&input_actions::Redo, window, cx);
            assert_eq!(state.text(), "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}");
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

        type_text(&field, cx, "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\u{43c}\u{438}\u{440}");
        assert_eq!(text_of(&field, cx), "\u{41f}\u{440}\u{438}\u{432}\u{435}");

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
    fn a_paste_taken_by_the_owner_leaves_the_text_alone(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        cx.write_to_clipboard(ClipboardItem::new_string("pasted".into()));
        edit(&field, cx, |state, window, cx| {
            state.set_on_paste(Some(std::rc::Rc::new(|_: &mut gpui::Window, _: &mut gpui::App| true)));
            state.paste(&Paste, window, cx);
            assert_eq!(state.text(), "");
        });
        edit(&field, cx, |state, window, cx| {
            state.set_on_paste(Some(std::rc::Rc::new(|_: &mut gpui::Window, _: &mut gpui::App| false)));
            state.paste(&Paste, window, cx);
            assert_eq!(state.text(), "pasted");
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
            state.replace_text_in_range(None, "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}", window, cx);
            state.select_all(&SelectAll, window, cx);
            state.cut(&Cut, window, cx);
            assert_eq!(state.text(), "");
        });
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}".to_owned())
        );
    }

    #[gpui::test]
    fn a_secure_field_shows_one_bullet_per_grapheme(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, window, cx| {
            state.set_secure(true, cx);
            state.replace_text_in_range(None, "a👍🏻\u{431}", window, cx);
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
            state.set_text("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}", cx);
            assert_eq!(state.text(), "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}");
            state.set_text("\u{43c}\u{438}\u{440}", cx);
            assert_eq!(state.text(), "\u{43c}\u{438}\u{440}");
            state.undo(&Undo, window, cx);
            assert_eq!(state.text(), "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}");
        });
    }

    #[gpui::test]
    fn reset_forgets_the_history(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, _window, cx| {
            state.set_text("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}", cx);
            state.reset("\u{43c}\u{438}\u{440}", cx);
            assert_eq!(state.text(), "\u{43c}\u{438}\u{440}");
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
    fn only_a_multi_line_field_wraps_and_rows_fall_back_to_lines(cx: &mut TestAppContext) {
        let field = open(InputMode::SingleLine, cx);
        edit(&field, cx, |state, _window, cx| {
            state.set_wraps(true, cx);
            assert!(!state.wraps(), "a single-line field scrolls sideways instead");
        });

        let composer = open(InputMode::MultiLine, cx);
        edit(&composer, cx, |state, window, cx| {
            assert!(state.wraps(), "a composer wraps by default");
            state.set_wraps(false, cx);
            assert!(!state.wraps());
            state.set_wraps(true, cx);
            state.replace_text_in_range(None, "one\ntwo", window, cx);
            assert_eq!(state.visual_row_count(), state.display().line_count());
        });
    }

    #[gpui::test]
    fn drawing_the_element_shapes_the_visible_rows_and_answers_a_click(cx: &mut TestAppContext) {
        let composer = open(InputMode::MultiLine, cx);
        edit(&composer, cx, |state, window, cx| {
            let handle = state.focus_handle.clone();
            window.focus(&handle, cx);
            state.replace_text_in_range(None, "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\n\u{43c}\u{438}\u{440}\n日本語", window, cx);
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
            assert_eq!(state.selection().head, "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\n".len());

            state.on_mouse_down(middle_of_second_row, 2, false, cx);
            assert_eq!(
                state.selection().range(),
                "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\n".len().."\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\n\u{43c}\u{438}\u{440}".len(),
                "a double click takes the word under it"
            );

            state.on_mouse_down(middle_of_second_row, 3, false, cx);
            assert_eq!(
                state.selection().range(),
                "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\n".len().."\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\n\u{43c}\u{438}\u{440}".len()
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

        for character in "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}".chars() {
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
        assert_eq!(rewrite_line_breaks("\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\n\u{43c}\u{438}\u{440}", ' '), "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{43c}\u{438}\u{440}");
    }

    #[test]
    fn a_composer_keeps_the_breaks_but_never_a_carriage_return() {
        assert_eq!(rewrite_line_breaks("one\r\ntwo", '\n'), "one\ntwo");
        assert_eq!(rewrite_line_breaks("one\rtwo", '\n'), "one\ntwo");
        assert_eq!(rewrite_line_breaks("one\ntwo", '\n'), "one\ntwo");
    }
}
