use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::AUTH_SIGNED_IN_ON_INSTALL;

/// which auth screen onboarding opens
/// fresh install starts on registration, after a sign in it starts on sign in
/// stamped on sign in, outside the keychain
pub enum AuthEntryOrder {}

impl AuthEntryOrder {
    const KEY: &'static str = AUTH_SIGNED_IN_ON_INSTALL;

    pub fn opens_on_registration() -> bool {
        !UserDefaults::standard().bool(Self::KEY)
    }

    pub fn record_signed_in() {
        if !Self::opens_on_registration() {
            return;
        }
        UserDefaults::standard().set(Self::KEY, true);
    }
}
