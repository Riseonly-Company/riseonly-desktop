use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, ElementId, Entity, FocusHandle, FontWeight, Focusable, SharedString, Subscription, Window, anchored,
    black, deferred, point,
};

use crate::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use crate::guide_anchor_ui::guide_anchor_registry::{GuideAnchorCoverKind, GuideAnchorCoverModifiers};
use crate::keyboard_dismiss::ScreenFocus;
use crate::layer_keys::{LayerCancel, LayerKeysExt};
use crate::input_ui::input_ui_state::{InputUiEvent, InputUiState};
use crate::main_text::{MainText, TextAlignT};
use crate::swiftui::prelude::*;
use crate::swiftui::{ButtonAction, button_chrome};

const CARD_WIDTH: f32 = 300.0;
const CARD_CORNER_RADIUS: f32 = 14.0;
const CARD_PADDING: f32 = 16.0;
const CARD_SPACING: f32 = 14.0;
const FIELD_HEIGHT: f32 = 36.0;
const FIELD_HORIZONTAL_PADDING: f32 = 12.0;
const FIELD_CORNER_RADIUS: f32 = 10.0;
const FIELD_FONT_SIZE: f32 = 15.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_VERTICAL_PADDING: f32 = 8.0;

#[derive(Clone)]
struct TextFieldAlertHandlers {
    is_presented: Binding<bool>,
    text: Binding<String>,
    on_confirm: ButtonAction,
}

impl TextFieldAlertHandlers {
    fn cancel(&self, window: &mut Window, cx: &mut App) {
        self.is_presented.set(false, window, cx);
    }

    fn confirm(&self, window: &mut Window, cx: &mut App) {
        self.is_presented.set(false, window, cx);
        (self.on_confirm)(window, cx);
    }
}

struct TextFieldAlertState {
    field: Entity<InputUiState>,
    handlers: Rc<RefCell<Option<TextFieldAlertHandlers>>>,
    focused_presentation: bool,
    previous_focus: Option<FocusHandle>,
    _events: Subscription,
}

impl TextFieldAlertState {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = cx.new(InputUiState::single_line);
        field.update(cx, |field, cx| {
            field.set_chromeless(true, cx);
            field.set_font_size(Some(px(FIELD_FONT_SIZE)), cx);
            field.set_escape_to_layer(false);
        });
        let handlers: Rc<RefCell<Option<TextFieldAlertHandlers>>> = Rc::default();
        let events = cx.subscribe_in(&field, window, |this, field, event, window, cx| {
            let Some(handlers) = this.handlers.borrow().clone() else {
                return;
            };
            match event {
                InputUiEvent::Changed => {
                    let text = field.read(cx).text().to_owned();
                    if handlers.text.get(cx) == text {
                        return;
                    }
                    handlers.text.set(text.clone(), window, cx);
                    let stored = handlers.text.get(cx);
                    if stored != text {
                        field.update(cx, |field, cx| field.set_text(stored, cx));
                    }
                }
                InputUiEvent::Submitted => handlers.confirm(window, cx),
                InputUiEvent::Cancelled => handlers.cancel(window, cx),
            }
        });
        Self { field, handlers, focused_presentation: false, previous_focus: None, _events: events }
    }
}

#[derive(IntoElement)]
pub struct TextFieldAlert {
    id: ElementId,
    is_presented: Binding<bool>,
    title: SharedString,
    placeholder: SharedString,
    text: Binding<String>,
    cancel_title: SharedString,
    confirm_title: SharedString,
    on_confirm: ButtonAction,
}

impl TextFieldAlert {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        is_presented: Binding<bool>,
        title: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        text: Binding<String>,
        cancel_title: impl Into<SharedString>,
        confirm_title: impl Into<SharedString>,
        on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            is_presented,
            title: title.into(),
            placeholder: placeholder.into(),
            text,
            cancel_title: cancel_title.into(),
            confirm_title: confirm_title.into(),
            on_confirm: Rc::new(on_confirm),
        }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn button(
        id: ElementId,
        title: SharedString,
        font_weight: FontWeight,
        color: gpui::Hsla,
        action: ButtonAction,
    ) -> AnyElement {
        button_chrome(div().id(id).occlude(), false, Some(action))
            .flex_1()
            .flex()
            .justify_center()
            .py(px(BUTTON_VERTICAL_PADDING))
            .child(MainText::new(title).px(15.).font_weight(font_weight).color(color))
            .into_any_element()
    }
}

impl RenderOnce for TextFieldAlert {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, TextFieldAlertState::new);
        let (field, slot) = {
            let state = state.read(cx);
            (state.field.clone(), state.handlers.clone())
        };
        let handlers = TextFieldAlertHandlers {
            is_presented: self.is_presented.clone(),
            text: self.text.clone(),
            on_confirm: self.on_confirm.clone(),
        };
        *slot.borrow_mut() = Some(handlers.clone());

        let is_presented = self.is_presented.get(cx);
        let opening = field.clone();
        let placeholder = self.placeholder.clone();
        let initial_text = self.text.clone();
        let focus_owner = state.clone();
        on_change(self.child_id("presented"), is_presented, true, window, cx, move |_, open, window, cx| {
            let field_handle = opening.read(cx).focus_handle(cx);
            if !*open {
                let (focused_presentation, previous) = focus_owner.update(cx, |state, _| {
                    (std::mem::take(&mut state.focused_presentation), state.previous_focus.take())
                });
                if !focused_presentation || window.focused(cx).is_some_and(|focused| focused != field_handle) {
                    return;
                }
                match previous {
                    Some(previous) => window.focus(&previous, cx),
                    None => ScreenFocus::release_text_field(window, cx),
                }
                return;
            }
            let previous = window.focused(cx);
            focus_owner.update(cx, |state, _| {
                state.focused_presentation = true;
                state.previous_focus = previous;
            });
            let text = initial_text.get(cx);
            opening.update(cx, |field, cx| {
                field.set_placeholder(placeholder, cx);
                field.set_text(text, cx);
                field.select_all_text(cx);
            });
            field_handle.focus(window, cx);
        });

        if !is_presented {
            return div().into_any_element();
        }

        let theme = crate::theme(cx).clone();
        field.update(cx, |field, cx| {
            field.set_text_color(Some(theme.text.primary), cx);
            field.set_placeholder_color(Some(theme.text.secondary), cx);
        });

        let on_escape = handlers.clone();
        let on_submit = handlers.clone();
        let on_cancel = handlers.clone();
        let on_scrim = handlers.clone();
        let on_confirm = handlers;
        let viewport = window.viewport_size();
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .id(self.child_id("scrim"))
                    .occlude()
                    .w(viewport.width)
                    .h(viewport.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(black().opacity(BottomSheetMetrics::BACKDROP_OPACITY as f32))
                    .on_click(move |_, window, cx| on_scrim.cancel(window, cx))
                    .child(
                        div()
                            .id(self.child_id("card"))
                            .occlude()
                            .layer_form_keys(
                                LayerCancel::dismiss(move |window, cx| on_escape.cancel(window, cx)),
                                true,
                                move |window, cx| on_submit.confirm(window, cx),
                            )
                            .w(px(CARD_WIDTH))
                            .p(px(CARD_PADDING))
                            .rounded(px(CARD_CORNER_RADIUS))
                            .bg(theme.bg._200)
                            .flex()
                            .flex_col()
                            .gap(px(CARD_SPACING))
                            .child(
                                MainText::new(self.title.clone())
                                    .px(17.)
                                    .tac(TextAlignT::Center)
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .h(px(FIELD_HEIGHT))
                                    .px(px(FIELD_HORIZONTAL_PADDING))
                                    .flex()
                                    .items_center()
                                    .rounded(px(FIELD_CORNER_RADIUS))
                                    .bg(theme.bg._100)
                                    .child(field),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .gap(px(BUTTON_SPACING))
                                    .child(Self::button(
                                        self.child_id("cancel"),
                                        self.cancel_title.clone(),
                                        FontWeight::NORMAL,
                                        theme.text.primary,
                                        Rc::new(move |window: &mut Window, cx: &mut App| on_cancel.cancel(window, cx)),
                                    ))
                                    .child(Self::button(
                                        self.child_id("confirm"),
                                        self.confirm_title.clone(),
                                        FontWeight::SEMIBOLD,
                                        theme.primary._100,
                                        Rc::new(move |window: &mut Window, cx: &mut App| {
                                            on_confirm.confirm(window, cx)
                                        }),
                                    )),
                            )
                            .guide_anchor_cover(self.child_id("guide-alert"), GuideAnchorCoverKind::Alert),
                    )
                    .guide_anchor_cover(self.child_id("guide-cover"), GuideAnchorCoverKind::Layer),
            ),
        )
        .with_priority(1)
        .into_any_element()
    }
}
