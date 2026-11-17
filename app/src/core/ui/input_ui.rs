use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, Focusable, FontWeight, MouseButton, SharedString, Subscription, WeakEntity, Window,
};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::NSRange;
use rise_i18n::tr;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState, Selection};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::canon_field_rule::{RiseFieldRule, RiseFocusedTextInput, RiseTextInputCaret};
use crate::core::ui::input_field_accessory::InputFieldAccessory;

/// format callback gets typed and previous text and returns the new text
pub type InputUiFormat = Rc<dyn Fn(&str, &str) -> String>;
/// set value callback gets name and value
pub type InputUiSetValue = Rc<dyn Fn(&str, &str, &mut Window, &mut App)>;
pub type InputUiSubmit = Rc<dyn Fn(&mut Window, &mut App)>;
/// focus change callback gets the new focus state
pub type InputUiFocusChange = Rc<dyn Fn(bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct InputUi {
    id: ElementId,
    values: HashMap<String, String>,
    errors: HashMap<String, String>,
    error_message: Option<String>,
    name: String,
    placeholder: String,
    trailing_text: Option<String>,
    is_secure: bool,
    height: Option<f32>,
    auto_focus: bool,
    focus_request: Option<i64>,
    shows_secure_toggle: bool,
    rule: RiseFieldRule,
    format: Option<InputUiFormat>,
    focus_token: i64,
    on_focus_change: Option<InputUiFocusChange>,
    on_submit: InputUiSubmit,
    on_cancel: Option<InputUiSubmit>,
    set_value: Option<InputUiSetValue>,
}

impl InputUi {
    pub const HORIZONTAL_PADDING: f32 = 18.;

    const SECURE_TOGGLE_WIDTH: f32 = 44.;

    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            values: HashMap::new(),
            errors: HashMap::new(),
            error_message: None,
            name: String::new(),
            placeholder: String::new(),
            trailing_text: None,
            is_secure: false,
            height: None,
            auto_focus: false,
            focus_request: None,
            shows_secure_toggle: false,
            rule: RiseFieldRule::NONE,
            format: None,
            focus_token: 0,
            on_focus_change: None,
            on_submit: Rc::new(|_, _| {}),
            on_cancel: None,
            set_value: None,
        }
    }

    pub fn values(mut self, values: HashMap<String, String>) -> Self {
        self.values = values;
        self
    }

    pub fn errors(mut self, errors: HashMap<String, String>) -> Self {
        self.errors = errors;
        self
    }

    pub fn error_message(mut self, error_message: Option<String>) -> Self {
        self.error_message = error_message;
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn trailing_text(mut self, trailing_text: Option<String>) -> Self {
        self.trailing_text = trailing_text;
        self
    }

    pub fn is_secure(mut self, is_secure: bool) -> Self {
        self.is_secure = is_secure;
        self
    }

    pub fn height(mut self, height: Option<f32>) -> Self {
        self.height = height;
        self
    }

    pub fn auto_focus(mut self, auto_focus: bool) -> Self {
        self.auto_focus = auto_focus;
        self
    }

    pub fn shows_secure_toggle(mut self, shows_secure_toggle: bool) -> Self {
        self.shows_secure_toggle = shows_secure_toggle;
        self
    }

    pub fn rule(mut self, rule: RiseFieldRule) -> Self {
        self.rule = rule;
        self
    }

    pub fn format(mut self, format: impl Fn(&str, &str) -> String + 'static) -> Self {
        self.format = Some(Rc::new(format));
        self
    }

    pub fn focus_token(mut self, focus_token: i64) -> Self {
        self.focus_token = focus_token;
        self
    }

    pub fn on_focus_change(mut self, on_focus_change: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_focus_change = Some(Rc::new(on_focus_change));
        self
    }

    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Rc::new(on_submit);
        self
    }

    /// desktop only, escape in the field after a composition has been taken back
    /// the field keeps the action to itself so a screen that steps back on escape has to ask here
    pub fn on_cancel(mut self, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }

    /// desktop only, each new value focuses the field
    /// a field that stayed on screen under a sheet doesnt appear again so auto_focus cant bring focus back
    /// none asks nothing
    pub fn focus_request(mut self, focus_request: Option<i64>) -> Self {
        self.focus_request = focus_request;
        self
    }

    pub fn set_value(mut self, set_value: impl Fn(&str, &str, &mut Window, &mut App) + 'static) -> Self {
        self.set_value = Some(Rc::new(set_value));
        self
    }

    fn resolved_error_text(&self) -> String {
        if let Some(error_message) = self.error_message.as_ref().filter(|message| !message.is_empty()) {
            return error_message.clone();
        }
        self.errors.get(&format!("{}Err", self.name)).cloned().unwrap_or_default()
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

/// what the field reads from its caller when an edit lands, refreshed every render
#[derive(Clone)]
struct InputUiProps {
    name: String,
    rule: RiseFieldRule,
    format: Option<InputUiFormat>,
    set_value: Option<InputUiSetValue>,
    on_focus_change: Option<InputUiFocusChange>,
    on_submit: InputUiSubmit,
    on_cancel: Option<InputUiSubmit>,
}

impl InputUiProps {
    /// value as the store will accept it, caller format first and then the canon rule
    fn shaped(&self, typed: &str, previous: &str) -> String {
        match &self.format {
            Some(format) => self.rule.clamped(&format(typed, previous)),
            None => self.rule.clamped(typed),
        }
    }
}

struct InputUiSession {
    field: Entity<InputUiState>,
    published: String,
    reveals_secure_text: bool,
    caret: Rc<dyn RiseTextInputCaret>,
    props: InputUiProps,
    _subscriptions: Vec<Subscription>,
}

impl InputUiSession {
    fn new(seed: String, props: InputUiProps, auto_focus: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            let mut field = InputUiState::new(InputMode::SingleLine, cx);
            field.set_chromeless(true, cx);
            field.reset(seed.clone(), cx);
            field
        });
        let handle = field.read(cx).focus_handle(cx);
        let caret: Rc<dyn RiseTextInputCaret> = Rc::new(InputUiCaret::new(&field));

        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, _, event: &InputUiEvent, window, cx| match event {
                InputUiEvent::Changed => this.text_changed(window, cx),
                InputUiEvent::Submitted => {
                    let on_submit = this.props.on_submit.clone();
                    on_submit(window, cx);
                }
                InputUiEvent::Cancelled => match this.props.on_cancel.clone() {
                    Some(on_cancel) => on_cancel(window, cx),
                    None => ScreenFocus::release_text_field(window, cx),
                },
            }),
            cx.on_focus(&handle, window, |this, window, cx| {
                RiseFocusedTextInput::report(this.caret.clone(), cx);
                if let Some(on_focus_change) = this.props.on_focus_change.clone() {
                    on_focus_change(true, window, cx);
                }
            }),
            cx.on_blur(&handle, window, |this, window, cx| {
                RiseFocusedTextInput::resign(&this.caret, cx);
                if let Some(on_focus_change) = this.props.on_focus_change.clone() {
                    on_focus_change(false, window, cx);
                }
                this.settle(cx);
            }),
        ];

        if auto_focus {
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }

        Self { field, published: seed, reveals_secure_text: false, caret, props, _subscriptions: subscriptions }
    }

    fn text_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.field.read(cx).text().to_owned();
        if typed == self.published {
            return;
        }
        let shaped_text = self.props.shaped(&typed, &self.published);
        self.published = shaped_text.clone();
        if let Some(set_value) = self.props.set_value.clone() {
            let name = self.props.name.clone();
            let value = shaped_text.clone();
            set_value(&name, &value, window, cx);
        }

        if shaped_text != typed && !differs_only_at_edges(&typed, &shaped_text) {
            self.write_field(&shaped_text, cx);
        }
        cx.notify();
    }

    fn settle(&mut self, cx: &mut Context<Self>) {
        let text = self.field.read(cx).text().to_owned();
        let settled = self.props.shaped(&text, &self.published);
        if settled == text {
            return;
        }
        self.published = settled.clone();
        self.write_field(&settled, cx);
        cx.notify();
    }

    fn take_external(&mut self, external: &str, cx: &mut Context<Self>) {
        if external == self.published {
            return;
        }
        self.published = external.to_owned();
        let external = external.to_owned();
        self.field.update(cx, |field, cx| field.reset(external, cx));
        cx.notify();
    }

    /// assigns text and puts the caret at the end
    fn write_field(&self, text: &str, cx: &mut Context<Self>) {
        self.field.update(cx, |field, cx| {
            field.set_text(text, cx);
            let end = field.text().len();
            field.set_selection(Selection::cursor(end), cx);
        });
    }
}

fn differs_only_at_edges(typed: &str, shaped_text: &str) -> bool {
    trimming_whitespaces_and_newlines(typed) == trimming_whitespaces_and_newlines(shaped_text)
}

impl RenderOnce for InputUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let incoming = self.values.get(&self.name).cloned().unwrap_or_default();
        let error_text = self.resolved_error_text();
        let props = InputUiProps {
            name: self.name.clone(),
            rule: self.rule,
            format: self.format.clone(),
            set_value: self.set_value.clone(),
            on_focus_change: self.on_focus_change.clone(),
            on_submit: self.on_submit.clone(),
            on_cancel: self.on_cancel.clone(),
        };

        let seed = incoming.clone();
        let auto_focus = self.auto_focus;
        let init_props = props.clone();
        let session = window.use_keyed_state(self.child_id("input-ui"), cx, move |window, cx| {
            InputUiSession::new(seed, init_props, auto_focus, window, cx)
        });
        let escape_to_layer = props.on_cancel.is_none();
        session.update(cx, |session, cx| {
            session.field.update(cx, |field, _| field.set_escape_to_layer(escape_to_layer));
            session.props = props;
        });

        let tracked = session.clone();
        on_change(self.child_id("incoming"), incoming, false, window, cx, move |_, external, _, cx| {
            tracked.update(cx, |session, cx| session.take_external(external, cx));
        });

        let (field, published, reveals_secure_text) = {
            let session = session.read(cx);
            (session.field.clone(), session.published.clone(), session.reveals_secure_text)
        };
        let secure = self.is_secure && !reveals_secure_text;
        let placeholder = self.placeholder.clone();
        field.update(cx, |field, cx| {
            if field.is_secure() != secure {
                field.set_secure(secure, cx);
            }
            if field.placeholder().as_ref() != placeholder.as_str() {
                field.set_placeholder(placeholder, cx);
            }
        });
        let handle = field.read(cx).focus_handle(cx);
        if let Some(focus_request) = self.focus_request {
            let requested = handle.clone();
            on_change(self.child_id("focus-request"), focus_request, false, window, cx, move |_, _, window, cx| {
                window.focus(&requested, cx);
            });
        }
        let token_focus = handle.clone();
        on_change(self.child_id("focus-token"), self.focus_token, false, window, cx, move |_, _, window, cx| {
            window.focus(&token_focus, cx);
        });

        let resolved_height = self.height.map(px).unwrap_or(theme.input.height_300);
        let remaining = self.rule.remaining(&published);
        let shows_accessory =
            self.trailing_text.is_none() && InputFieldAccessory::is_visible(remaining, Some(&error_text));
        let trailing_text_width =
            if self.trailing_text.is_none() && !shows_accessory { 0. } else { InputFieldAccessory::WIDTH };
        let secure_toggle_width =
            if self.is_secure && self.shows_secure_toggle { Self::SECURE_TOGGLE_WIDTH } else { 0. };

        let focus_on_frame = handle.clone();
        let text_slot = div()
            .padding_edges(EdgeSet::LEADING, Self::HORIZONTAL_PADDING)
            .padding_edges(
                EdgeSet::TRAILING,
                Self::HORIZONTAL_PADDING + trailing_text_width + secure_toggle_width,
            )
            .w_full()
            .h(resolved_height)
            .flex()
            .items_center()
            .text_color(theme.text.primary)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| window.focus(&focus_on_frame, cx))
            .child(field);

        let mut trailing = hstack(0.).padding_edges(EdgeSet::TRAILING, 2.);
        if let Some(trailing_text) = self.trailing_text.clone() {
            trailing = trailing.child(
                div()
                    .w(px(trailing_text_width))
                    .h(resolved_height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .swiftui_font(Font::system(12., FontWeight::MEDIUM, FontDesign::Monospaced))
                    .text_color(theme.text.secondary)
                    .child(trailing_text),
            );
        } else if shows_accessory {
            trailing = trailing.child(
                div().h(resolved_height).flex().items_center().child(InputFieldAccessory::new(
                    self.child_id("accessory"),
                    remaining,
                    Some(error_text.clone()),
                )),
            );
        }
        if self.is_secure && self.shows_secure_toggle {
            let toggled = session.clone();
            let focus_after_toggle = handle;
            let symbol = if reveals_secure_text { "eye.slash" } else { "eye" };
            let label = if reveals_secure_text { tr("auth_hide_password") } else { tr("auth_show_password") };
            let tooltip_id = if reveals_secure_text { "secure-toggle-tooltip-hide" } else { "secure-toggle-tooltip-show" };
            trailing = trailing.child(
                div().id(self.child_id(tooltip_id)).tooltip_label(label).child(
                    plain(self.child_id("secure-toggle"))
                        .on_press(move |window, cx| {
                            toggled.update(cx, |session, cx| {
                                session.reveals_secure_text = !session.reveals_secure_text;
                                cx.notify();
                            });
                            window.focus(&focus_after_toggle, cx);
                        })
                        .w(px(secure_toggle_width))
                        .h(resolved_height)
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized(symbol, px(15.), theme.text.secondary, false)),
                ),
            );
        }

        div()
            .relative()
            .w_full()
            .bg(theme.bg._200)
            .rounded_full()
            .overflow_hidden()
            .child(zstack(Alignment::Trailing).child(text_slot).child(trailing))
            .when(!error_text.is_empty(), |field| {
                field.child(centred_stroke(theme.semantic.error_100, 1., Shape::Capsule))
            })
    }
}

/// kit field behind a focused input
pub struct InputUiCaret {
    field: WeakEntity<InputUiState>,
}

impl InputUiCaret {
    pub fn new(field: &Entity<InputUiState>) -> Self {
        Self { field: field.downgrade() }
    }
}

impl RiseTextInputCaret for InputUiCaret {
    fn text(&self, cx: &App) -> String {
        self.field.upgrade().map(|field| field.read(cx).text().to_owned()).unwrap_or_default()
    }

    fn marked_range(&self, cx: &App) -> Option<Range<usize>> {
        self.field.upgrade()?.read(cx).marked_range()
    }

    fn selected_range(&self, cx: &App) -> NSRange {
        self.field.upgrade().map(|field| field.read(cx).selected_range()).unwrap_or_default()
    }

    fn replace(&self, text: &str, _window: &mut Window, cx: &mut App) {
        let Some(field) = self.field.upgrade() else {
            return;
        };
        field.update(cx, |field, cx| {
            field.set_text(text, cx);
            let end = field.text().len();
            field.set_selection(Selection::cursor(end), cx);
        });
    }

    fn replace_keeping_caret(&self, text: &str, _window: &mut Window, cx: &mut App) {
        let Some(field) = self.field.upgrade() else {
            return;
        };
        field.update(cx, |field, cx| {
            let from_end = field.text().len().saturating_sub(field.selection().head);
            field.set_text(text, cx);
            let caret = field.text().len().saturating_sub(from_end);
            field.set_selection(Selection::cursor(caret), cx);
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOutlinedRowStyle {
    Outlined,
    Filled,
}

#[derive(IntoElement)]
pub struct InputOutlinedRowUi {
    id: ElementId,
    text: Binding<String>,
    placeholder: String,
    field_height: Option<f32>,
    style: InputOutlinedRowStyle,
    leading: AnyElement,
    trailing: Option<AnyElement>,
    auto_focus: bool,
    on_submit: Option<InputUiSubmit>,
    on_cancel: Option<InputUiSubmit>,
}

impl InputOutlinedRowUi {
    pub fn new(
        id: impl Into<ElementId>,
        text: Binding<String>,
        placeholder: impl Into<String>,
        leading: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            text,
            placeholder: placeholder.into(),
            field_height: None,
            style: InputOutlinedRowStyle::Outlined,
            leading: leading.into_any_element(),
            trailing: None,
            auto_focus: false,
            on_submit: None,
            on_cancel: None,
        }
    }

    /// desktop only, a picker search takes the keyboard as soon as its sheet opens
    pub fn auto_focus(mut self, auto_focus: bool) -> Self {
        self.auto_focus = auto_focus;
        self
    }

    /// enter in the field
    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }

    /// escape in the field after a composition has been taken back
    /// the field keeps the action so the sheet around it never sees that escape
    /// without a handler escape takes focus off the field and the next one reaches the sheet
    pub fn on_cancel(mut self, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }

    pub fn field_height(mut self, field_height: Option<f32>) -> Self {
        self.field_height = field_height;
        self
    }

    pub fn style(mut self, style: InputOutlinedRowStyle) -> Self {
        self.style = style;
        self
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }
}

struct InputOutlinedRowSession {
    field: Entity<InputUiState>,
    text: Binding<String>,
    on_submit: Option<InputUiSubmit>,
    on_cancel: Option<InputUiSubmit>,
    caret: Rc<dyn RiseTextInputCaret>,
    _subscriptions: Vec<Subscription>,
}

impl InputOutlinedRowSession {
    fn new(text: Binding<String>, auto_focus: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let seed = text.get(cx);
        let field = cx.new(|cx| {
            let mut field = InputUiState::new(InputMode::SingleLine, cx);
            field.set_chromeless(true, cx);
            field.reset(seed, cx);
            field
        });
        let handle = field.read(cx).focus_handle(cx);
        let caret: Rc<dyn RiseTextInputCaret> = Rc::new(InputUiCaret::new(&field));
        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, field, event: &InputUiEvent, window, cx| {
                match event {
                    InputUiEvent::Changed => {
                        let typed = field.read(cx).text().to_owned();
                        if this.text.get(cx) != typed {
                            this.text.set(typed, window, cx);
                        }
                    }
                    InputUiEvent::Submitted => {
                        if let Some(on_submit) = this.on_submit.clone() {
                            on_submit(window, cx);
                        }
                    }
                    InputUiEvent::Cancelled => {
                        match this.on_cancel.clone() {
                            Some(on_cancel) => on_cancel(window, cx),
                            None => ScreenFocus::release_text_field(window, cx),
                        }
                    }
                }
            }),
            cx.on_focus(&handle, window, |this, _, cx| {
                RiseFocusedTextInput::report(this.caret.clone(), cx);
            }),
            cx.on_blur(&handle, window, |this, _, cx| {
                RiseFocusedTextInput::resign(&this.caret, cx);
            }),
        ];
        if auto_focus {
            let focus = handle.clone();
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        Self { field, text, on_submit: None, on_cancel: None, caret, _subscriptions: subscriptions }
    }
}

impl RenderOnce for InputOutlinedRowUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let h = self.field_height.map(px).unwrap_or(theme.input.height_300);

        let binding = self.text.clone();
        let auto_focus = self.auto_focus;
        let session = window.use_keyed_state(self.child_id("outlined-row"), cx, move |window, cx| {
            InputOutlinedRowSession::new(binding, auto_focus, window, cx)
        });
        let text = self.text.clone();
        let on_submit = self.on_submit.clone();
        let on_cancel = self.on_cancel.clone();
        let escape_to_layer = on_cancel.is_none();
        session.update(cx, |session, cx| {
            session.field.update(cx, |field, _| field.set_escape_to_layer(escape_to_layer));
            session.text = text;
            session.on_submit = on_submit;
            session.on_cancel = on_cancel;
        });

        let field = session.read(cx).field.clone();
        let current = self.text.get(cx);
        on_change(self.child_id("text"), current, false, window, cx, move |_, external, _, cx| {
            field.update(cx, |field, cx| {
                if field.text() != external.as_str() {
                    field.reset(external.clone(), cx);
                }
            });
        });

        let field = session.read(cx).field.clone();
        let placeholder = self.placeholder.clone();
        field.update(cx, |field, cx| {
            if field.placeholder().as_ref() != placeholder.as_str() {
                field.set_placeholder(placeholder, cx);
            }
        });
        let handle = field.read(cx).focus_handle(cx);

        let background = match self.style {
            InputOutlinedRowStyle::Filled => theme.bg._200,
            InputOutlinedRowStyle::Outlined => theme.input.bg_100,
        };

        let row = hstack(8.)
            .relative()
            .padding_edges(EdgeSet::HORIZONTAL, 14.)
            .h(h)
            .bg(background)
            .rounded_full()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| window.focus(&handle, cx))
            .child(self.leading)
            .child(div().flex_1().h_full().flex().items_center().text_color(theme.text.primary).child(field))
            .children(self.trailing);

        row.when(self.style == InputOutlinedRowStyle::Outlined, |row| {
            row.child(centred_stroke(theme.input.border_300, 1., Shape::Capsule))
        })
    }
}
