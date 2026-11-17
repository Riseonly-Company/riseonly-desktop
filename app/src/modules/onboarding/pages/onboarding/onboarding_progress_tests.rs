use super::onboarding_progress::OnboardingProgress;

#[test]
fn going_back_a_slide_does_not_unsee_it() {
    OnboardingProgress::record(10);
    assert_eq!(OnboardingProgress::slides_seen(), 11);

    OnboardingProgress::record(0);
    assert_eq!(OnboardingProgress::slides_seen(), 11, "a step back must not lower the count");

    OnboardingProgress::record(2);
    assert_eq!(OnboardingProgress::slides_seen(), 11);
}
