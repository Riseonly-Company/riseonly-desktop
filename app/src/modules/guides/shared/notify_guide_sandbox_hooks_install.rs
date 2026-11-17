use gpui::App;

use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::notify::stores::notify_actions::notify_profile_settings_wire_types::{
    ProfileNotificationScope, ProfileNotificationSettingsModel,
};
use crate::modules::notify::stores::notify_services::notify_guide_sandbox_hooks::{
    NotifyGuideSandboxHooks, set_notify_guide_sandbox_hooks,
};

pub fn install_notify_guide_sandbox_hooks() {
    let _ = set_notify_guide_sandbox_hooks(NotifyGuideSandboxHooks {
        owns_person_notifications,
        person_notifications,
        set_person_notification,
        set_all_person_notifications,
    });
}

fn owns_person_notifications(target_user_id: &str, cx: &mut App) -> bool {
    GuideSandbox::shared(cx).read(cx).owns_person_notifications(target_user_id)
}

fn person_notifications(target_user_id: &str, cx: &mut App) -> Option<ProfileNotificationSettingsModel> {
    GuideSandbox::shared(cx).read(cx).person_notifications(target_user_id)
}

fn set_person_notification(scope: ProfileNotificationScope, enabled: bool, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_person_notification(scope, enabled, cx));
}

fn set_all_person_notifications(enabled: bool, cx: &mut App) {
    GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.set_all_person_notifications(enabled, cx));
}
