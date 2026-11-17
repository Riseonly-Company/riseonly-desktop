use gpui::{AnyElement, App, SharedString, Window};
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_ui::lottie_view::LottieView;
use rise_ui::swiftui::prelude::*;

pub enum SheddingHeroMetrics {}

impl SheddingHeroMetrics {
    pub const SLOT: f32 = 120.0;
    pub const CANVAS: f32 = 440.0;
}

/// header icon that sheds sparkles, mostly left and right
/// only the slot takes part in layout, the canvas hangs around it so sparkles pass under later views
#[derive(IntoElement)]
pub struct SheddingHeroArt {
    animation: SharedString,
    still: Box<dyn FnOnce() -> AnyElement>,
}

impl SheddingHeroArt {
    pub fn new<Still: IntoElement>(
        animation: impl Into<SharedString>,
        still: impl FnOnce() -> Still + 'static,
    ) -> Self {
        Self {
            animation: animation.into(),
            still: Box::new(move || still().into_any_element()),
        }
    }

    fn moves(cx: &App) -> bool {
        !reduce_motion(cx)
            && !RiseEnergySavingStore::snapshot_reduces(RiseEnergySavingFeature::LoopAnimations)
    }
}

impl RenderOnce for SheddingHeroArt {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let slot = SheddingHeroMetrics::SLOT;
        let canvas = SheddingHeroMetrics::CANVAS;
        let content = if Self::moves(cx) && LottieView::can_play(&self.animation, cx) {
            let inset = (slot - canvas) / 2.0;
            div()
                .absolute()
                .top(px(inset))
                .left(px(inset))
                .frame_size(canvas, canvas)
                .child(LottieView::new(self.animation).size(canvas))
                .into_any_element()
        } else {
            (self.still)()
        };
        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .frame_size(slot, slot)
            .child(content)
    }
}
