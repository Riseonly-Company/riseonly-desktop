use std::rc::Rc;

use gpui::{App, BoxShadow, ElementId, FontWeight, Hsla, Window, black, point, white};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::{Animated, Lerp};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::text::TruncationMode;
use rise_ui::swiftui::transition::{Presence, Transition};
use rise_ui::{IconUi, theme};

/// top row of the picker editor
/// where the selection goes on the left, selection circle of the current asset on the right
#[derive(IntoElement)]
pub struct MediaEditorPickerTopBar {
    pub destination_title: Option<String>,
    pub max_title_width: f32,
    pub shows_selection_control: bool,
    pub selection_number: Option<i64>,
    pub is_selectable: bool,
    on_toggle_selection: ButtonAction,
}

impl MediaEditorPickerTopBar {
    pub const HEIGHT: f32 = 44.0;
    const CIRCLE_SIZE: f32 = 30.0;
    const SPRING_RESPONSE: f64 = 0.28;
    const SPRING_DAMPING_FRACTION: f64 = 0.8;

    pub fn new(
        destination_title: Option<String>,
        max_title_width: f32,
        shows_selection_control: bool,
        selection_number: Option<i64>,
        is_selectable: bool,
        on_toggle_selection: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            destination_title,
            max_title_width,
            shows_selection_control,
            selection_number,
            is_selectable,
            on_toggle_selection: Rc::new(on_toggle_selection),
        }
    }

    fn destination(title: String, max_title_width: f32) -> impl IntoElement {
        let color: Hsla = white().opacity(0.6);
        pass_through()
            .flex()
            .items_center()
            .gap(px(6.0))
            .max_w(px(max_title_width))
            .min_w_0()
            .children(IconUi::sized("arrow.up", px(15.0), color, false))
            .child(
                MainText::new(title)
                    .px(17.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(color)
                    .number_of_lines(1)
                    .truncation(TruncationMode::Tail),
            )
    }

    fn selection_circle(
        selection_number: Option<i64>,
        is_selectable: bool,
        on_toggle_selection: ButtonAction,
        window: &mut Window,
        cx: &mut App,
    ) -> PlainButton {
        let target: f32 = if selection_number.is_some() { 1.0 } else { 0.0 };
        let now = RiseClock::monotonic();
        let motion = spring(Self::SPRING_RESPONSE, Self::SPRING_DAMPING_FRACTION);
        let selected = window.use_keyed_state("media-editor-picker-top-bar-selected", cx, |_, _| Animated::new(target));
        selected.update(cx, |selected, cx| {
            if *selected.target() != target {
                selected.set_in(target, motion, now, cx);
            }
        });
        let number = window.use_keyed_state("media-editor-picker-top-bar-number", cx, |_, _| {
            Presence::new(selection_number).with_transition(Transition::opacity())
        });
        number.update(cx, |number, cx| number.set_in(selection_number, motion, now, cx));

        let progress = selected.read(cx).frame_value(now, window).clamp(0.0, 1.0);
        let fill = black().opacity(0.25).lerp(&theme(cx).primary._100, progress);
        let stroke = white().opacity(0.9 + 0.1 * progress);
        let label = number.read(cx).render_transitioned(now, window, |selection_number| {
            div().px(px(4.0)).child(
                MainText::new(selection_number.to_string())
                    .px(15.0)
                    .font_weight(FontWeight::BOLD)
                    .color(white())
                    .minimum_scale_factor(0.6)
                    .line_limit(1),
            )
        });

        plain("media-editor-picker-top-bar-selection")
            .w(px(44.0))
            .h(px(44.0))
            .flex()
            .items_center()
            .justify_end()
            .opacity(if is_selectable { 1.0 } else { 0.4 })
            .on_press(move |window, cx| on_toggle_selection(window, cx))
            .child(
                div()
                    .id(ElementId::Name("media-editor-picker-top-bar-circle".into()))
                    .tooltip_label(tr("ctx_select"))
                    .w(px(Self::CIRCLE_SIZE))
                    .h(px(Self::CIRCLE_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(fill)
                    .border(px(2.0))
                    .border_color(stroke)
                    .shadow(vec![BoxShadow {
                        color: black().opacity(0.25),
                        offset: point(px(0.0), px(0.0)),
                        blur_radius: px(4.0),
                        spread_radius: px(0.0),
                        inset: false,
                    }])
                    .children(label),
            )
    }
}

impl RenderOnce for MediaEditorPickerTopBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let destination = self
            .destination_title
            .filter(|title| !title.is_empty())
            .map(|title| Self::destination(title, self.max_title_width));
        let circle = self.shows_selection_control.then(|| {
            Self::selection_circle(self.selection_number, self.is_selectable, self.on_toggle_selection, window, cx)
        });
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .h(px(Self::HEIGHT))
            .children(destination)
            .child(div().flex_1().min_w_0())
            .children(circle)
    }
}
