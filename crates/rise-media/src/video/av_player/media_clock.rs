use std::collections::VecDeque;
use std::time::Duration;

pub const MIN_TEMPO: f64 = 0.5;
pub const MAX_TEMPO: f64 = 100.0;

pub const DROP_LATE_AFTER: Duration = Duration::from_millis(80);

pub const LONGEST_PICTURE_WAIT: Duration = Duration::from_secs(1);

pub const AUDIO_CLOCK_POLL: Duration = Duration::from_millis(10);

/// tempo rate plays at, none when the rate stops the player (zero or negative or NaN)
pub fn tempo_of_rate(rate: f32) -> Option<f64> {
    if !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    Some(f64::from(rate).clamp(MIN_TEMPO, MAX_TEMPO))
}

/// whether two tempos play the same, so a rate set again doesnt rebuild the audio graph
pub fn same_tempo(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

/// sample rate the abuffer source of the graph declares for a source of source_rate
/// with atempo it is declared as is
/// without it the source is declared tempo times faster and resampled to the device rate
pub fn declared_source_rate(source_rate: u32, tempo: f64, has_atempo: bool) -> u32 {
    if has_atempo || same_tempo(tempo, 1.0) {
        return source_rate;
    }
    let declared = (f64::from(source_rate) * tempo).round();
    if declared < 1.0 {
        1
    } else if declared > f64::from(u32::MAX) {
        u32::MAX
    } else {
        declared as u32
    }
}

/// filters between the abuffer source and the sink
/// atempo when the tempo is not one and the prefix has it
/// then libswresample to the device rate and interleaved float layout
pub fn audio_filter_spec(tempo: f64, has_atempo: bool, device_rate: u32, device_channels: u16) -> String {
    let tempo_filter = if has_atempo && !same_tempo(tempo, 1.0) { format!("atempo={tempo},") } else { String::new() };
    format!(
        "{tempo_filter}aresample={device_rate},aformat=sample_fmts=flt:sample_rates={device_rate}:channel_layouts={device_channels}c"
    )
}

/// one run of samples pushed at one tempo from one media origin
#[derive(Clone, Copy, PartialEq, Debug)]
struct TimelineSegment {
    first_frame: u64,
    origin_us: i64,
    tempo: f64,
}

/// media time of every sample frame pushed to the device ring
/// a new segment starts when a graph starts (new item or new tempo)
/// frames of one segment advance media time by tempo device frames per frame
#[derive(Clone, PartialEq, Debug)]
pub struct AudioTimeline {
    device_rate: u32,
    pushed_frames: u64,
    segments: VecDeque<TimelineSegment>,
}

impl AudioTimeline {
    pub fn new(device_rate: u32) -> Self {
        Self { device_rate: device_rate.max(1), pushed_frames: 0, segments: VecDeque::new() }
    }

    /// frames the graph pushes from now on start at origin_us and play at tempo
    pub fn begin_segment(&mut self, origin_us: i64, tempo: f64) {
        let segment = TimelineSegment { first_frame: self.pushed_frames, origin_us, tempo };
        if self.segments.back().is_some_and(|last| last.first_frame == self.pushed_frames) {
            self.segments.pop_back();
        }
        self.segments.push_back(segment);
    }

    pub fn has_segment(&self) -> bool {
        !self.segments.is_empty()
    }

    /// frames more frames went into the ring
    pub fn pushed(&mut self, frames: u64) {
        self.pushed_frames = self.pushed_frames.saturating_add(frames);
    }

    pub fn pushed_frames(&self) -> u64 {
        self.pushed_frames
    }

    /// media time after the first played_frames pushed frames, none before any segment
    /// segments wholly before the played frames are forgotten
    pub fn media_us_at(&mut self, played_frames: u64) -> Option<i64> {
        while self.segments.len() > 1 && self.segments[1].first_frame <= played_frames {
            self.segments.pop_front();
        }
        let segment = self.segments.front()?;
        let frames = played_frames.saturating_sub(segment.first_frame);
        let media = frames as f64 / f64::from(self.device_rate) * segment.tempo * 1_000_000.0;
        Some(segment.origin_us.saturating_add(media.round() as i64))
    }

    /// media time after every pushed frame, the end of what the device will play
    pub fn end_us(&self) -> Option<i64> {
        let segment = self.segments.back()?;
        let frames = self.pushed_frames.saturating_sub(segment.first_frame);
        let media = frames as f64 / f64::from(self.device_rate) * segment.tempo * 1_000_000.0;
        Some(segment.origin_us.saturating_add(media.round() as i64))
    }
}

/// leading clock when there is no sound or after it ended
/// media time advances with the monotonic host clock at the tempo while it runs
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HostMediaClock {
    anchor_host: Duration,
    anchor_us: i64,
    tempo: f64,
    running: bool,
}

impl HostMediaClock {
    /// stopped clock at media_us
    pub fn at(media_us: i64, now: Duration) -> Self {
        Self { anchor_host: now, anchor_us: media_us, tempo: 1.0, running: false }
    }

    /// media time at host time now
    pub fn media_us(&self, now: Duration) -> i64 {
        if !self.running {
            return self.anchor_us;
        }
        let elapsed = now.saturating_sub(self.anchor_host).as_secs_f64() * self.tempo * 1_000_000.0;
        self.anchor_us.saturating_add(elapsed.round() as i64)
    }

    pub fn run(&mut self, tempo: f64, now: Duration) {
        self.anchor_us = self.media_us(now);
        self.anchor_host = now;
        self.tempo = tempo;
        self.running = true;
    }

    /// stands still at the media time of now
    pub fn stop(&mut self, now: Duration) {
        self.anchor_us = self.media_us(now);
        self.anchor_host = now;
        self.running = false;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
}

/// what to do with a decoded picture, given the leading clock
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaPictureDue {
    Show,
    Drop,
    After(Duration),
}

/// says when the picture at pts_us should show against the leading clock
/// with audio leading the wait is capped at the audio poll interval
pub fn media_picture_due(pts_us: i64, clock_us: i64, tempo: f64, audio_leads: bool) -> MediaPictureDue {
    let ahead_us = pts_us.saturating_sub(clock_us);
    if ahead_us <= 0 {
        let late = Duration::from_micros(ahead_us.unsigned_abs());
        return if late > DROP_LATE_AFTER { MediaPictureDue::Drop } else { MediaPictureDue::Show };
    }
    let tempo = if tempo.is_finite() && tempo > 0.0 { tempo } else { 1.0 };
    let wait = Duration::from_secs_f64(ahead_us as f64 / 1_000_000.0 / tempo);
    if wait > LONGEST_PICTURE_WAIT {
        return MediaPictureDue::Show;
    }
    if audio_leads && wait > AUDIO_CLOCK_POLL {
        return MediaPictureDue::After(AUDIO_CLOCK_POLL);
    }
    MediaPictureDue::After(wait)
}

/// current time in seconds for a media time, clamped between zero and the known duration
pub fn item_seconds(media_us: i64, duration_us: Option<i64>) -> f64 {
    let clamped = match duration_us {
        Some(duration) if duration > 0 => media_us.clamp(0, duration),
        _ => media_us.max(0),
    };
    clamped as f64 / 1_000_000.0
}

/// container duration in seconds, none while its unknown or not positive
pub fn duration_seconds(duration_us: Option<i64>) -> Option<f64> {
    duration_us.filter(|duration| *duration > 0).map(|duration| duration as f64 / 1_000_000.0)
}
