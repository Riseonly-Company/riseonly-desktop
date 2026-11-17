use std::cell::Cell;

use gpui::{AnyElement, App, ElementId, FontWeight, Hsla, Window};
use rise_i18n::tr;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT};
use rise_ui::swiftui::linear_gradient::{GradientStop, LinearGradient};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::UnitPoint;
use rise_ui::theme;

use crate::core::ui::rich_text::post_rich_text_render::PostRichTextCalibration;

/// post caption clamped to the line limit with more and less under it
/// keeps its rich text
/// the clamp is a height cut at whole lines
#[derive(IntoElement)]
pub struct PostCaptionClampUi {
    id: ElementId,
    font_size: f32,
    line_spacing: f32,
    toggle_color: Hsla,
    is_expanded: Binding<bool>,
    caption: AnyElement,
}

#[derive(Default)]
struct PostCaptionClampUiState {
    natural_height: Cell<f32>,
}

impl PostCaptionClampUi {
    pub fn new(
        id: impl Into<ElementId>,
        font_size: f32,
        line_spacing: f32,
        toggle_color: Hsla,
        is_expanded: Binding<bool>,
        caption: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            font_size,
            line_spacing,
            toggle_color,
            is_expanded,
            caption: caption.into_any_element(),
        }
    }

    /// system font line height plus the caption line spacing
    fn line_height(&self) -> f32 {
        self.font_size.max(PostRichTextCalibration::MINIMUM_FONT_SIZE) * NATURAL_LINE_HEIGHT + self.line_spacing
    }

    fn clamp_height(&self) -> f32 {
        (self.line_height() * PostRichTextCalibration::CAPTION_CONTENT_LINE_LIMIT as f32).floor()
    }

    /// one line of slack so a caption that fits the limit never shows more
    fn is_clampable(&self, natural_height: f32) -> bool {
        natural_height > self.clamp_height() + self.line_height() * 0.5
    }
}

impl RenderOnce for PostCaptionClampUi {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| PostCaptionClampUiState::default());
        let natural_height = state.read(cx).natural_height.get();
        let is_expanded = self.is_expanded.get(cx);
        let is_clampable = self.is_clampable(natural_height);
        let clamps = is_clampable && !is_expanded;
        let line_height = self.line_height();
        let clamp_height = self.clamp_height();
        let background = theme(cx).bg._100;

        let measured = state.clone();
        let natural = div()
            .relative()
            .w_full()
            .flex_none()
            .child(self.caption)
            .child(measure((self.id.clone(), "caption-natural"), move |bounds, _, cx| {
                let height = f32::from(bounds.size.height);
                let changed = {
                    let state = measured.read(cx);
                    if (state.natural_height.get() - height).abs() > 0.5 {
                        state.natural_height.set(height);
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    measured.update(cx, |_, cx| cx.notify());
                }
            }));

        let mut clip = div().relative().w_full().overflow_hidden().child(natural);
        if clamps {
            let fade_start = (1.0 - line_height / clamp_height).max(0.0);
            let fade = LinearGradient::new(
                vec![
                    GradientStop::new(background.opacity(0.0), 0.0),
                    GradientStop::new(background.opacity(0.0), fade_start),
                    GradientStop::new(background, 1.0),
                ],
                UnitPoint::TOP,
                UnitPoint::BOTTOM,
            );
            clip = clip.h(px(clamp_height)).child(div().absolute().inset_0().child(fade.view()));
        }

        let mut column = vstack(PostRichTextCalibration::CAPTION_TOGGLE_GAP).items_start().w_full().child(clip);
        if is_clampable {
            let binding = self.is_expanded.clone();
            column = column.child(
                plain((self.id, "caption-toggle"))
                    .w_full()
                    .on_press(move |window, cx| binding.toggle(window, cx))
                    .child(
                        div()
                            .w_full()
                            .swiftui_font(Font::system_size(self.font_size).weight(FontWeight::MEDIUM))
                            .text_color(self.toggle_color)
                            .child(tr(if is_expanded { "profile_bio_less" } else { "profile_bio_more" })),
                    ),
            );
        }
        column
    }
}
