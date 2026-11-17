use gpui::{App, IntoElement, RenderOnce, Window};
use rise_i18n::tr;

use crate::core::animations::components::party_animation::PartyAnimation;
use crate::modules::onboarding::pages::onboarding::onboarding::OnboardingSlideLayout;

#[derive(IntoElement, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnboardingSlide4;

impl RenderOnce for OnboardingSlide4 {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        OnboardingSlideLayout::new(
            "onboarding.slide4",
            tr("onboarding_slide4_title"),
            tr("onboarding_slide4_description"),
            225.0,
            |size| PartyAnimation::new().size(size),
        )
    }
}
