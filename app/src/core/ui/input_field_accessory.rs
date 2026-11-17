use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{App, ElementId, FontWeight, SharedString, Window, anchored, deferred};
use rise_canon::RiseLimits;
use rise_ui::IconUi;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

/// trailing accessory inside an input field, error indicator or remaining counter
/// strict priority and never both, with an error only the error shows
/// the counter is wrong then and two signals in one slot read as noise
#[derive(IntoElement)]
pub struct InputFieldAccessory {
    id: ElementId,
    remaining: Option<usize>,
    error_message: Option<String>,
    on_error_tap: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

static REVEAL_AT: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("input_counter_reveal_remaining"));

impl InputFieldAccessory {
    pub const WIDTH: f32 = 38.;

    pub fn new(id: impl Into<ElementId>, remaining: Option<usize>, error_message: Option<String>) -> Self {
        Self { id: id.into(), remaining, error_message, on_error_tap: None }
    }

    pub fn on_error_tap(mut self, on_error_tap: Option<Rc<dyn Fn(&mut Window, &mut App)>>) -> Self {
        self.on_error_tap = on_error_tap;
        self
    }

    /// asked before layout, an empty slot would still eat 38 pt from a field with nothing to show
    pub fn is_visible(remaining: Option<usize>, error_message: Option<&str>) -> bool {
        if error_message.is_some_and(|message| !message.is_empty()) {
            return true;
        }
        let Some(remaining) = remaining else {
            return false;
        };
        Self::reveals(remaining)
    }

    fn reveals(remaining: usize) -> bool {
        i64::try_from(remaining).is_ok_and(|remaining| remaining <= *REVEAL_AT)
    }
}

impl RenderOnce for InputFieldAccessory {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let shows_error = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("shows-error")),
            cx,
            |_, _| false,
        );

        if let Some(error_message) = self.error_message.filter(|message| !message.is_empty()) {
            let on_error_tap = self.on_error_tap.clone();
            let toggle = shows_error.clone();
            let is_shown = *shows_error.read(cx);
            let dismiss = shows_error.clone();
            return div()
                .relative()
                .child(
                    plain(self.id.clone())
                        .on_press(move |window, cx| {
                            if let Some(on_error_tap) = &on_error_tap {
                                on_error_tap(window, cx);
                            } else {
                                toggle.update(cx, |shown, cx| {
                                    *shown = true;
                                    cx.notify();
                                });
                            }
                        })
                        .w(px(Self::WIDTH))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized(
                            "exclamationmark.circle.fill",
                            px(17.),
                            theme.semantic.error_100,
                            true,
                        )),
                )
                .when(is_shown, |slot| {
                    slot.child(deferred(
                        anchored().snap_to_window().child(
                            div()
                                .occlude()
                                .on_mouse_down_out(move |_, _, cx| {
                                    dismiss.update(cx, |shown, cx| {
                                        *shown = false;
                                        cx.notify();
                                    });
                                })
                                .child(InputFieldErrorTip::new(error_message)),
                        ),
                    ))
                })
                .into_any_element();
        }

        if let Some(remaining) = self.remaining.filter(|remaining| Self::reveals(*remaining)) {
            return div()
                .w(px(Self::WIDTH))
                .flex()
                .items_center()
                .justify_center()
                .swiftui_font(Font::system_weight(12., FontWeight::MEDIUM).monospaced_digit())
                .text_color(theme.text.secondary)
                .child(remaining.to_string())
                .into_any_element();
        }

        div().into_any_element()
    }
}

#[derive(IntoElement)]
struct InputFieldErrorTip {
    message: SharedString,
}

impl InputFieldErrorTip {
    const FONT_SIZE: f32 = 15.;
    const MAX_TEXT_WIDTH: f32 = 260.;

    fn new(message: impl Into<SharedString>) -> Self {
        Self { message: message.into() }
    }
}

impl RenderOnce for InputFieldErrorTip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        div()
            .bg(theme.bg._100)
            .rounded(theme.radius._300)
            .shadow_md()
            .padding_edges(EdgeSet::HORIZONTAL, 14.)
            .padding_edges(EdgeSet::VERTICAL, 10.)
            .child(
                div()
                    .max_w(px(Self::MAX_TEXT_WIDTH))
                    .swiftui_font(Font::system_size(Self::FONT_SIZE))
                    .text_color(theme.text.primary)
                    .child(self.message),
            )
    }
}
