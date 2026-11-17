use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::BUG_REPORT_SHAKE_ENABLED;

/// whether shaking the device opens the report sheet
/// device local not per account and out of canon, no server side and no android twin yet
/// defaults to on
pub enum BugReportShakePreference {}

impl BugReportShakePreference {
    pub fn is_enabled() -> bool {
        let defaults = UserDefaults::standard();
        if defaults.object(BUG_REPORT_SHAKE_ENABLED).is_none() {
            return true;
        }
        defaults.bool(BUG_REPORT_SHAKE_ENABLED)
    }

    pub fn set_is_enabled(new_value: bool) {
        UserDefaults::standard().set(BUG_REPORT_SHAKE_ENABLED, new_value);
    }
}
