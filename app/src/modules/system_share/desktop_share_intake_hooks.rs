use gpui::{AnyWindowHandle, App};

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::share_extension::entry::share_item_loader::ShareExtensionItem;

/// what the share intake needs from the shell window
pub struct DesktopShareIntakeHooks {
    pub present_shared_items: fn(AnyWindowHandle, Vec<ShareExtensionItem>, &mut App),
}

pub(crate) static DESKTOP_SHARE_INTAKE_HOOKS: FeatureHookSlot<DesktopShareIntakeHooks> =
    FeatureHookSlot::new("DESKTOP_SHARE_INTAKE_HOOKS");

pub fn set_desktop_share_intake_hooks(hooks: DesktopShareIntakeHooks) {
    let _ = DESKTOP_SHARE_INTAKE_HOOKS.set(hooks);
}

pub fn desktop_share_intake_hooks() -> Option<&'static DesktopShareIntakeHooks> {
    DESKTOP_SHARE_INTAKE_HOOKS.get()
}
