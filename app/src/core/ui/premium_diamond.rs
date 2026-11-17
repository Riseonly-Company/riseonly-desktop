use gpui::{App, Window};
use rise_ui::lottie_view::LottieView;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::shedding_hero_art::SheddingHeroArt;

/// the premium mark, the spark in the premium gradient from the sparks sources
/// paints 83% of its frame, the middle frame is the reduce motion still
/// diamond.json stays in the bundle as achievement artwork
#[derive(IntoElement, Clone, Copy, Debug, PartialEq)]
pub struct PremiumDiamond {
    size: f32,
    looping: bool,
}

impl Default for PremiumDiamond {
    fn default() -> Self {
        Self { size: 16.0, looping: true }
    }
}

impl PremiumDiamond {
    pub const ANIMATION: &'static str = "premium_spark";

    pub fn new() -> Self {
        Self::default()
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn looping(mut self, looping: bool) -> Self {
        self.looping = looping;
        self
    }
}

impl RenderOnce for PremiumDiamond {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex_none()
            .frame_size(self.size, self.size)
            .child(LottieView::new(Self::ANIMATION).size(self.size).looping(self.looping))
    }
}

/// header of the premium page, the premium spark shedding sparkles
/// the still is the middle frame of the mark at hero size, 96 pt tip to tip
#[derive(IntoElement, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PremiumPageHero;

impl PremiumPageHero {
    pub fn new() -> Self {
        Self
    }
}

impl RenderOnce for PremiumPageHero {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        SheddingHeroArt::new("premium_page_hero", || {
            LottieView::new(PremiumDiamond::ANIMATION)
                .size(96.0 / 0.83)
                .looping(false)
                .speed(0.0)
        })
    }
}
