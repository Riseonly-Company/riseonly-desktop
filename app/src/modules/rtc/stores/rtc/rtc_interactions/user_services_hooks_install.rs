use gpui::App;

use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services_hooks::{
    UserServicesHooks, UserServicesRtcHooks,
};

pub(crate) fn install_user_services_hooks() {
    UserServicesHooks::install_rtc(UserServicesRtcHooks {
        refresh_listener_volumes: Box::new(|cx: &mut App| {
            RTCInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.refresh_listener_volumes(cx));
        }),
    });
}
