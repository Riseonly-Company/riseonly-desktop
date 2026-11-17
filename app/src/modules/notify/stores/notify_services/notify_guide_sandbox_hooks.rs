use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::notify::stores::notify_actions::notify_profile_settings_wire_types::{
    ProfileNotificationScope, ProfileNotificationSettingsModel,
};

/// what the notify stores ask the guide sandbox
pub struct NotifyGuideSandboxHooks {
    pub owns_person_notifications: fn(target_user_id: &str, cx: &mut App) -> bool,
    pub person_notifications: fn(target_user_id: &str, cx: &mut App) -> Option<ProfileNotificationSettingsModel>,
    pub set_person_notification: fn(scope: ProfileNotificationScope, enabled: bool, cx: &mut App),
    pub set_all_person_notifications: fn(enabled: bool, cx: &mut App),
}

pub(crate) static NOTIFY_GUIDE_SANDBOX_HOOKS: FeatureHookSlot<NotifyGuideSandboxHooks> =
    FeatureHookSlot::new("NOTIFY_GUIDE_SANDBOX_HOOKS");

static UNINSTALLED: NotifyGuideSandboxHooks = NotifyGuideSandboxHooks {
    owns_person_notifications: |_, _| false,
    person_notifications: |_, _| None,
    set_person_notification: |_, _, _| {},
    set_all_person_notifications: |_, _| {},
};

pub fn set_notify_guide_sandbox_hooks(hooks: NotifyGuideSandboxHooks) -> Result<(), NotifyGuideSandboxHooks> {
    NOTIFY_GUIDE_SANDBOX_HOOKS.set(hooks)
}

pub fn notify_guide_sandbox_hooks() -> &'static NotifyGuideSandboxHooks {
    NOTIFY_GUIDE_SANDBOX_HOOKS.get().unwrap_or(&UNINSTALLED)
}
