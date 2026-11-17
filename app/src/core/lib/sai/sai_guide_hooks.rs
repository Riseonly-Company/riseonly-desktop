use serde_json::Value;

use crate::core::feature_hooks::FeatureHookSlot;

pub(crate) struct SaiGuideHooks {
    pub refuses_json_object: Box<dyn Fn(&Value) -> bool + Send + Sync>,
    pub refuses_service_method: Box<dyn Fn(&str, &str) -> bool + Send + Sync>,
}

pub(crate) static SAI_GUIDE_HOOKS: FeatureHookSlot<SaiGuideHooks> = FeatureHookSlot::new("SAI_GUIDE_HOOKS");

pub(crate) fn set_sai_guide_hooks(hooks: SaiGuideHooks) {
    let _ = SAI_GUIDE_HOOKS.set(hooks);
}

pub(crate) fn sai_guide_hooks() -> Option<&'static SaiGuideHooks> {
    SAI_GUIDE_HOOKS.get()
}
