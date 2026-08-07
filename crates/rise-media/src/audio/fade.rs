//! Volume ramps, and the duck that a manual resume has to be able to cancel.
//!
//! Ported from `ROMusicFilePlayer`'s fade methods. Three of them exist because
//! the product ducks music under other audio — a voice message, a video in the
//! feed, a call — and the fourth exists because of a race the reference
//! documents in a comment: the duck schedules a pause for when the ramp
//! finishes, and if the user hits play in the meantime, that pending pause must
//! not land. `restore_full_volume` is what cancels it.
//!
//! Applied per FRAME, not per sample. Ramping per sample walks the gain between
//! the left and right channel of the same instant, which is a moving stereo
//! image rather than a volume change.

/// What the mixer should do once the ramp reaches its target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OnComplete {
    /// Stop the device. The duck's whole purpose: silence first, then pause, so
    /// the transition is not a cut.
    Pause,
}

#[derive(Clone, Copy, Debug)]
pub struct VolumeRamp {
    current: f32,
    target: f32,
    frames_remaining: u64,
    on_complete: Option<OnComplete>,
}

impl VolumeRamp {
    pub const fn full() -> Self {
        Self {
            current: 1.0,
            target: 1.0,
            frames_remaining: 0,
            on_complete: None,
        }
    }

    pub const fn silent() -> Self {
        Self {
            current: 0.0,
            target: 0.0,
            frames_remaining: 0,
            on_complete: None,
        }
    }

    pub const fn gain(&self) -> f32 {
        self.current
    }

    pub const fn is_ramping(&self) -> bool {
        self.frames_remaining > 0
    }

    pub const fn has_pending_pause(&self) -> bool {
        matches!(self.on_complete, Some(OnComplete::Pause))
    }

    pub fn set(&mut self, target: f32, over_frames: u64) {
        self.target = target.clamp(0.0, 1.0);
        self.frames_remaining = over_frames;
        self.on_complete = None;

        if over_frames == 0 {
            self.current = self.target;
        }
    }

    /// Ramp to silence and then stop.
    pub fn duck_and_pause(&mut self, over_frames: u64) {
        self.set(0.0, over_frames);
        self.on_complete = Some(OnComplete::Pause);
    }

    /// Start from silence and come back up. The caller resumes the device
    /// first; this is only the gain.
    pub fn fade_in(&mut self, over_frames: u64) {
        self.current = 0.0;
        self.set(1.0, over_frames);
    }

    /// Back to full immediately, cancelling any pending pause.
    ///
    /// This is the cancellation the reference needs: without it, a user who
    /// presses play during a duck gets full volume and then, a fraction of a
    /// second later, a pause from the ramp that was already in flight.
    pub fn restore_full_volume(&mut self) {
        self.current = 1.0;
        self.target = 1.0;
        self.frames_remaining = 0;
        self.on_complete = None;
    }

    /// Apply the ramp to one interleaved block, in place.
    pub fn apply(&mut self, block: &mut [f32], channels: u16) -> Option<OnComplete> {
        let channels = channels.max(1) as usize;
        let frames = block.len() / channels;
        let mut completed = None;

        for frame in 0..frames {
            if self.frames_remaining > 0 {
                let step = (self.target - self.current) / self.frames_remaining as f32;
                self.current += step;
                self.frames_remaining -= 1;

                if self.frames_remaining == 0 {
                    self.current = self.target;
                    completed = self.on_complete.take();
                }
            }

            let start = frame * channels;
            for sample in &mut block[start..start + channels] {
                *sample *= self.current;
            }
        }

        completed
    }
}

impl Default for VolumeRamp {
    fn default() -> Self {
        Self::full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(frames: usize, channels: usize) -> Vec<f32> {
        vec![1.0; frames * channels]
    }

    #[test]
    fn full_volume_leaves_the_audio_untouched() {
        let mut ramp = VolumeRamp::full();
        let mut samples = block(64, 2);

        assert_eq!(ramp.apply(&mut samples, 2), None);

        assert!(samples.iter().all(|sample| *sample == 1.0));
    }

    #[test]
    fn a_ramp_reaches_its_target_exactly_and_then_stops_moving() {
        let mut ramp = VolumeRamp::full();
        ramp.set(0.25, 100);

        let mut samples = block(100, 2);
        ramp.apply(&mut samples, 2);

        assert_eq!(ramp.gain(), 0.25);
        assert!(!ramp.is_ramping());
        assert!((samples[samples.len() - 1] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn both_channels_of_one_frame_get_the_same_gain() {
        let mut ramp = VolumeRamp::full();
        ramp.set(0.0, 8);
        let mut samples = block(8, 2);

        ramp.apply(&mut samples, 2);

        for frame in samples.chunks(2) {
            assert_eq!(
                frame[0], frame[1],
                "ramping per sample moves the stereo image instead of the volume"
            );
        }
    }

    #[test]
    fn a_duck_reports_its_pause_only_once_the_audio_has_reached_silence() {
        let mut ramp = VolumeRamp::full();
        ramp.duck_and_pause(64);

        let mut first = block(32, 2);
        assert_eq!(ramp.apply(&mut first, 2), None);
        assert!(ramp.gain() > 0.0);

        let mut second = block(32, 2);
        assert_eq!(ramp.apply(&mut second, 2), Some(OnComplete::Pause));
        assert_eq!(ramp.gain(), 0.0);
    }

    #[test]
    fn resuming_during_a_duck_cancels_the_pause_that_was_already_in_flight() {
        let mut ramp = VolumeRamp::full();
        ramp.duck_and_pause(64);
        let mut samples = block(32, 2);
        ramp.apply(&mut samples, 2);

        ramp.restore_full_volume();

        let mut rest = block(64, 2);
        assert_eq!(
            ramp.apply(&mut rest, 2),
            None,
            "the pause from the cancelled duck would stop playback the user just started"
        );
        assert_eq!(ramp.gain(), 1.0);
        assert!(!ramp.has_pending_pause());
    }

    #[test]
    fn a_fade_in_starts_from_silence_rather_than_from_wherever_it_was() {
        let mut ramp = VolumeRamp::full();
        ramp.fade_in(32);

        let mut samples = block(1, 2);
        ramp.apply(&mut samples, 2);

        assert!(samples[0] < 0.1);
        assert!(ramp.is_ramping());
    }

    #[test]
    fn an_instant_change_takes_effect_on_the_next_sample() {
        let mut ramp = VolumeRamp::full();
        ramp.set(0.5, 0);

        let mut samples = block(4, 2);
        ramp.apply(&mut samples, 2);

        assert!(samples.iter().all(|sample| (*sample - 0.5).abs() < 1e-6));
    }

    #[test]
    fn a_gain_above_one_or_below_zero_is_clamped_rather_than_clipping_the_output() {
        let mut ramp = VolumeRamp::full();

        ramp.set(4.0, 0);
        assert_eq!(ramp.gain(), 1.0);

        ramp.set(-1.0, 0);
        assert_eq!(ramp.gain(), 0.0);
    }

    #[test]
    fn mono_is_ramped_as_one_channel_per_frame() {
        let mut ramp = VolumeRamp::full();
        ramp.set(0.0, 4);
        let mut samples = block(4, 1);

        assert_eq!(ramp.apply(&mut samples, 1), None);

        assert_eq!(ramp.gain(), 0.0);
    }
}
