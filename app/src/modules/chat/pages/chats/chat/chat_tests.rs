use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::ChatBackdropMotionPolicy;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperMotionPreferencePolicy;
use crate::modules::chat::pages::chats::chat::chat_screen::chat_screen_support::ChatHeaderRefreshSchedule;

#[test]
fn wallpaper_motion_defaults_off_and_migrates_legacy_opt_in_once() {
    assert!(!ChatWallpaperMotionPreferencePolicy::DEFAULT_ENABLED);

    let migrated = ChatWallpaperMotionPreferencePolicy::resolve(Some(true), false);
    assert!(!migrated.is_enabled);
    assert!(migrated.should_persist);

    let explicit_opt_in = ChatWallpaperMotionPreferencePolicy::resolve(Some(true), true);
    assert!(explicit_opt_in.is_enabled);
    assert!(!explicit_opt_in.should_persist);
}

#[test]
fn wallpaper_transition_respects_all_energy_and_accessibility_gates() {
    assert!(ChatBackdropMotionPolicy::TRANSITION_DURATION <= 0.25);
    assert!(ChatBackdropMotionPolicy::allows_transition(true, false, false));
    assert!(!ChatBackdropMotionPolicy::allows_transition(false, false, false));
    assert!(!ChatBackdropMotionPolicy::allows_transition(true, true, false));
    assert!(!ChatBackdropMotionPolicy::allows_transition(true, false, true));
}

#[test]
fn chat_header_schedules_only_the_next_meaningful_expiry() {
    let typing_expiry =
        ChatHeaderRefreshSchedule::next_refresh_timestamp(&[100.0, 102.0], 6.0, true, None, 104.0, None);
    assert_eq!(typing_expiry, Some(106.0));

    let no_refresh =
        ChatHeaderRefreshSchedule::next_refresh_timestamp(&[90.0], 6.0, true, Some(1_000_000), 104.0, None);
    assert_eq!(no_refresh, None);

    let relative_status_refresh =
        ChatHeaderRefreshSchedule::next_refresh_timestamp(&[], 6.0, false, Some(1_000_000), 1_030.0, None);
    assert_eq!(relative_status_refresh, Some(1_060.05));
}

#[test]
fn chat_header_reads_presence_timestamps_as_milliseconds() {
    assert_eq!(ChatHeaderRefreshSchedule::normalized_timestamp(Some(1_725_000_000_000)), Some(1_725_000_000.0));
    assert_eq!(ChatHeaderRefreshSchedule::normalized_timestamp(Some(1_500)), Some(1.5));
    assert_eq!(ChatHeaderRefreshSchedule::normalized_timestamp(None), None);
}
