use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;

pub struct PremiumAuthHooks {
    pub is_current_user: Box<dyn Fn(&str, &App) -> bool + Send + Sync>,
    pub profile: Box<dyn Fn(&App) -> Option<ProfileModel> + Send + Sync>,
}

pub(crate) static PREMIUM_AUTH_HOOKS: FeatureHookSlot<PremiumAuthHooks> = FeatureHookSlot::new("PREMIUM_AUTH_HOOKS");

impl PremiumAuthHooks {
    pub fn install(hooks: Self) {
        let _ = PREMIUM_AUTH_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        PREMIUM_AUTH_HOOKS.get()
    }

    /// false until auth installs the slot
    pub fn is_current_user(sender_id: &str, cx: &App) -> bool {
        Self::installed().is_some_and(|hooks| (hooks.is_current_user)(sender_id, cx))
    }

    /// none until auth installs the slot
    pub fn profile(cx: &App) -> Option<ProfileModel> {
        Self::installed().and_then(|hooks| (hooks.profile)(cx))
    }
}
