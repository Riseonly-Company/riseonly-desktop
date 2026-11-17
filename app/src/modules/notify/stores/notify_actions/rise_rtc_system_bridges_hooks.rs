use crate::core::feature_hooks::FeatureHookSlot;

pub struct RiseRTCSystemBridgesHooks {
    pub current: fn() -> String,
}

pub(crate) static RISE_RTC_SYSTEM_BRIDGES_HOOKS: FeatureHookSlot<RiseRTCSystemBridgesHooks> = FeatureHookSlot::new("RISE_RTC_SYSTEM_BRIDGES_HOOKS");

/// a second install is ignored, the first owner keeps the slot
pub fn set_rise_rtc_system_bridges_hooks(hooks: RiseRTCSystemBridgesHooks) {
    let _ = RISE_RTC_SYSTEM_BRIDGES_HOOKS.set(hooks);
}

pub fn rise_rtc_system_bridges_hooks() -> Option<&'static RiseRTCSystemBridgesHooks> {
    RISE_RTC_SYSTEM_BRIDGES_HOOKS.get()
}
