use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext, Entity, ElementId, Focusable, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Subscription, Window, div,
    px,
};
use rise_i18n::i18n::tr;

use crate::icon_ui::IconUi;
use crate::input_ui::input_ui_state::{InputUiEvent, InputUiState};
use crate::keyboard_dismiss::KeyboardDismissModifiers;
use crate::liquid_chrome::LiquidChromeModifiers;
use crate::liquid_glass_button::{LiquidGlassButtonModifiers, LiquidPressShape};
use crate::main_text::MainText;
use crate::swiftui::binding::Binding;
use crate::swiftui::button_style::{ButtonAction, keyboard_focus_ring, plain};
use crate::swiftui::stacks::hstack;

const ICON_SIDE: f32 = 16.0;
const TEXT_SIZE: f32 = 15.0;
const PLACEHOLDER_OPACITY: f32 = 0.82;
const HEIGHT: f32 = 42.0;
const HORIZONTAL_PADDING: f32 = 14.0;
const CLEAR_HIT_SIDE: f32 = 36.0;
const CLEAR_TRAILING_PADDING: f32 = 4.0;

type TextHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// search capsule, with uses_inline_text_field it edits value itself
/// escape clears it and on an empty field calls on_cancel if set
/// otherwise it shows the query and on_activate opens the real search
#[derive(IntoElement)]
pub struct SearchFieldUi {
    id: ElementId,
    value: Binding<String>,
    placeholder: Option<SharedString>,
    on_change: Option<TextHandler>,
    on_activate: Option<ButtonAction>,
    on_submit: Option<ButtonAction>,
    on_cancel: Option<ButtonAction>,
    uses_inline_text_field: bool,
    auto_focus: bool,
    uses_flat_chrome: bool,
    keeps_focus_on_submit: bool,
    on_focus_lost: Option<ButtonAction>,
    focus_requests: u64,
}

impl SearchFieldUi {
    pub fn new(id: impl Into<ElementId>, value: Binding<String>) -> Self {
        Self {
            id: id.into(),
            value,
            placeholder: None,
            on_change: None,
            on_activate: None,
            on_submit: None,
            on_cancel: None,
            uses_inline_text_field: false,
            auto_focus: false,
            uses_flat_chrome: false,
            keeps_focus_on_submit: false,
            on_focus_lost: None,
            focus_requests: 0,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<Option<SharedString>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn on_change(mut self, on_change: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(on_change));
        self
    }

    pub fn on_activate(mut self, on_activate: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_activate = Some(Rc::new(on_activate));
        self
    }

    pub fn on_submit(mut self, on_submit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(on_submit));
        self
    }

    /// desktop only, the focused field binds escape to its own cancel so a key handler above never sees it
    /// a field its screen closes by a button closes by escape through this
    pub fn on_cancel(mut self, on_cancel: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(on_cancel));
        self
    }

    pub fn uses_inline_text_field(mut self, uses: bool) -> Self {
        self.uses_inline_text_field = uses;
        self
    }

    pub fn auto_focus(mut self, auto_focus: bool) -> Self {
        self.auto_focus = auto_focus;
        self
    }

    pub fn uses_flat_chrome(mut self, uses: bool) -> Self {
        self.uses_flat_chrome = uses;
        self
    }

    pub fn keeps_focus_on_submit(mut self, keeps: bool) -> Self {
        self.keeps_focus_on_submit = keeps;
        self
    }

    /// called when the inline field loses focus
    /// so a search mode can end on focus and not on the software keyboard hiding
    pub fn on_focus_lost(mut self, on_focus_lost: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_focus_lost = Some(Rc::new(on_focus_lost));
        self
    }

    /// desktop only, cmd f or ctrl f of the page
    /// each new value moves the caret into the field
    pub fn focus_requests(mut self, focus_requests: u64) -> Self {
        self.focus_requests = focus_requests;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }
}

/// handlers of the latest render, read by the field event subscription
struct SearchFieldHandlers {
    value: Binding<String>,
    on_change: Option<TextHandler>,
    on_submit: Option<ButtonAction>,
    on_cancel: Option<ButtonAction>,
    keeps_focus_on_submit: bool,
    on_focus_lost: Option<ButtonAction>,
}

struct SearchFieldState {
    field: Entity<InputUiState>,
    handlers: Rc<std::cell::RefCell<Option<SearchFieldHandlers>>>,
    _events: Subscription,
    _blur: Subscription,
    auto_focus_settled: bool,
}

impl SearchFieldState {
    fn new(window: &mut Window, cx: &mut gpui::Context<Self>) -> Self {
        let field = cx.new(InputUiState::single_line);
        field.update(cx, |field, cx| {
            field.set_chromeless(true, cx);
            field.set_font_size(Some(px(TEXT_SIZE)), cx);
        });
        let handlers: Rc<std::cell::RefCell<Option<SearchFieldHandlers>>> = Rc::default();
        let events = cx.subscribe_in(&field, window, |this, field, event, window, cx| {
            let Some((value, on_change, on_submit, on_cancel, keeps_focus)) = this.handlers.borrow().as_ref().map(|h| {
                (h.value.clone(), h.on_change.clone(), h.on_submit.clone(), h.on_cancel.clone(), h.keeps_focus_on_submit)
            }) else {
                return;
            };
            match event {
                InputUiEvent::Changed => {
                    let text = field.read(cx).text().to_owned();
                    if value.get(cx) != text {
                        value.set(text.clone(), window, cx);
                    }
                    if let Some(on_change) = on_change {
                        on_change(&text, window, cx);
                    }
                }
                InputUiEvent::Submitted => {
                    if let Some(on_submit) = on_submit {
                        on_submit(window, cx);
                    }
                    if keeps_focus {
                        field.read(cx).focus_handle(cx).focus(window, cx);
                    }
                }
                InputUiEvent::Cancelled => {
                    let is_empty = field.read(cx).text().is_empty();
                    match on_cancel.filter(|_| is_empty) {
                        Some(on_cancel) => on_cancel(window, cx),
                        None => field.update(cx, |field, cx| field.clear(cx)),
                    }
                }
            }
        });
        let field_focus = field.read(cx).focus_handle(cx);
        tracing::warn!(target: "riseonly::X697", "search field state created, focus {:?}", field_focus);
        let blur = cx.on_blur(&field_focus, window, |this, window, cx| {
            let field_focus = this.field.read(cx).focus_handle(cx);
            tracing::warn!(
                target: "riseonly::X697",
                "search field lost focus to {:?}, field {:?} in frame {}",
                window.focused(cx),
                field_focus,
                field_focus.is_in_rendered_frame(window),
            );
            let on_focus_lost = this.handlers.borrow().as_ref().and_then(|h| h.on_focus_lost.clone());
            if let Some(on_focus_lost) = on_focus_lost {
                on_focus_lost(window, cx);
            }
        });
        Self {
            field,
            handlers,
            _events: events,
            _blur: blur,
            auto_focus_settled: false,
        }
    }
}

fn clear(value: &Binding<String>, on_change: Option<&TextHandler>, window: &mut Window, cx: &mut App) {
    value.set(String::new(), window, cx);
    if let Some(on_change) = on_change {
        on_change("", window, cx);
    }
}

fn clear_icon(secondary: gpui::Hsla) -> AnyElement {
    match IconUi::sized("xmark.circle.fill", px(ICON_SIDE), secondary, true) {
        Some(icon) => icon.into_any_element(),
        None => div().size(px(ICON_SIDE)).into_any_element(),
    }
}

impl RenderOnce for SearchFieldUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = crate::theme(cx);
        let text_primary = theme.text.primary;
        let text_secondary = theme.text.secondary;
        let surface = theme.bg._100;
        let prompt_text: SharedString = self
            .placeholder
            .clone()
            .unwrap_or_else(|| tr("search_placeholder").into());
        let value = self.value.get(cx);

        let search_icon = match IconUi::sized("magnifyingglass", px(ICON_SIDE), text_secondary, false) {
            Some(icon) => icon.into_any_element(),
            None => div().size(px(ICON_SIDE)).into_any_element(),
        };

        let mut inline_field: Option<Entity<InputUiState>> = None;
        let text_slot: AnyElement = if self.uses_inline_text_field {
            let state = window.use_keyed_state(self.child_id("field"), cx, SearchFieldState::new);
            let (field, handlers) = {
                let state = state.read(cx);
                (state.field.clone(), state.handlers.clone())
            };
            handlers.replace(Some(SearchFieldHandlers {
                value: self.value.clone(),
                on_change: self.on_change.clone(),
                on_submit: self.on_submit.clone(),
                on_cancel: self.on_cancel.clone(),
                keeps_focus_on_submit: self.keeps_focus_on_submit,
                on_focus_lost: self.on_focus_lost.clone(),
            }));
            let placeholder = prompt_text.clone();
            let incoming = value.clone();
            let escape_to_layer = self.on_cancel.is_none();
            field.update(cx, |field, cx| {
                field.set_escape_clears_text(true);
                field.set_escape_to_layer(escape_to_layer);
                field.set_text_color(Some(text_primary), cx);
                field.set_placeholder_color(Some(text_secondary.opacity(PLACEHOLDER_OPACITY)), cx);
                if field.placeholder() != &placeholder {
                    field.set_placeholder(placeholder, cx);
                }
                if field.text() != incoming {
                    field.reset(incoming, cx);
                }
            });
            let requested_field = field.clone();
            crate::swiftui::lifecycle::on_change(
                self.child_id("focus-request"),
                self.focus_requests,
                false,
                window,
                cx,
                move |_, _, window, cx| requested_field.read(cx).focus_handle(cx).focus(window, cx),
            );
            inline_field = Some(field.clone());
            if self.auto_focus && !state.read(cx).auto_focus_settled {
                let handle = field.read(cx).focus_handle(cx);
                if handle.is_focused(window) {
                    state.update(cx, |state, _| state.auto_focus_settled = true);
                } else {
                    window.defer(cx, move |window, cx| handle.focus(window, cx));
                }
            }
            let line_height = field.read(cx).line_height();
            let field_element = if line_height > px(0.) {
                field.cached(StyleRefinement::default().w_full().h(line_height)).into_any_element()
            } else {
                field.into_any_element()
            };
            div()
                .flex_1()
                .min_w_0()
                .child(field_element)
                .into_any_element()
        } else {
            let (text, color) = if value.is_empty() {
                (prompt_text.clone(), text_secondary.opacity(PLACEHOLDER_OPACITY))
            } else {
                (SharedString::from(value.clone()), text_primary)
            };
            div()
                .flex_1()
                .min_w_0()
                .child(MainText::new(text).px(TEXT_SIZE).color(color).number_of_lines(1))
                .into_any_element()
        };

        let mut row = hstack(8.0).child(search_icon).child(text_slot);
        if !value.is_empty() {
            if let Some(field) = inline_field.clone() {
                let binding = self.value.clone();
                let on_change = self.on_change.clone();
                row = row.child(
                    plain(self.child_id("clear"))
                        .child(clear_icon(text_secondary))
                        .on_press(move |window, cx| {
                            clear(&binding, on_change.as_ref(), window, cx);
                            field.read(cx).focus_handle(cx).focus(window, cx);
                        }),
                );
            } else {
                row = row.child(clear_icon(text_secondary));
            }
        }
        let row = row.relative().w_full().h(px(HEIGHT)).px(px(HORIZONTAL_PADDING));
        let focus_ring = inline_field.as_ref().and_then(|field| {
            keyboard_focus_ring(&field.read(cx).focus_handle(cx), crate::theme(cx).primary._100, px(HEIGHT / 2.0), window)
        });

        let surfaced: AnyElement = if self.uses_flat_chrome {
            row.bg(surface).rounded_full().into_any_element()
        } else {
            row.rounded_full().liquid_chrome_capsule_with(0.12, true, None, false)
        };

        let tapped: AnyElement = if let Some(field) = inline_field {
            div()
                .id(self.child_id("field-tap"))
                .w_full()
                .child(surfaced)
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    field.read(cx).focus_handle(cx).focus(window, cx);
                    window.prevent_default();
                })
                .keyboard_dismiss_excluded()
                .into_any_element()
        } else if self.uses_flat_chrome {
            let on_activate = self.on_activate.clone();
            div()
                .id(self.child_id("tap"))
                .w_full()
                .child(surfaced)
                .on_click(move |_, window, cx| {
                    if let Some(on_activate) = on_activate.as_ref() {
                        on_activate(window, cx);
                    }
                })
                .into_any_element()
        } else {
            surfaced.liquid_press(
                self.child_id("press"),
                LiquidPressShape::Capsule,
                true,
                false,
                false,
                self.on_activate.clone(),
            )
        };

        let mut container = div().relative().w_full().child(tapped).children(focus_ring);
        if !self.uses_inline_text_field && !value.is_empty() {
            let binding = self.value.clone();
            let on_change = self.on_change.clone();
            let clear_action: ButtonAction = Rc::new(move |window, cx| clear(&binding, on_change.as_ref(), window, cx));
            container = container.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(px(CLEAR_TRAILING_PADDING))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .size(px(CLEAR_HIT_SIDE))
                            .liquid_press(
                                self.child_id("clear-press"),
                                LiquidPressShape::Circle,
                                false,
                                false,
                                false,
                                Some(clear_action),
                            ),
                    ),
            );
        }
        container
    }
}
