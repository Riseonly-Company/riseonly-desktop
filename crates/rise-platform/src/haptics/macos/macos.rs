use objc2_app_kit::{
    NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime,
    NSHapticFeedbackPerformer as _,
};

use crate::haptics::*;

pub(crate) struct NativeHaptics;

pub(crate) fn native_pattern(pattern: HapticPattern) -> NSHapticFeedbackPattern {
    match pattern {
        HapticPattern::Generic => NSHapticFeedbackPattern::Generic,
        HapticPattern::Alignment => NSHapticFeedbackPattern::Alignment,
        HapticPattern::LevelChange => NSHapticFeedbackPattern::LevelChange,
    }
}

impl HapticPerformer for NativeHaptics {
    fn available(&self) -> Result<(), HapticsUnavailable> {
        Ok(())
    }

    fn perform(&self, pattern: HapticPattern) -> Result<(), HapticsUnavailable> {
        NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(
            native_pattern(pattern),
            NSHapticFeedbackPerformanceTime::Now,
        );
        Ok(())
    }
}
