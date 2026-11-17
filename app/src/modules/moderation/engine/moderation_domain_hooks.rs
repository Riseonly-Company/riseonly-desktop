use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

#[derive(Default)]
pub struct ModerationDomainHooks {
    pub refresh_loaded_surfaces: Option<fn(&mut App)>,
}

pub(crate) static MODERATION_DOMAIN_HOOKS: FeatureHookSlot<ModerationDomainHooks> = FeatureHookSlot::new("MODERATION_DOMAIN_HOOKS");

impl ModerationDomainHooks {
    pub fn install(hooks: Self) {
        let _ = MODERATION_DOMAIN_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static Self> {
        MODERATION_DOMAIN_HOOKS.get()
    }

    /// refreshes the moderation surfaces that are loaded
    pub fn refresh_loaded_surfaces(cx: &mut App) {
        if let Some(hook) = Self::installed().and_then(|hooks| hooks.refresh_loaded_surfaces) {
            hook(cx);
        }
    }
}
