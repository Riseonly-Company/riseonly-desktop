use std::ops::RangeInclusive;

use gpui::{
    App, BoxShadow, ElementId, FocusHandle, FontWeight, KeyDownEvent, Window, black, container_query, point, white,
};
use rise_canon::RiseGuideAnchor;
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;
use rise_ui::swiftui::slider::KEYBOARD_FRACTION;

use crate::modules::media_picker::editor::editor_bottom_bar::editor_panel_escape;
use crate::modules::media_picker::editor::editor_state::EditorImageAdjustments;

#[derive(IntoElement)]
pub struct EditorAdjustmentsPanel {
    pub adjustments: Binding<EditorImageAdjustments>,
    pub on_close: ButtonAction,
    pub on_confirm: ButtonAction,
}

impl EditorAdjustmentsPanel {
    pub fn new(
        adjustments: Binding<EditorImageAdjustments>,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
        on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { adjustments, on_close: std::rc::Rc::new(on_close), on_confirm: std::rc::Rc::new(on_confirm) }
    }

    fn component(
        &self,
        read: fn(&EditorImageAdjustments) -> f64,
        write: fn(&mut EditorImageAdjustments, f64),
    ) -> Binding<f64> {
        let reader = self.adjustments.clone();
        let writer = self.adjustments.clone();
        Binding::new(
            move |cx| read(&reader.get(cx)),
            move |value, window, cx| {
                let mut adjustments = writer.get(cx);
                write(&mut adjustments, value);
                writer.set(adjustments, window, cx);
            },
        )
    }

    fn slider(
        id: &'static str,
        title: String,
        binding: Binding<f64>,
        range: RangeInclusive<f64>,
        is_signed: bool,
        cx: &App,
    ) -> impl IntoElement {
        let shown = Self::format_value(binding.get(cx), is_signed);
        vstack(6.)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .child(
                hstack(DEFAULT_STACK_SPACING)
                    .w_full()
                    .child(
                        div()
                            .swiftui_font(Font::system(13., FontWeight::MEDIUM, FontDesign::Default))
                            .text_color(white().opacity(0.85))
                            .child(title),
                    )
                    .child(spacer())
                    .child(
                        div()
                            .swiftui_font(Font::system(11., FontWeight::SEMIBOLD, FontDesign::Default))
                            .text_color(white().opacity(0.6))
                            .child(shown),
                    ),
            )
            .child(div().w_full().h(px(22.)).child(BipolarSlider { id: id.into(), value: binding, range, is_signed }))
    }

    fn footer(on_close: ButtonAction, on_confirm: ButtonAction) -> impl IntoElement {
        const ICONS: [&str; 4] = ["slider.horizontal.3", "drop", "circle.lefthalf.filled", "waveform"];
        hstack(14.)
            .w_full()
            .child(
                LiquidGlassButton::new("editor-adjustments-close", LiquidGlassButtonShape::Circle)
                    .size(48.)
                    .on_press(move |window, cx| on_close(window, cx))
                    .children(IconUi::sized("xmark", px(16.), white(), false)),
            )
            .child(spacer_min_w(0.))
            .child(
                hstack(0.)
                    .px(px(4.))
                    .children(ICONS.into_iter().map(|icon| {
                        div()
                            .frame_size(43., 48.)
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized(icon, px(17.), white().opacity(0.82), false))
                    }))
                    .liquid_chrome_capsule_with(0.08, true, None, false),
            )
            .child(spacer_min_w(0.))
            .child(
                LiquidGlassButton::new("editor-adjustments-confirm", LiquidGlassButtonShape::Circle)
                    .size(48.)
                    .on_press(move |window, cx| on_confirm(window, cx))
                    .children(IconUi::sized("checkmark", px(16.), white(), false))
                    .guide_anchor(RiseGuideAnchor::EditorAdjustDone),
            )
            .px(px(14.))
            .pt(px(6.))
            .pb(px(22.))
    }

    fn format_value(value: f64, is_signed: bool) -> String {
        let percent = (value * 100.).round() as i64;
        if is_signed && percent >= 0 { format!("+{percent}") } else { percent.to_string() }
    }
}

impl RenderOnce for EditorAdjustmentsPanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let enhance = self.component(|a| a.enhance, |a, v| a.enhance = v);
        let brightness = self.component(|a| a.brightness, |a, v| a.brightness = v);
        let contrast = self.component(|a| a.contrast, |a, v| a.contrast = v);
        let saturation = self.component(|a| a.saturation, |a, v| a.saturation = v);
        let temperature = self.component(|a| a.temperature, |a, v| a.temperature = v);
        let highlights = self.component(|a| a.highlights, |a, v| a.highlights = v);
        let shadows = self.component(|a| a.shadows, |a, v| a.shadows = v);
        let vignette = self.component(|a| a.vignette, |a, v| a.vignette = v);

        let sliders = vstack(14.)
            .w_full()
            .child(Self::slider("editor-adj-enhance", tr("editor_adj_enhance"), enhance, 0.0..=1.0, false, cx))
            .child(Self::slider("editor-adj-brightness", tr("editor_adj_brightness"), brightness, -1.0..=1.0, true, cx))
            .child(Self::slider("editor-adj-contrast", tr("editor_adj_contrast"), contrast, -1.0..=1.0, true, cx))
            .child(Self::slider("editor-adj-saturation", tr("editor_adj_saturation"), saturation, -1.0..=1.0, true, cx))
            .child(Self::slider("editor-adj-temperature", tr("editor_adj_temperature"), temperature, -1.0..=1.0, true, cx))
            .child(Self::slider("editor-adj-highlights", tr("editor_adj_highlights"), highlights, -1.0..=1.0, true, cx))
            .child(Self::slider("editor-adj-shadows", tr("editor_adj_shadows"), shadows, -1.0..=1.0, true, cx))
            .child(Self::slider("editor-adj-vignette", tr("editor_adj_vignette"), vignette, 0.0..=1.0, false, cx))
            .px(px(18.))
            .pt(px(16.))
            .pb(px(12.));

        let content = vstack(0.)
            .w_full()
            .child(scroll_view("editor-adjustments-scroll").w_full().max_h(px(240.)).child(sliders))
            .child(Self::footer(self.on_close.clone(), self.on_confirm));
        editor_panel_escape("editor-adjustments-panel", "editor-adjustments-panel-appear", self.on_close, content, window, cx)
    }
}

struct BipolarSliderState {
    width: f32,
    press_is_on_knob: Option<bool>,
    focus: FocusHandle,
}

#[derive(IntoElement)]
struct BipolarSlider {
    id: ElementId,
    value: Binding<f64>,
    range: RangeInclusive<f64>,
    is_signed: bool,
}

impl BipolarSlider {
    const KNOB_SIZE: f32 = 16.;
    const TAP_SLOP: f32 = 10.;

    fn normalize(value: f64, range: &RangeInclusive<f64>) -> f64 {
        let span = range.end() - range.start();
        if span <= 0. {
            return 0.;
        }
        (value - range.start()) / span
    }

    fn denormalize(progress: f64, range: &RangeInclusive<f64>) -> f64 {
        range.start() + (range.end() - range.start()) * progress
    }

    fn value_at(x: f32, width: f32, range: &RangeInclusive<f64>) -> f64 {
        let progress = (x / width.max(1.)).clamp(0., 1.);
        Self::denormalize(f64::from(progress), range)
    }

    fn body(width: f32, progress: f32, is_signed: bool) -> impl IntoElement {
        let knob_x = width * progress;
        let (fill_x, fill_width) = if is_signed {
            let middle = width * 0.5;
            (middle.min(knob_x), middle.max(knob_x) - middle.min(knob_x))
        } else {
            (0., knob_x)
        };
        let track = |x: f32, track_width: f32| div().absolute().left(px(x)).top(px(9.5)).w(px(track_width)).h(px(3.)).capsule();
        div()
            .relative()
            .size_full()
            .child(track(0., width).bg(white().opacity(0.18)))
            .child(track(fill_x, fill_width).bg(white()))
            .child(
                div()
                    .absolute()
                    .left(px(knob_x - Self::KNOB_SIZE / 2.))
                    .top(px(3.))
                    .size(px(Self::KNOB_SIZE))
                    .circle()
                    .bg(white())
                    .shadow(vec![BoxShadow {
                        color: black().opacity(0.4),
                        offset: point(px(0.), px(1.)),
                        blur_radius: px(2.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .child(centred_stroke(black().opacity(0.2), 0.5, Shape::Circle)),
            )
    }
}

impl RenderOnce for BipolarSlider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| BipolarSliderState {
            width: 0.,
            press_is_on_knob: None,
            focus: cx.focus_handle(),
        });
        let focus = state.read(cx).focus.clone();
        let progress = Self::normalize(self.value.get(cx), &self.range) as f32;
        let is_signed = self.is_signed;

        let changed = {
            let state = state.clone();
            let value = self.value.clone();
            let range = self.range.clone();
            move |gesture: &DragGestureValue, window: &mut Window, cx: &mut App| {
                let width = state.read(cx).width;
                let on_knob = state.read(cx).press_is_on_knob.unwrap_or_else(|| {
                    let knob_x = width * Self::normalize(value.get(cx), &range) as f32;
                    (f32::from(gesture.start_location.x) - knob_x).abs() <= Self::KNOB_SIZE / 2.
                });
                state.update(cx, |state, _| state.press_is_on_knob = Some(on_knob));
                if on_knob {
                    value.set(Self::value_at(f32::from(gesture.location.x), width, &range), window, cx);
                }
            }
        };
        let ended = {
            let state = state.clone();
            let value = self.value.clone();
            let range = self.range.clone();
            move |gesture: &DragGestureValue, window: &mut Window, cx: &mut App| {
                let (width, on_knob) = {
                    let state = state.read(cx);
                    (state.width, state.press_is_on_knob)
                };
                state.update(cx, |state, _| state.press_is_on_knob = None);
                let travelled = f32::from(gesture.translation.width).hypot(f32::from(gesture.translation.height));
                if on_knob != Some(true) && travelled <= Self::TAP_SLOP {
                    value.set(Self::value_at(f32::from(gesture.location.x), width, &range), window, cx);
                }
            }
        };

        let measured = state.clone();
        let keys_value = self.value;
        let keys_range = self.range;
        div()
            .id(self.id.clone())
            .relative()
            .w_full()
            .h(px(22.))
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let direction = match event.keystroke.key.as_str() {
                    "left" | "down" => -1.,
                    "right" | "up" => 1.,
                    _ => return,
                };
                cx.stop_propagation();
                let span = keys_range.end() - keys_range.start();
                let next = keys_value.get(cx) + direction * span * KEYBOARD_FRACTION;
                keys_value.set(next.clamp(*keys_range.start(), *keys_range.end()), window, cx);
            })
            .child(container_query(move |size, _, _| Self::body(f32::from(size.width), progress, is_signed)))
            .child(measure("bipolar-slider-bounds", move |bounds, _, cx| {
                measured.update(cx, |state, _| state.width = f32::from(bounds.size.width));
            }))
            .gesture((self.id, "gesture"), DragGesture::new(0.).on_changed(changed).on_ended(ended))
    }
}
