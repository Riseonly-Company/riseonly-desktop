use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::app_badge::app_badge_hooks::{AppBadgeHooks, set_app_badge_hooks};
use rise_platform::app_info::identity_for;

use crate::core::config::app_environment::AppEnvironment;

pub(crate) fn install_app_badge_hooks() {
    set_app_badge_hooks(AppBadgeHooks {
        spawn: |work| drop(RiseAccountRuntime::current().spawn(work)),
        bundle_identifier: || {
            identity_for(AppEnvironment::compiled().as_str()).map(|identity| identity.bundle_identifier)
        },
    });
}
