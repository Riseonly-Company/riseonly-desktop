use super::audio_capture::{AacQuality, AacSettings, AudioCaptureError};

pub const FFMPEG_AAC_ENCODER: &str = "aac";
pub const FFMPEG_M4A_MUXER: &str = "ipod";
pub const AAC_FRAME_LENGTH: usize = 1024;
pub const FFMPEG_CONFIGURE_HINT: &str = "--enable-encoder=aac --enable-muxer=ipod";

pub const MEDIUM_BIT_RATE_PER_CHANNEL: u32 = 64_000;
pub const HIGH_BIT_RATE_PER_CHANNEL: u32 = 96_000;

/// what the encoder context opens with, rate and channels and bit rate and frame length
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FfmpegAacPlan {
    pub sample_rate: u32,
    pub channels: u16,
    pub bit_rate: u32,
}

impl FfmpegAacPlan {
    pub fn for_settings(settings: &AacSettings) -> Result<Self, AudioCaptureError> {
        settings.validate()?;
        Ok(Self {
            sample_rate: settings.sample_rate,
            channels: settings.channels,
            bit_rate: ffmpeg_bit_rate(settings),
        })
    }
}

/// the take own rate if it names one (32 kbit/s for a voice message)
/// otherwise the quality rate times the channel count
pub fn ffmpeg_bit_rate(settings: &AacSettings) -> u32 {
    match settings.bit_rate {
        Some(bits_per_second) => bits_per_second,
        None => {
            let per_channel = match settings.quality {
                AacQuality::Medium => MEDIUM_BIT_RATE_PER_CHANNEL,
                AacQuality::High => HIGH_BIT_RATE_PER_CHANNEL,
            };
            per_channel * u32::from(settings.channels)
        }
    }
}

/// why a build cant record, none when both encoder and muxer are linked in
pub fn missing_component_reason(has_encoder: bool, has_muxer: bool) -> Option<String> {
    let missing = match (has_encoder, has_muxer) {
        (true, true) => return None,
        (false, true) => format!("the \"{FFMPEG_AAC_ENCODER}\" encoder"),
        (true, false) => format!("the \"{FFMPEG_M4A_MUXER}\" muxer"),
        (false, false) => format!("the \"{FFMPEG_AAC_ENCODER}\" encoder and the \"{FFMPEG_M4A_MUXER}\" muxer"),
    };
    Some(format!(
        "the LGPL FFmpeg build has no {missing} (scripts/build-ffmpeg-lgpl.sh {FFMPEG_CONFIGURE_HINT})"
    ))
}

/// reason for a build compiled without the ffmpeg feature
pub fn feature_off_reason() -> String {
    "this build is compiled without the ffmpeg feature, the only AAC encoder of the Linux route"
        .to_owned()
}

/// one encoder frame, frames samples per plane and plane c holds channel c
#[derive(Clone, PartialEq, Debug)]
pub struct PlanarFrame {
    pub planes: Vec<Vec<f32>>,
    pub frames: usize,
    pub pts: i64,
}

/// interleaved float pcm in and planar frames of exactly frame_length out (the last may be shorter)
/// out of range samples clip and nan becomes silence
#[derive(Clone, Debug)]
pub struct PlanarFrames {
    channels: usize,
    frame_length: usize,
    pending: Vec<Vec<f32>>,
    emitted_frames: u64,
}

impl PlanarFrames {
    /// frame_length 0 (encoder takes any size) falls back to one aac frame
    pub fn new(channels: u16, frame_length: usize) -> Self {
        let channels = usize::from(channels.max(1));
        let frame_length = if frame_length == 0 { AAC_FRAME_LENGTH } else { frame_length };
        Self {
            channels,
            frame_length,
            pending: vec![Vec::with_capacity(frame_length * 2); channels],
            emitted_frames: 0,
        }
    }

    pub const fn frame_length(&self) -> usize {
        self.frame_length
    }

    /// frames handed out so far
    pub const fn emitted_frames(&self) -> u64 {
        self.emitted_frames
    }

    /// frames waiting for a full encoder frame
    pub fn pending_frames(&self) -> usize {
        self.pending.first().map_or(0, Vec::len)
    }

    /// splits interleaved samples into channel planes, a trailing partial frame is dropped
    pub fn push(&mut self, interleaved: &[f32]) {
        for frame in interleaved.chunks_exact(self.channels) {
            for (plane, sample) in self.pending.iter_mut().zip(frame) {
                plane.push(clean(*sample));
            }
        }
    }

    /// next full frame if one is buffered
    pub fn pop_full(&mut self) -> Option<PlanarFrame> {
        if self.pending_frames() < self.frame_length {
            return None;
        }
        Some(self.take(self.frame_length))
    }

    /// at the end of a take, the shorter last frame
    /// or one silent frame if nothing was handed out (a container with no samples cant play)
    pub fn pop_last(&mut self) -> Option<PlanarFrame> {
        let pending = self.pending_frames();
        if pending > 0 {
            return Some(self.take(pending));
        }
        if self.emitted_frames == 0 {
            for plane in &mut self.pending {
                plane.resize(self.frame_length, 0.0);
            }
            return Some(self.take(self.frame_length));
        }
        None
    }

    fn take(&mut self, frames: usize) -> PlanarFrame {
        let planes = self.pending.iter_mut().map(|plane| plane.drain(..frames).collect()).collect();
        let pts = i64::try_from(self.emitted_frames).unwrap_or(i64::MAX);
        self.emitted_frames += frames as u64;
        PlanarFrame { planes, frames, pts }
    }
}

fn clean(sample: f32) -> f32 {
    if sample.is_nan() { 0.0 } else { sample.clamp(-1.0, 1.0) }
}
