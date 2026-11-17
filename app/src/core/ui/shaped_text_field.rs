use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Axis, Context, ElementId, Entity, Focusable, FontWeight, Hsla, SharedString, StyleRefinement, Subscription, TextAlign, Window};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState, Selection};
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_field_rule::{
    RiseFocusedTextInput, RiseFocusedTextInputOutcome, RiseTextInputCaret,
};
use crate::core::ui::input_ui::InputUiCaret;

/// text field that shows what the store kept, not what was typed
/// a setter that leaves the value unchanged wouldnt update the field
/// so it keeps its own draft and writes the shaped key into the field
#[derive(IntoElement)]
pub struct ShapedTextField {
    id: ElementId,
    text: Binding<String>,
    placeholder: String,
    prompt: Option<String>,
    axis: Axis,
    max_visible_lines: Option<usize>,
    font_size: Option<f32>,
    text_align: Option<TextAlign>,
    placeholder_color: Option<Hsla>,
    on_submit: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_cancel: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_focus_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
    auto_focus: bool,
    font_weight: Option<FontWeight>,
    text_color: Option<Hsla>,
    is_secure: bool,
}

impl ShapedTextField {
    pub fn new(id: impl Into<ElementId>, placeholder: impl Into<String>, text: Binding<String>) -> Self {
        Self {
            id: id.into(),
            text,
            placeholder: placeholder.into(),
            prompt: None,
            axis: Axis::Horizontal,
            max_visible_lines: None,
            font_size: None,
            text_align: None,
            placeholder_color: None,
            on_submit: None,
            on_cancel: None,
            on_focus_change: None,
            auto_focus: false,
            font_weight: None,
            text_color: None,
            is_secure: false,
        }
    }

    pub fn prompt(mut self, prompt: Option<String>) -> Self {
        self.prompt = prompt;
        self
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// max lines before the vertical field starts scrolling
    pub fn max_visible_lines(mut self, max_visible_lines: usize) -> Self {
        self.max_visible_lines = Some(max_visible_lines);
        self
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = Some(font_size);
        self
    }

    /// text alignment inside the field
    pub fn text_align(mut self, text_align: TextAlign) -> Self {
        self.text_align = Some(text_align);
        self
    }

    pub fn placeholder_color(mut self, color: Hsla) -> Self {
        self.placeholder_color = Some(color);
        self
    }

    /// enter in a single line field
    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }

    /// escape with no composition open
    /// the field takes the key before the sheet, so a field in a sheet passes the sheet close here
    /// without one escape just unfocuses the field
    pub fn on_cancel(mut self, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.font_weight = Some(font_weight);
        self
    }

    pub fn text_color(mut self, color: Hsla) -> Self {
        self.text_color = Some(color);
        self
    }

    /// focus on appear so the field takes focus in its first frame
    /// secure masks the text, flipping it keeps text and caret
    pub fn is_secure(mut self, is_secure: bool) -> Self {
        self.is_secure = is_secure;
        self
    }

    pub fn auto_focus(mut self, auto_focus: bool) -> Self {
        self.auto_focus = auto_focus;
        self
    }

    /// reports focus changes, the focus handle stays inside the session
    pub fn on_focus_change(mut self, on_focus_change: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_focus_change = Some(Rc::new(on_focus_change));
        self
    }
}

struct ShapedTextFieldSession {
    field: Entity<InputUiState>,
    text: Binding<String>,
    handed_over: String,
    caret: Rc<dyn RiseTextInputCaret>,
    on_submit: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_cancel: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_focus_change: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
    _subscriptions: Vec<Subscription>,
}

impl ShapedTextFieldSession {
    fn new(
        text: Binding<String>,
        mode: InputMode,
        auto_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let seed = text.get(cx);
        let field = cx.new(|cx| {
            let mut field = InputUiState::new(mode, cx);
            field.set_chromeless(true, cx);
            field.reset(seed.clone(), cx);
            field
        });
        let handle = field.read(cx).focus_handle(cx);
        let caret: Rc<dyn RiseTextInputCaret> = Rc::new(InputUiCaret::new(&field));
        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, field, event: &InputUiEvent, window, cx| {
                if *event == InputUiEvent::Changed {
                    let typed = field.read(cx).text().to_owned();
                    this.hand_over(typed, window, cx);
                }
                if *event == InputUiEvent::Cancelled {
                    match this.on_cancel.clone() {
                        Some(on_cancel) => on_cancel(window, cx),
                        None => ScreenFocus::release_text_field(window, cx),
                    }
                }
                if *event == InputUiEvent::Submitted
                    && let Some(on_submit) = this.on_submit.clone()
                {
                    on_submit(window, cx);
                }
            }),
            cx.on_focus(&handle, window, |this, window, cx| {
                RiseFocusedTextInput::report(this.caret.clone(), cx);
                if let Some(on_focus_change) = this.on_focus_change.clone() {
                    on_focus_change(true, window, cx);
                }
            }),
            cx.on_blur(&handle, window, |this, window, cx| {
                RiseFocusedTextInput::resign(&this.caret, cx);
                if let Some(on_focus_change) = this.on_focus_change.clone() {
                    on_focus_change(false, window, cx);
                }
            }),
        ];
        if auto_focus {
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
        Self { field, text, handed_over: seed, caret, on_submit: None, on_cancel: None, on_focus_change: None, _subscriptions: subscriptions }
    }

    fn hand_over(&mut self, typed: String, window: &mut Window, cx: &mut Context<Self>) {
        if typed == self.text.get(cx) {
            return;
        }
        self.text.set(typed.clone(), window, cx);
        let kept = self.text.get(cx);
        self.handed_over = kept.clone();
        if kept == typed {
            return;
        }
        match RiseFocusedTextInput::shape(&typed, &kept, window, cx) {
            RiseFocusedTextInputOutcome::Composing => {}
            RiseFocusedTextInputOutcome::Clamped => {}
            RiseFocusedTextInputOutcome::NotFocused => {
                self.field.update(cx, |field, cx| {
                    field.set_text(&kept, cx);
                    let end = field.text().len();
                    field.set_selection(Selection::cursor(end), cx);
                });
            }
        }
    }

    fn take_outside_change(&mut self, kept: &str, cx: &mut Context<Self>) {
        if kept == self.handed_over || kept == self.field.read(cx).text() {
            return;
        }
        self.handed_over = kept.to_owned();
        let kept = kept.to_owned();
        self.field.update(cx, |field, cx| field.reset(kept, cx));
    }
}

impl RenderOnce for ShapedTextField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mode = if self.axis == Axis::Vertical { InputMode::MultiLine } else { InputMode::SingleLine };
        let binding = self.text.clone();
        let auto_focus = self.auto_focus;
        let session = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("shaped-text-field")),
            cx,
            move |window, cx| ShapedTextFieldSession::new(binding, mode, auto_focus, window, cx),
        );
        let text = self.text.clone();
        let on_submit = self.on_submit.clone();
        let on_cancel = self.on_cancel.clone();
        let on_focus_change = self.on_focus_change.clone();
        let escape_to_layer = on_cancel.is_none();
        session.update(cx, |session, cx| {
            session.field.update(cx, |field, _| field.set_escape_to_layer(escape_to_layer));
            session.text = text;
            session.on_submit = on_submit;
            session.on_cancel = on_cancel;
            session.on_focus_change = on_focus_change;
        });

        let tracked = session.clone();
        on_change(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("text")),
            self.text.get(cx),
            false,
            window,
            cx,
            move |_, kept, _, cx| {
                tracked.update(cx, |session, cx| session.take_outside_change(kept, cx));
            },
        );

        let field = session.read(cx).field.clone();
        let placeholder = self.prompt.clone().unwrap_or_else(|| self.placeholder.clone());
        let max_visible_lines = self.max_visible_lines;
        let font_size = self.font_size.map(px);
        let text_align = self.text_align;
        let placeholder_color = self.placeholder_color;
        let font_weight = self.font_weight;
        let text_color = self.text_color;
        let is_secure = self.is_secure;
        field.update(cx, |field, cx| {
            if field.is_secure() != is_secure {
                field.set_secure(is_secure, cx);
            }
            if field.placeholder().as_ref() != placeholder.as_str() {
                field.set_placeholder(placeholder, cx);
            }
            if let Some(lines) = max_visible_lines
                && field.max_visible_lines() != lines
            {
                field.set_max_visible_lines(lines, cx);
            }
            if font_size.is_some() && field.font_size() != font_size {
                field.set_font_size(font_size, cx);
            }
            if let Some(text_align) = text_align
                && field.text_align() != text_align
            {
                field.set_text_align(text_align, cx);
            }
            if placeholder_color.is_some() && field.placeholder_color() != placeholder_color {
                field.set_placeholder_color(placeholder_color, cx);
            }
            if font_weight.is_some() && field.font_weight() != font_weight {
                field.set_font_weight(font_weight, cx);
            }
            if text_color.is_some() && field.text_color() != text_color {
                field.set_text_color(text_color, cx);
            }
        });
        let line_height = field.read(cx).line_height();
        let field_element = if mode == InputMode::SingleLine && line_height > px(0.) {
            field.cached(StyleRefinement::default().w_full().h(line_height)).into_any_element()
        } else {
            field.into_any_element()
        };
        div().flex_1().child(field_element)
    }
}
