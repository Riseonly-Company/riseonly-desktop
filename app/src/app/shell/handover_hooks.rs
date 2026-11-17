use std::path::PathBuf;

use gpui::{AnyWindowHandle, App};
use rise_core::foundation::foundation_url::Url;

use crate::core::feature_hooks::FeatureHookSlot;

/// what the handover needs from the shell window and the share intake
pub struct HandoverHooks {
    pub is_shell: fn(AnyWindowHandle) -> bool,
    pub open_shell: fn(&mut App) -> Option<AnyWindowHandle>,
    pub open_urls: fn(AnyWindowHandle, &[Url], &mut App),
    pub open_url: fn(AnyWindowHandle, Url, &mut App) -> Result<(), String>,
    pub deliver_shared: fn(&[PathBuf], &[String], &mut App),
}

pub(crate) static HANDOVER_HOOKS: FeatureHookSlot<HandoverHooks> = FeatureHookSlot::new("HANDOVER_HOOKS");

pub fn set_handover_hooks(hooks: HandoverHooks) {
    let _ = HANDOVER_HOOKS.set(hooks);
}

pub fn handover_hooks() -> Option<&'static HandoverHooks> {
    HANDOVER_HOOKS.get()
}
