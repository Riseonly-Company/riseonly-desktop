use gpui::{App, IntoElement, RenderOnce, Window};
use rise_i18n::tr;

use crate::core::animations::components::shield_animation::ShieldAnimation;
use crate::modules::onboarding::pages::onboarding::onboarding::OnboardingSlideLayout;

#[derive(IntoElement, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnboardingSlide3;

impl RenderOnce for OnboardingSlide3 {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        OnboardingSlideLayout::new(
            "onboarding.slide3",
            tr("onboarding_slide3_title"),
            tr("onboarding_slide3_description"),
            250.0,
            |size| ShieldAnimation::new().size(size),
        )
    }
}
