use std::any::TypeId;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, FocusHandle, Global, GlobalElementId,
    HighlightStyle, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, KeyContext,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    SharedString, StyledText, TextLayout, TextRun, Window, fill, point,
};
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;

use crate::input_ui::input_actions;
use crate::input_ui::text_boundaries::{clamp_to_char_boundary, line_range_at, word_range_at};

/// what one press selects, from the click count of the press that started it
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SelectionGranularity {
    #[default]
    Character,
    Word,
    Paragraph,
}

impl SelectionGranularity {
    /// granularity a press with this click count selects with
    pub fn for_click_count(click_count: usize) -> Self {
        match click_count {
            0 | 1 => Self::Character,
            2 => Self::Word,
            _ => Self::Paragraph,
        }
    }

    /// unit around index this granularity selects
    pub fn unit_at(self, text: &str, index: usize) -> Range<usize> {
        let index = clamp_to_char_boundary(text, index);
        match self {
            Self::Character => index..index,
            Self::Word => word_range_at(text, index),
            Self::Paragraph => line_range_at(text, index),
        }
    }
}

/// selection of one selectable text, where it was pressed and where the pointer is now
/// byte offsets, always on char boundaries
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextSelection {
    anchor: Range<usize>,
    head: Range<usize>,
    granularity: SelectionGranularity,
    dragging: bool,
}

impl TextSelection {
    /// selected byte range, start before end whichever way the pointer moved
    pub fn range(&self) -> Range<usize> {
        let start = self.anchor.start.min(self.head.start);
        let end = self.anchor.end.max(self.head.end);
        start..end
    }

    /// nothing selected
    pub fn is_empty(&self) -> bool {
        self.range().is_empty()
    }

    /// true while the press of a drag is still held
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn granularity(&self) -> SelectionGranularity {
        self.granularity
    }

    /// left press at index, extend is shift
    /// a single shift click moves the end of the existing selection instead of starting a new one
    pub fn press(&mut self, text: &str, index: usize, click_count: usize, extend: bool) {
        let granularity = SelectionGranularity::for_click_count(click_count);
        if extend && granularity == SelectionGranularity::Character {
            self.head = self.granularity.unit_at(text, index);
        } else {
            let unit = granularity.unit_at(text, index);
            self.anchor = unit.clone();
            self.head = unit;
            self.granularity = granularity;
        }
        self.dragging = true;
    }

    /// pointer moved to index with the press held
    /// true if the selection changed so the caller redraws only then
    pub fn drag_to(&mut self, text: &str, index: usize) -> bool {
        if !self.dragging {
            return false;
        }
        let head = self.granularity.unit_at(text, index);
        if head == self.head {
            return false;
        }
        self.head = head;
        true
    }

    /// the press ended
    pub fn release(&mut self) {
        self.dragging = false;
    }

    /// right press at index, a selection holding the point stays since the menu acts on it
    /// otherwise the word there gets selected, returns whether it changed
    pub fn secondary_press(&mut self, text: &str, index: usize) -> bool {
        let index = clamp_to_char_boundary(text, index);
        let range = self.range();
        self.dragging = false;
        if !range.is_empty() && range.start <= index && index <= range.end {
            return false;
        }
        let word = word_range_at(text, index);
        let changed = word != range;
        self.anchor = word.clone();
        self.head = word;
        self.granularity = SelectionGranularity::Word;
        changed
    }

    pub fn select_all(&mut self, text: &str) {
        self.anchor = 0..0;
        self.head = text.len()..text.len();
        self.granularity = SelectionGranularity::Character;
        self.dragging = false;
    }

    /// drops the selection, true if there was one
    pub fn clear(&mut self) -> bool {
        let had = !self.is_empty();
        *self = Self::default();
        had
    }

    /// keeps the selection valid for text, a changed text loses it instead of selecting bytes that are gone
    pub fn fit(&mut self, text: &str) {
        let range = self.range();
        if range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            *self = Self::default();
        }
    }

    /// selected text, none when nothing is selected
    pub fn selected_text<'a>(&self, text: &'a str) -> Option<&'a str> {
        let range = self.range();
        if range.is_empty() {
            return None;
        }
        text.get(range)
    }
}

pub const SELECTABLE_TEXT_MENU_COPY: &str = "copy";
pub const SELECTABLE_TEXT_MENU_SELECT_ALL: &str = "select_all";

/// one item of the selectable text menu as data, the owner turns it into a context menu item
/// label key is resolved with tr there
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectableTextMenuItem {
    pub id: &'static str,
    pub label_key: &'static str,
    pub enabled: bool,
}

/// items of the right click menu, copy only with something selected
pub fn selectable_text_menu_items(has_selection: bool) -> [SelectableTextMenuItem; 2] {
    [
        SelectableTextMenuItem { id: SELECTABLE_TEXT_MENU_COPY, label_key: "copy", enabled: has_selection },
        SelectableTextMenuItem {
            id: SELECTABLE_TEXT_MENU_SELECT_ALL,
            label_key: "desktop_menu_select_all",
            enabled: true,
        },
    ]
}

/// where the last press on text inside a press surface landed
struct SelectableTextPressMark(Point<Pixels>);

impl Global for SelectableTextPressMark {}

/// records that the press went to selectable text, for the views around it
/// called by the text own listener which runs before its ancestors
pub fn mark_selectable_text_press(position: Point<Pixels>, cx: &mut App) {
    cx.set_global(SelectableTextPressMark(position));
}

/// true if the press went to selectable text, a drag from there selects
/// so an enclosing drag gesture (a mouse swipe of the row) must leave it alone
pub fn selectable_text_took_press(position: Point<Pixels>, cx: &App) -> bool {
    cx.try_global::<SelectableTextPressMark>().is_some_and(|mark| mark.0 == position)
}

type ContextMenuHandler = Rc<dyn Fn(&SelectableTextMenuRequest, &mut Window, &mut App)>;
type PressPredicate = Rc<dyn Fn(&mut App) -> bool>;

/// what a right click on selectable text hands its owner
/// where to anchor the menu and how to run the chosen item
/// selected text is taken at the click because picking a menu row clears the selection
#[derive(Clone)]
pub struct SelectableTextMenuRequest {
    position: Point<Pixels>,
    selected: Option<SharedString>,
    selection: Rc<RefCell<TextSelection>>,
    text: SharedString,
}

impl SelectableTextMenuRequest {
    fn new(position: Point<Pixels>, selection: Rc<RefCell<TextSelection>>, text: SharedString) -> Self {
        let selected = selection.borrow().selected_text(&text).map(|selected| SharedString::from(selected.to_owned()));
        Self { position, selected, selection, text }
    }

    /// pointer position of the click, the menu anchor
    pub fn position(&self) -> Point<Pixels> {
        self.position
    }

    /// text that was selected at the click, if any
    pub fn selected_text(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    /// items to show
    pub fn items(&self) -> [SelectableTextMenuItem; 2] {
        selectable_text_menu_items(self.selected.is_some())
    }

    /// runs the item with this id, false for an unknown id or a copy with nothing selected
    pub fn perform(&self, id: &str, window: &mut Window, cx: &mut App) -> bool {
        match id {
            SELECTABLE_TEXT_MENU_COPY => match &self.selected {
                Some(selected) => {
                    UIPasteboard::general().set_string(selected, cx);
                    true
                }
                None => false,
            },
            SELECTABLE_TEXT_MENU_SELECT_ALL => {
                self.selection.borrow_mut().select_all(&self.text);
                window.refresh();
                true
            }
            _ => false,
        }
    }
}

fn ordered(a: Pixels, b: Pixels) -> (Pixels, Pixels) {
    if a <= b { (a, b) } else { (b, a) }
}

/// highlight rects behind a selection from start to end
/// given as top left points of the end rows one row is one rect
/// more rows give the first row tail, full middle rows and the last row head
pub fn selection_rects(
    start: Point<Pixels>,
    end: Point<Pixels>,
    line_height: Pixels,
    bounds: Bounds<Pixels>,
) -> Vec<Bounds<Pixels>> {
    if end.y - start.y < line_height * 0.5 {
        let (left, right) = ordered(start.x, end.x);
        if left == right {
            return Vec::new();
        }
        return vec![Bounds::from_corners(point(left, start.y), point(right, start.y + line_height))];
    }
    let first_bottom = start.y + line_height;
    let mut rects = vec![Bounds::from_corners(
        point(start.x, start.y),
        point(bounds.right(), first_bottom),
    )];
    if end.y > first_bottom {
        rects.push(Bounds::from_corners(point(bounds.left(), first_bottom), point(bounds.right(), end.y)));
    }
    if end.x > bounds.left() {
        rects.push(Bounds::from_corners(point(bounds.left(), end.y), point(end.x, end.y + line_height)));
    }
    rects
}

fn index_at(layout: &TextLayout, position: Point<Pixels>) -> usize {
    match layout.index_for_position(position) {
        Ok(index) | Err(index) => index,
    }
}

/// selectable text with an id that keeps the selection across frames
/// id comes from the domain (a message id), not a list position
pub fn selectable_text(id: impl Into<ElementId>, text: impl Into<SharedString>) -> SelectableText {
    SelectableText::new(id, text)
}

/// element behind selectable_text, font and color and wrapping come from the enclosing text style
pub struct SelectableText {
    id: ElementId,
    text: SharedString,
    styled: StyledText,
    selection_color: Option<Hsla>,
    on_context_menu: Option<ContextMenuHandler>,
    hosted_in_press_surface: bool,
    press_gate: Option<PressPredicate>,
    single_click_when: Option<PressPredicate>,
}

impl SelectableText {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        Self {
            id: id.into(),
            styled: StyledText::new(text.clone()),
            text,
            selection_color: None,
            on_context_menu: None,
            hosted_in_press_surface: false,
            press_gate: None,
            single_click_when: None,
        }
    }

    /// styled ranges over the enclosing text style
    pub fn with_highlights(mut self, highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>) -> Self {
        self.styled = self.styled.with_highlights(highlights);
        self
    }

    pub fn with_runs(mut self, runs: Vec<TextRun>) -> Self {
        self.styled = self.styled.with_runs(runs);
        self
    }

    /// runs drawn in another font face
    pub fn with_font_family_overrides(
        mut self,
        overrides: impl IntoIterator<Item = (Range<usize>, SharedString)>,
    ) -> Self {
        self.styled = self.styled.with_font_family_overrides(overrides);
        self
    }

    /// text sits on a surface that owns the press, eg a chat bubble
    /// press takes focus only once something is selected so a plain click leaves the composer focused
    pub fn hosted_in_press_surface(mut self) -> Self {
        self.hosted_in_press_surface = true;
        self
    }

    /// while true the surface has taken the press over (a hold menu is up)
    /// a new press selects nothing and a running drag drops its selection
    pub fn press_gate(mut self, gate: impl Fn(&mut App) -> bool + 'static) -> Self {
        self.press_gate = Some(Rc::new(gate));
        self
    }

    /// while true the second click belongs to the owner (a double tap), so it doesnt select a word
    pub fn single_click_when(mut self, single_click: impl Fn(&mut App) -> bool + 'static) -> Self {
        self.single_click_when = Some(Rc::new(single_click));
        self
    }

    /// highlight color, defaults to the theme input selection color so labels and fields match
    pub fn selection_color(mut self, color: Hsla) -> Self {
        self.selection_color = Some(color);
        self
    }

    /// right click handler, the owner opens the context menu at the request position
    /// then calls perform with the id, without it a right click does nothing
    pub fn on_context_menu(
        mut self,
        handler: impl Fn(&SelectableTextMenuRequest, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_context_menu = Some(Rc::new(handler));
        self
    }
}

/// lives in element state between frames
#[doc(hidden)]
#[derive(Clone)]
pub struct SelectableTextState {
    selection: Rc<RefCell<TextSelection>>,
    focus: FocusHandle,
}

#[doc(hidden)]
pub struct SelectableTextPrepaint {
    hitbox: Hitbox,
    state: SelectableTextState,
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = SelectableTextPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.styled.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled.prepaint(None, inspector_id, bounds, request_layout, window, cx);
        input_actions::install_key_bindings(cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let state = match global_id {
            Some(global_id) => window.with_element_state::<SelectableTextState, _>(global_id, |state, _| {
                let state = state.unwrap_or_else(|| SelectableTextState {
                    selection: Rc::default(),
                    focus: cx.focus_handle(),
                });
                (state.clone(), state)
            }),
            None => SelectableTextState { selection: Rc::default(), focus: cx.focus_handle() },
        };
        window.set_focus_handle(&state.focus, cx);
        SelectableTextPrepaint { hitbox, state }
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled.layout().clone();
        let SelectableTextPrepaint { hitbox, state } = prepaint;
        state.selection.borrow_mut().fit(&self.text);

        let range = state.selection.borrow().range();
        if !range.is_empty()
            && let (Some(start), Some(end)) = (layout.position_for_index(range.start), layout.position_for_index(range.end))
        {
            let color = self
                .selection_color
                .or_else(|| cx.try_global::<AppTheme>().map(|theme| theme.input.selection));
            if let Some(color) = color {
                for rect in selection_rects(start, end, layout.line_height(), bounds) {
                    window.paint_quad(fill(rect, color));
                }
            }
        }
        self.styled.paint(None, inspector_id, bounds, request_layout, &mut (), window, cx);
        window.set_cursor_style(CursorStyle::IBeam, hitbox);
        let hosted = self.hosted_in_press_surface;
        let view = window.current_view();

        window.on_mouse_event({
            let selection = state.selection.clone();
            let focus = state.focus.clone();
            let hitbox = hitbox.clone();
            let layout = layout.clone();
            let text = self.text.clone();
            let on_context_menu = self.on_context_menu.clone();
            let press_gate = self.press_gate.clone();
            let single_click_when = self.single_click_when.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if !hitbox.is_hovered(window) {
                    if selection.borrow_mut().clear() {
                        cx.notify(view);
                    }
                    return;
                }
                match event.button {
                    MouseButton::Left => {
                        if press_gate.as_ref().is_some_and(|gate| gate(cx)) {
                            return;
                        }
                        if hosted {
                            mark_selectable_text_press(event.position, cx);
                        }
                        let click_count =
                            if single_click_when.as_ref().is_some_and(|single| single(cx)) { 1 } else { event.click_count };
                        let index = index_at(&layout, event.position);
                        selection.borrow_mut().press(&text, index, click_count, event.modifiers.shift);
                        if !hosted || !selection.borrow().is_empty() {
                            window.focus(&focus, cx);
                        }
                        cx.notify(view);
                    }
                    MouseButton::Right if !hosted => {
                        let index = index_at(&layout, event.position);
                        if selection.borrow_mut().secondary_press(&text, index) {
                            cx.notify(view);
                        }
                        window.focus(&focus, cx);
                        if let Some(handler) = &on_context_menu {
                            let request = SelectableTextMenuRequest::new(event.position, selection.clone(), text.clone());
                            handler(&request, window, cx);
                        }
                    }
                    _ => {}
                }
            }
        });

        window.on_mouse_event({
            let selection = state.selection.clone();
            let layout = layout.clone();
            let text = self.text.clone();
            let focus = state.focus.clone();
            let press_gate = self.press_gate.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !selection.borrow().is_dragging() {
                    return;
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    selection.borrow_mut().release();
                    return;
                }
                if press_gate.as_ref().is_some_and(|gate| gate(cx)) {
                    selection.borrow_mut().release();
                    selection.borrow_mut().clear();
                    cx.notify(view);
                    return;
                }
                let index = index_at(&layout, event.position);
                if selection.borrow_mut().drag_to(&text, index) {
                    if hosted && !selection.borrow().is_empty() {
                        window.focus(&focus, cx);
                    }
                    cx.notify(view);
                }
            }
        });

        window.on_mouse_event({
            let selection = state.selection.clone();
            move |event: &MouseUpEvent, phase, _window, _cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                    selection.borrow_mut().release();
                }
            }
        });

        if let Ok(context) = KeyContext::parse(input_actions::KEY_CONTEXT) {
            window.set_key_context(context);
        }

        window.on_action(TypeId::of::<input_actions::Copy>(), {
            let selection = state.selection.clone();
            let text = self.text.clone();
            move |_, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                match selection.borrow().selected_text(&text) {
                    Some(selected) => UIPasteboard::general().set_string(selected, cx),
                    None => cx.propagate(),
                }
            }
        });

        window.on_action(TypeId::of::<input_actions::SelectAll>(), {
            let selection = state.selection.clone();
            let text = self.text.clone();
            move |_, phase, _window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if hosted && selection.borrow().is_empty() {
                    cx.propagate();
                    return;
                }
                selection.borrow_mut().select_all(&text);
                cx.notify(view);
            }
        });
    }
}
