use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, Entity, Focusable, Hsla, MouseButton, SharedString, Subscription, Window, rgb};
use rise_ui::input_ui::{InputMode, InputUiEvent, InputUiState};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::NavigationStore;
use crate::core::lib::canon_field_rule::{RiseFocusedTextInput, RiseTextInputCaret};
use crate::core::ui::input_ui::InputUiCaret;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

pub fn hex_color(value: u32) -> Hsla {
    Hsla::from(rgb(value))
}

pub fn pop_screen(nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
    }
}

/// true if the guide sandbox owns the jobs scene, a sandbox never built is never armed
pub fn guide_owns_jobs_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_jobs_scene())
}

/// form root tracks focus so cmd or ctrl enter works with no field focused
/// and a click beside a field blurs it
pub fn keeps_form_focus(field: impl IntoElement) -> Div {
    div().w_full().on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default()).child(field)
}

pub(crate) type VacancyTextEditorFocusChange = Rc<dyn Fn(bool, &mut Window, &mut App)>;

struct VacancyTextEditorSession {
    field: Entity<InputUiState>,
    text: Binding<String>,
    on_focus_change: Option<VacancyTextEditorFocusChange>,
    placeholder_color: Option<Hsla>,
    caret: Rc<dyn RiseTextInputCaret>,
    _subscriptions: Vec<Subscription>,
}

impl VacancyTextEditorSession {
    fn new(text: Binding<String>, seed: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| {
            let mut field = InputUiState::new(InputMode::MultiLine, cx);
            field.set_chromeless(true, cx);
            field.reset(seed, cx);
            field
        });
        let handle = field.read(cx).focus_handle(cx);
        let caret: Rc<dyn RiseTextInputCaret> = Rc::new(InputUiCaret::new(&field));
        let subscriptions = vec![
            cx.subscribe_in(&field, window, |this, field, event: &InputUiEvent, window, cx| {
                if *event == InputUiEvent::Changed {
                    let value = field.read(cx).text().to_owned();
                    let text = this.text.clone();
                    text.set(value, window, cx);
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
        Self { field, text, on_focus_change: None, placeholder_color: None, caret, _subscriptions: subscriptions }
    }
}

/// text editor of the vacancy forms, paints its own box and scrolls to the focused editor
#[derive(IntoElement)]
pub struct VacancyTextEditor {
    id: ElementId,
    text: Binding<String>,
    placeholder: String,
    placeholder_color: Option<Hsla>,
    font_size: f32,
    min_height: f32,
    max_lines: usize,
    on_focus_change: Option<VacancyTextEditorFocusChange>,
}

impl VacancyTextEditor {
    const DEFAULT_MAX_LINES: usize = 64;

    pub fn new(id: impl Into<ElementId>, text: Binding<String>) -> Self {
        Self {
            id: id.into(),
            text,
            placeholder: String::new(),
            placeholder_color: None,
            font_size: 15.0,
            min_height: 0.0,
            max_lines: Self::DEFAULT_MAX_LINES,
            on_focus_change: None,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>, color: Hsla) -> Self {
        self.placeholder = placeholder.into();
        self.placeholder_color = Some(color);
        self
    }

    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    pub fn min_height(mut self, min_height: f32) -> Self {
        self.min_height = min_height;
        self
    }

    pub fn max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = max_lines;
        self
    }

    pub fn on_focus_change(mut self, on_focus_change: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_focus_change = Some(Rc::new(on_focus_change));
        self
    }
}

impl RenderOnce for VacancyTextEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let value = self.text.get(cx);
        let session_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("session"));
        let seed = value.clone();
        let seed_binding = self.text.clone();
        let session = window
            .use_keyed_state(session_id, cx, move |window, cx| VacancyTextEditorSession::new(seed_binding, seed, window, cx));
        let text = self.text;
        let on_focus_change = self.on_focus_change;
        let placeholder_color = self.placeholder_color;
        let color_changed = session.update(cx, |session, _| {
            session.text = text;
            session.on_focus_change = on_focus_change;
            let changed = session.placeholder_color != placeholder_color;
            session.placeholder_color = placeholder_color;
            changed
        });
        let field = session.read(cx).field.clone();
        let placeholder = self.placeholder;
        let max_lines = self.max_lines;
        let font_size = px(self.font_size);
        field.update(cx, |field, cx| {
            if field.placeholder().as_ref() != placeholder.as_str() {
                field.set_placeholder(placeholder, cx);
            }
            if color_changed {
                field.set_placeholder_color(placeholder_color, cx);
            }
            if field.max_visible_lines() != max_lines {
                field.set_max_visible_lines(max_lines, cx);
            }
            if field.font_size() != Some(font_size) {
                field.set_font_size(Some(font_size), cx);
            }
            if field.text() != value && field.marked_range().is_none() {
                field.reset(value, cx);
            }
        });
        let handle = field.read(cx).focus_handle(cx);
        div()
            .w_full()
            .min_h(px(self.min_height))
            .swiftui_font(Font::system_size(self.font_size))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&handle, cx);
                window.prevent_default();
            })
            .child(field)
    }
}
