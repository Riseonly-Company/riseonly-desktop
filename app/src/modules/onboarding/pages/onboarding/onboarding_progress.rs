use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::ONBOARDING_SLIDES_SEEN;

/// how far the intro carousel got before the person left
/// it runs before an account exists and the analytics sender drops events with no profile
/// so this number crosses that gap for onboarding_complete
pub struct OnboardingProgress;

impl OnboardingProgress {
    pub const KEY: &'static str = ONBOARDING_SLIDES_SEEN;

    pub fn slides_seen() -> i64 {
        UserDefaults::standard().integer(Self::KEY)
    }

    /// monotonic, going back a slide doesnt un-see it
    pub fn record(slide_index: usize) {
        let reached = i64::try_from(slide_index).unwrap_or(i64::MAX).saturating_add(1);
        if reached <= Self::slides_seen() {
            return;
        }
        UserDefaults::standard().set(Self::KEY, reached);
    }
}
