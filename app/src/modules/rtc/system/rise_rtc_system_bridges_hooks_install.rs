use crate::modules::notify::stores::notify_actions::rise_rtc_system_bridges_hooks::{
    RiseRTCSystemBridgesHooks, set_rise_rtc_system_bridges_hooks,
};
use crate::modules::rtc::system::rise_rtc_system_bridges::RiseRTCSystemVoIPAPNSEnvironment;

pub(crate) fn install_rise_rtc_system_bridges_hooks() {
    set_rise_rtc_system_bridges_hooks(RiseRTCSystemBridgesHooks {
        current: || RiseRTCSystemVoIPAPNSEnvironment::current().raw_value().to_owned(),
    });
}
