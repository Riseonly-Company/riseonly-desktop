use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::STORY_STEALTH_ENABLED;

/// stealth story viewing, watch without appearing in the viewer list
/// device preference not a server setting
/// free accounts refused
pub enum StoryStealthPreference {}

impl StoryStealthPreference {
    pub fn is_enabled() -> bool {
        UserDefaults::standard().bool(STORY_STEALTH_ENABLED)
    }

    pub fn set_is_enabled(new_value: bool) {
        UserDefaults::standard().set(STORY_STEALTH_ENABLED, new_value);
    }
}
