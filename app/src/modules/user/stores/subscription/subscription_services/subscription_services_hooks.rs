use rise_core::foundation::foundation_url::Url;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct SubscriptionServicesHooks {
    pub public_site_url: fn() -> Url,
}

pub(crate) static SUBSCRIPTION_SERVICES_HOOKS: FeatureHookSlot<SubscriptionServicesHooks> = FeatureHookSlot::new("SUBSCRIPTION_SERVICES_HOOKS");

impl SubscriptionServicesHooks {
    pub fn install(hooks: SubscriptionServicesHooks) {
        let _ = SUBSCRIPTION_SERVICES_HOOKS.set(hooks);
    }

    pub fn installed() -> Option<&'static SubscriptionServicesHooks> {
        SUBSCRIPTION_SERVICES_HOOKS.get()
    }

    /// none until the public link layer installed the slot
    pub fn public_site_url() -> Option<Url> {
        Self::installed().map(|hooks| (hooks.public_site_url)())
    }
}
