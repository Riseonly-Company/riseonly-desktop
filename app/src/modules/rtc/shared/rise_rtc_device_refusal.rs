use gpui::App;
use rise_i18n::tr;
use rise_ui::native_confirm_alert::NativeConfirmAlert;

use crate::modules::rtc::engine::rise_rtc_engine_models::RiseRTCStateError;

pub enum RiseRTCDeviceRefusal {}

impl RiseRTCDeviceRefusal {
    pub const CAMERA: &'static str = "rtc_camera_permission_denied";
    pub const MICROPHONE: &'static str = "rtc_microphone_permission_denied";
    pub const MICROPHONE_RESTRICTED: &'static str = "rtc_microphone_permission_restricted";

    /// shows the settings alert and returns true when error is a refused device
    pub fn present_settings_alert(error: &anyhow::Error, cx: &mut App) -> bool {
        let Some(RiseRTCStateError::Remote(code)) = error.downcast_ref::<RiseRTCStateError>() else {
            return false;
        };
        match code.as_str() {
            Self::CAMERA => NativeConfirmAlert::present_open_settings(
                &tr("chat_camera_access_denied_title"),
                &tr("rtc_camera_access_denied_message"),
                cx,
            ),
            Self::MICROPHONE | Self::MICROPHONE_RESTRICTED => NativeConfirmAlert::present_open_settings(
                &tr("chat_mic_access_denied_title"),
                &tr("rtc_mic_access_denied_message"),
                cx,
            ),
            _ => return false,
        }
        true
    }
}
