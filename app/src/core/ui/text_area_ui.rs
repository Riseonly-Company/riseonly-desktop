use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, Edges, ElementId, Entity, Focusable, MouseButton, Pixels, SharedString, Subscription, Window};
use rise_core::foundation::ns_range::{byte_to_utf16_range, utf16_to_byte_range};
use rise_ui::keyboard_dismiss::{KeyboardDismissModifiers, ScreenFocus};
use rise_ui::error_text_ui::ErrorTextUi;
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState, Selection};
use rise_ui::rich_text::rich_text_input_view::RichTextInputView;
use rise_ui::swiftui::environment_values::is_enabled;
use rise_ui::swiftui::prelude::*;
use rise_ui::text_input_caret::TextInputCaret;

use crate::core::lib::canon_field_rule::{RiseFieldRule, RiseFocusedTextInput, RiseTextInputCaret};
use crate::core::ui::input_ui::{InputUiCaret, InputUiFocusChange, InputUiSetValue};

#[derive(IntoElement)]
pub struct TextAreaUi {
    id: ElementId,
    values: HashMap<String, String>,
    errors: HashMap<String, String>,
    name: String,
    placeholder: String,
    min_lines: usize,
    max_lines: usize,
    formatting: bool,
    auto_focus: bool,
    error_message: Option<String>,
    rule: RiseFieldRule,
    max_chars: Option<usize>,
    focus_token: i64,
    on_focus_change: Option<InputUiFocusChange>,
    set_value: Option<InputUiSetValue>,
}

impl TextAreaUi {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            values: HashMap::new(),
            errors: HashMap::new(),
            name: String::new(),
            placeholder: String::new(),
            min_lines: 3,
            max_lines: 8,
            formatting: false,
            auto_focus: false,
            error_message: None,
            rule: RiseFieldRule::NONE,
            max_chars: None,
            focus_token: 0,
            on_focus_change: None,
            set_value: None,
        }
    }

    pub fn focus_token(mut self, focus_token: i64) -> Self {
        self.focus_token = focus_token;
        self
    }

    pub fn on_focus_change(mut self, on_focus_change: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_focus_change = Some(Rc::new(on_focus_change));
        self
    }

    pub fn values(mut self, values: HashMap<String, String>) -> Self {
        self.values = values;
        self
    }

    pub fn errors(mut self, errors: HashMap<String, String>) -> Self {
        self.errors = errors;
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

    pub fn min_lines(mut self, min_lines: usize) -> Self {
        self.min_lines = min_lines;
        self
    }

    pub fn max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = max_lines;
        self
    }

    pub fn formatting(mut self, formatting: bool) -> Self {
        self.formatting = formatting;
        self
    }

    pub fn auto_focus(mut self, auto_focus: bool) -> Self {
        self.auto_focus = auto_focus;
        self
    }

    pub fn error_message(mut self, error_message: Option<String>) -> Self {
        self.error_message = error_message;
        self
    }

    pub fn rule(mut self, rule: RiseFieldRule) -> Self {
        self.rule = rule;
        self
    }

    pub fn max_chars(mut self, max_chars: Option<usize>) -> Self {
        self.max_chars = max_chars;
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

/// applies the canon rule first then the service ceiling, never splits a scalar
fn clamped_text(rule: RiseFieldRule, max_chars: Option<usize>, raw: &str) -> String {
    let clamped = rule.clamped(raw);
    match max_chars {
        Some(limit) if clamped.chars().count() > limit => clamped.chars().take(limit).collect(),
        _ => clamped,
    }
}

enum TextAreaMetrics {}

impl TextAreaMetrics {
    const LINE_HEIGHT: f32 = 22.;
    const CORNER_RADIUS: f32 = 30.;
    const OUTER_HORIZONTAL_PADDING: f32 = 16.;
    const OUTER_VERTICAL_PADDING: f32 = 6.;
    const FONT_SIZE: f32 = 15.;

    fn shape() -> Shape {
        Shape::continuous(Self::CORNER_RADIUS)
    }

    fn vertical_padding_total() -> f32 {
        Self::OUTER_VERTICAL_PADDING * 2. + 12.
    }

    /// text container insets, 8 top and bottom and 4 left and right
    fn text_container_inset() -> Edges<Pixels> {
        Edges { top: px(8.), left: px(4.), bottom: px(8.), right: px(4.) }
    }
}

struct TextAreaSession {
    field: Entity<InputUiState>,
    measured_height: f32,
    rule: RiseFieldRule,
    max_chars: Option<usize>,
    name: String,
    set_value: Option<InputUiSetValue>,
    on_focus_change: Option<InputUiFocusChange>,
    caret: Rc<dyn RiseTextInputCaret>,
    _subscriptions: Vec<Subscription>,
}

impl TextAreaSession {
    fn new(seed: String, auto_focus: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            let mut field = InputUiState::new(InputMode::MultiLine, cx);
            field.set_chromeless(true, cx);
            field.reset(seed.clone(), cx);
            field
        });
        let handle = field.read(cx).focus_handle(cx);
        let caret: Rc<dyn RiseTextInputCaret> = Rc::new(InputUiCaret::new(&field));
        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, _, event: &InputUiEvent, window, cx| {
                if *event == InputUiEvent::Changed {
                    this.text_did_change(window, cx);
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
        Self {
            field,
            measured_height: 0.,
            rule: RiseFieldRule::NONE,
            max_chars: None,
            name: String::new(),
            set_value: None,
            on_focus_change: None,
            caret,
            _subscriptions: subscriptions,
        }
    }

    fn text_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (current, marked, selection) = {
            let field = self.field.read(cx);
            (field.text().to_owned(), field.marked_range(), field.selection().range())
        };
        if marked.is_none() {
            let clamped = clamped_text(self.rule, self.max_chars, &current);
            if clamped != current {
                self.write_carrying_caret(&current, &clamped, selection, cx);
            }
        }
        let published = self.field.read(cx).text().to_owned();
        if let Some(set_value) = self.set_value.clone() {
            set_value(&self.name, &clamped_text(self.rule, self.max_chars, &published), window, cx);
        }
    }

    fn apply_external_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let (current, marked, selection) = {
            let field = self.field.read(cx);
            (field.text().to_owned(), field.marked_range(), field.selection().range())
        };
        if current == text || marked.is_some() {
            return;
        }
        self.write_carrying_caret(&current, text, selection, cx);
    }

    fn write_carrying_caret(&self, from: &str, to: &str, selection: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let carried = TextInputCaret::carried(byte_to_utf16_range(from, selection), from, to);
        let range = utf16_to_byte_range(to, carried).unwrap_or(to.len()..to.len());
        let to = to.to_owned();
        self.field.update(cx, |field, cx| {
            field.set_text(&to, cx);
            field.set_selection(Selection::new(range.start, range.end), cx);
        });
    }
}

impl RenderOnce for TextAreaUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let value = self.values.get(&self.name).cloned().unwrap_or_default();
        let error_text = self.resolved_error_text();

        let line_height = TextAreaMetrics::LINE_HEIGHT;
        let min_h = line_height * self.min_lines as f32 + TextAreaMetrics::vertical_padding_total();
        let max_h = line_height * self.max_lines as f32 + TextAreaMetrics::vertical_padding_total();

        let seed = value.clone();
        let auto_focus = self.auto_focus;
        let session = window.use_keyed_state(self.child_id("text-area"), cx, move |window, cx| {
            TextAreaSession::new(seed, auto_focus, window, cx)
        });
        let rule = self.rule;
        let max_chars = self.max_chars;
        let name = self.name.clone();
        let set_value = self.set_value.clone();
        let on_focus_change = self.on_focus_change.clone();
        session.update(cx, |session, _| {
            session.rule = rule;
            session.max_chars = max_chars;
            session.name = name;
            session.set_value = set_value;
            session.on_focus_change = on_focus_change;
        });
        let measured_height = session.read(cx).measured_height;
        let height = measured_height.max(min_h).min(max_h);

        let mut tap_target: Option<Entity<InputUiState>> = None;
        let content = if self.formatting {
            let reader = value.clone();
            let writer_name = self.name.clone();
            let writer = self.set_value.clone();
            let binding = Binding::new(
                move |_| reader.clone(),
                move |typed: String, window, cx| {
                    if let Some(writer) = &writer {
                        writer(&writer_name, &clamped_text(rule, max_chars, &typed), window, cx);
                    }
                },
            );
            let heights = session.clone();
            let focus_change = self.on_focus_change.clone();
            RichTextInputView::new(self.child_id("rich-text"), binding)
                .placeholder(self.placeholder.clone())
                .font_size(TextAreaMetrics::FONT_SIZE)
                .text_color(theme.text.primary)
                .placeholder_color(theme.text.secondary.opacity(0.82))
                .tint_color(theme.text.primary.opacity(0.9))
                .marker_color(theme.text.secondary.opacity(0.7))
                .code_color(theme.text.primary)
                .code_background(theme.text.primary.opacity(0.15))
                .autocorrection(true)
                .text_container_inset(TextAreaMetrics::text_container_inset())
                .is_scroll_enabled(measured_height > max_h)
                .focus_token(self.focus_token)
                .on_height_change(move |h, _, cx| {
                    heights.update(cx, |session, cx| {
                        if (h - session.measured_height).abs() > 0.5 {
                            session.measured_height = h;
                            cx.notify();
                        }
                    });
                })
                .on_focus_change(move |focused, window, cx| {
                    if let Some(focus_change) = &focus_change {
                        focus_change(focused, window, cx);
                    }
                })
                .into_any_element()
        } else {
            let tracked = session.clone();
            on_change(self.child_id("incoming"), value.clone(), false, window, cx, move |_, external, _, cx| {
                tracked.update(cx, |session, cx| session.apply_external_text(external, cx));
            });
            let field = session.read(cx).field.clone();
            let placeholder = self.placeholder.clone();
            let max_lines = self.max_lines;
            let is_enabled = is_enabled(cx);
            if !is_enabled && field.read(cx).focus_handle(cx).is_focused(window) {
                window.defer(cx, |window, cx| ScreenFocus::release_text_field(window, cx));
            }
            field.update(cx, |field, cx| {
                if field.is_disabled() == is_enabled {
                    field.set_disabled(!is_enabled, cx);
                }
                if field.placeholder().as_ref() != placeholder.as_str() {
                    field.set_placeholder(placeholder, cx);
                }
                if field.max_visible_lines() != max_lines {
                    field.set_max_visible_lines(max_lines, cx);
                }
            });
            if is_enabled {
                tap_target = Some(field.clone());
            }
            let inset = TextAreaMetrics::text_container_inset();
            div()
                .pt(inset.top)
                .pl(inset.left)
                .pb(inset.bottom)
                .pr(inset.right)
                .swiftui_font(Font::system_size(TextAreaMetrics::FONT_SIZE))
                .text_color(theme.text.primary)
                .child(field)
                .into_any_element()
        };

        let shape = TextAreaMetrics::shape();
        let keeps_focus_on_press = tap_target.is_some();
        let box_ = shape
            .round(div())
            .w_full()
            .when(self.formatting, |area| area.h(px(height)))
            .when(!self.formatting, |area| area.min_h(px(min_h)).max_h(px(max_h)))
            .padding_edges(EdgeSet::HORIZONTAL, TextAreaMetrics::OUTER_HORIZONTAL_PADDING)
            .padding_edges(EdgeSet::VERTICAL, TextAreaMetrics::OUTER_VERTICAL_PADDING)
            .bg(theme.bg._200)
            .overflow_hidden()
            .when_some(tap_target, |area, field| {
                area.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    let handle = field.read(cx).focus_handle(cx);
                    if handle.is_focused(window) {
                        return;
                    }
                    let end = field.read(cx).text().len();
                    field.update(cx, |field, cx| field.set_selection(Selection::new(end, end), cx));
                    window.focus(&handle, cx);
                })
            })
            .child(content);
        let box_ = if keeps_focus_on_press {
            box_.keyboard_dismiss_excluded().into_any_element()
        } else {
            box_.into_any_element()
        };

        vstack(4.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(box_)
            .when(!error_text.is_empty(), |area| area.child(ErrorTextUi::new(error_text).px(12.)))
    }
}
