use std::sync::OnceLock;

use gpui::App;

pub struct PremiumGiftCelebrationHooks {
    pub person_name: fn(user_id: &str, cx: &mut App) -> Option<String>,
}

static PREMIUM_GIFT_CELEBRATION_HOOKS: OnceLock<PremiumGiftCelebrationHooks> = OnceLock::new();

/// installs the hooks once, a second call keeps the first hooks
pub fn set_premium_gift_celebration_hooks(hooks: PremiumGiftCelebrationHooks) {
    let _ = PREMIUM_GIFT_CELEBRATION_HOOKS.set(hooks);
}

pub fn premium_gift_celebration_hooks() -> Option<&'static PremiumGiftCelebrationHooks> {
    PREMIUM_GIFT_CELEBRATION_HOOKS.get()
}

impl PremiumGiftCelebrationHooks {
    pub fn person_name_or_default(user_id: &str, cx: &mut App) -> Option<String> {
        premium_gift_celebration_hooks().and_then(|hooks| (hooks.person_name)(user_id, cx))
    }
}
