use gpui::App;

use crate::core::push_notifications::rise_app_icon_badge::RiseAppIconBadge;
use crate::core::push_notifications::rise_app_icon_badge_hooks::RiseAppIconBadgeHooks;

pub(crate) fn install_rise_app_icon_badge_hooks() {
    RiseAppIconBadgeHooks::install(RiseAppIconBadgeHooks {
        sync: Box::new(|force: bool, cx: &mut App| RiseAppIconBadge::sync(force, cx)),
    });
}
