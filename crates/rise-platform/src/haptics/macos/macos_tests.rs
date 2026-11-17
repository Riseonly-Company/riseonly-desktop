use objc2_app_kit::NSHapticFeedbackPattern;

use super::{native_pattern, NativeHaptics};
use crate::haptics::*;

#[test]
fn every_pattern_maps_to_its_appkit_pattern() {
    assert_eq!(native_pattern(HapticPattern::Generic), NSHapticFeedbackPattern::Generic);
    assert_eq!(native_pattern(HapticPattern::Alignment), NSHapticFeedbackPattern::Alignment);
    assert_eq!(
        native_pattern(HapticPattern::LevelChange),
        NSHapticFeedbackPattern::LevelChange
    );
}

#[test]
fn the_default_performer_takes_every_pattern() {
    assert_eq!(NativeHaptics.available(), Ok(()));
    for pattern in [
        HapticPattern::Generic,
        HapticPattern::Alignment,
        HapticPattern::LevelChange,
    ] {
        assert_eq!(NativeHaptics.perform(pattern), Ok(()));
    }
}
