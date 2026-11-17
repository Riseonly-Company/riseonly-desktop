use crate::modules::auth::services::account_state_reset_hooks::{AccountStateResetRtcHooks, set_account_state_reset_rtc_hooks};
use crate::modules::rtc::engine::rise_rtc_presentation::RiseRTCPresentation;
use crate::modules::rtc::media::rise_rtc_media_presentation::RiseRTCMediaPresentation;
use crate::modules::rtc::stores::group_voice::rise_rtc_group_voice_settings_stores::RTCGroupVoiceSettingsServicesStore;
use crate::modules::rtc::system::rise_rtc_call_system_coordinator::RiseRTCCallSystemCoordinator;

pub fn install_account_state_reset_rtc_hooks() {
    set_account_state_reset_rtc_hooks(AccountStateResetRtcHooks {
        authenticated_account_did_activate: |user_id, cx| {
            RiseRTCCallSystemCoordinator::shared(cx)
                .update(cx, |coordinator, cx| coordinator.authenticated_account_did_activate(user_id, cx));
        },
        fence_account_transition: |reason, cx| {
            RiseRTCCallSystemCoordinator::shared(cx).update(cx, |coordinator, cx| coordinator.fence_account_transition(reason, cx));
            RiseRTCPresentation::shared(cx).update(cx, |presentation, cx| presentation.deactivate(cx));
            RiseRTCMediaPresentation::shared(cx).update(cx, |presentation, cx| presentation.reset(cx));
        },
        reset_account_state: |cx| {
            RTCGroupVoiceSettingsServicesStore::shared(cx).update(cx, |store, cx| store.reset(None, cx));
        },
    });
}
