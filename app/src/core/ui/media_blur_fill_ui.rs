use gpui::{App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, hsla};
use rise_ui::swiftui::blur::BlurModifiers;
use rise_ui::swiftui::shapes::pass_through;
use rise_ui::swiftui::transition::TransitionModifiers;

use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};

const BLUR_RADIUS: f32 = 18.0;
const DIMMING: f32 = 0.22;
const OVERSCAN: f32 = 1.14;
const DECODE_PIXEL_DIMENSION: i64 = 96;

/// blurred copy of the frame behind itself for boxes the frame doesnt fill
/// cropping would alter someone elses frame and black bars read as a layout hole
/// bitmap is decoded tiny since the blur hides the upscale
#[derive(IntoElement)]
pub struct MediaBlurFillUi {
    source: SharedString,
    persist_to_disk: bool,
    cache_policy: CleverImageCachePolicy,
}

impl MediaBlurFillUi {
    pub fn new(source: impl Into<SharedString>) -> Self {
        Self {
            source: source.into(),
            persist_to_disk: false,
            cache_policy: CleverImageCachePolicy::Standard,
        }
    }

    pub fn persist_to_disk(mut self, persist_to_disk: bool) -> Self {
        self.persist_to_disk = persist_to_disk;
        self
    }

    pub fn cache_policy(mut self, cache_policy: CleverImageCachePolicy) -> Self {
        self.cache_policy = cache_policy;
        self
    }
}

impl RenderOnce for MediaBlurFillUi {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let image = CleverImageUi::new(self.source)
            .content_mode(ContentMode::Fill)
            .corner_radius(0.0)
            .persist_to_disk(self.persist_to_disk)
            .shows_progress_indicator(false)
            .maximum_decode_pixel_dimension(DECODE_PIXEL_DIMENSION)
            .cache_policy(self.cache_policy)
            .shows_failure_icon(false)
            .blur(BLUR_RADIUS);

        pass_through()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(image)
                    .scale_effect(OVERSCAN),
            )
            .child(div().absolute().inset_0().bg(hsla(0.0, 0.0, 0.0, DIMMING)))
    }
}
