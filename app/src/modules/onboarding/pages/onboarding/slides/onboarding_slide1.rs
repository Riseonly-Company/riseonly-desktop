use gpui::{App, IntoElement, RenderOnce, Window};
use rise_i18n::tr;

use crate::core::animations::components::magic_crystal_ball_animation::MagicCrystalBallAnimation;
use crate::modules::onboarding::pages::onboarding::onboarding::OnboardingSlideLayout;

#[derive(IntoElement, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnboardingSlide1;

impl RenderOnce for OnboardingSlide1 {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        OnboardingSlideLayout::new(
            "onboarding.slide1",
            tr("onboarding_slide1_title"),
            tr("onboarding_slide1_description"),
            200.0,
            |size| MagicCrystalBallAnimation::new().size(size),
        )
    }
}
