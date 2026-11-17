use std::sync::Arc;

use parking_lot::Mutex;

use crate::core::desktop_platform::av_audio_session::{
    AVAudioSession, AVAudioSessionInterruption, AVAudioSessionInterruptionOptions,
    AVAudioSessionInterruptionType,
};
use crate::core::desktop_platform::rise_app_events::RiseAppEvents;

#[test]
fn interruption_type_raw_values_match_ios() {
    assert_eq!(AVAudioSessionInterruptionType::Began.raw_value(), 1);
    assert_eq!(AVAudioSessionInterruptionType::Ended.raw_value(), 0);
    assert_eq!(
        AVAudioSessionInterruptionType::from_raw_value(1),
        Some(AVAudioSessionInterruptionType::Began)
    );
    assert_eq!(
        AVAudioSessionInterruptionType::from_raw_value(0),
        Some(AVAudioSessionInterruptionType::Ended)
    );
    assert_eq!(AVAudioSessionInterruptionType::from_raw_value(2), None);
}

#[test]
fn options_behave_as_an_option_set() {
    let empty = AVAudioSessionInterruptionOptions::empty();
    assert!(empty.is_empty());
    assert!(!empty.contains(AVAudioSessionInterruptionOptions::SHOULD_RESUME));
    assert_eq!(AVAudioSessionInterruptionOptions::SHOULD_RESUME.raw_value(), 1);

    let mut options = empty;
    options |= AVAudioSessionInterruptionOptions::SHOULD_RESUME;
    assert!(options.contains(AVAudioSessionInterruptionOptions::SHOULD_RESUME));
    assert_eq!(options, AVAudioSessionInterruptionOptions::from_raw_value(1));
    assert_eq!(AVAudioSessionInterruptionOptions::default(), empty);
}

#[test]
fn payload_reaches_observer_through_post_with() {
    let center = RiseAppEvents::new();
    let seen: Arc<Mutex<Option<AVAudioSessionInterruption>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&seen);
    let _subscription = center.observe(AVAudioSession::INTERRUPTION_NOTIFICATION, move |note| {
        *sink.lock() = note.payload::<AVAudioSessionInterruption>().copied();
    });

    let sent = AVAudioSessionInterruption {
        kind: AVAudioSessionInterruptionType::Ended,
        options: AVAudioSessionInterruptionOptions::SHOULD_RESUME,
    };
    center.post_with(AVAudioSession::INTERRUPTION_NOTIFICATION, sent);

    assert_eq!(*seen.lock(), Some(sent));
}
