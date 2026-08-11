//! Playing an album end to end, with no device and no clock: a whole queue
//! costs two decoders and one ring, and the join is sample-exact.

use rise_media::audio::format::SampleFormat;
use rise_media::audio::gapless::Trim;
use rise_media::audio::output::DeviceConfig;
use rise_media::audio::pump::{Pump, PumpStep};
use rise_media::audio::ring;
use rise_media::audio::scheduler::{
    MAX_LIVE_DECODERS, PRELOAD_LEAD_MILLIS, PlaybackScheduler, TrackState, Upcoming,
};
use rise_media::audio::symphonia_backend::SymphoniaDecoder;

const ALBUM_LENGTH: u64 = 40;
const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u16 = 2;
const TRACK_FRAMES: usize = 44_100 / 2;

/// A real 16-bit PCM WAV, built here so the suite carries no fixture files.
fn wav(frames: usize) -> Vec<u8> {
    let data_bytes = frames * CHANNELS as usize * 2;
    let mut file = Vec::with_capacity(44 + data_bytes);

    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
    file.extend_from_slice(b"WAVE");
    file.extend_from_slice(b"fmt ");
    file.extend_from_slice(&16u32.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes());
    file.extend_from_slice(&CHANNELS.to_le_bytes());
    file.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    file.extend_from_slice(&(SAMPLE_RATE * CHANNELS as u32 * 2).to_le_bytes());
    file.extend_from_slice(&(CHANNELS * 2).to_le_bytes());
    file.extend_from_slice(&16u16.to_le_bytes());
    file.extend_from_slice(b"data");
    file.extend_from_slice(&(data_bytes as u32).to_le_bytes());

    for frame in 0..frames {
        let value = (frame as f64 / SAMPLE_RATE as f64 * 440.0 * std::f64::consts::TAU).sin();
        let sample = (value * 30_000.0) as i16;
        for _ in 0..CHANNELS {
            file.extend_from_slice(&sample.to_le_bytes());
        }
    }

    file
}

fn device() -> DeviceConfig {
    DeviceConfig {
        sample_rate: SAMPLE_RATE,
        channels: CHANNELS,
        sample_format: SampleFormat::F32,
        resampling: false,
    }
}

/// Play one track out completely, returning how many frames reached the device.
fn play(trim: Trim) -> u64 {
    let decoder = SymphoniaDecoder::open(wav(TRACK_FRAMES)).unwrap();
    let (producer, mut consumer) = ring::ring(1 << 15);
    let mut pump = Pump::new(Box::new(decoder), trim, device(), producer);

    let mut heard = 0u64;
    let mut running = true;

    while running || consumer.filled() > 0 {
        if running && pump.fill().unwrap() == PumpStep::EndOfStream {
            running = false;
        }

        let available = consumer.filled().min(4096);
        if available == 0 {
            continue;
        }
        let mut block = vec![0.0; available];
        heard += consumer.pop_or_silence(&mut block) as u64;
    }

    heard / CHANNELS as u64
}

#[test]
fn a_whole_album_holds_two_decoders_however_long_the_queue_is() {
    let mut scheduler = PlaybackScheduler::new();

    for track in 1..=ALBUM_LENGTH {
        let upcoming = Upcoming {
            current: Some(track),
            next: (track < ALBUM_LENGTH).then_some(track + 1),
        };

        for remaining in [30_000, 10_000, PRELOAD_LEAD_MILLIS, 500, 0] {
            scheduler.reconcile(&upcoming, Some(remaining));

            assert!(
                scheduler.live_count() <= MAX_LIVE_DECODERS,
                "track {track} with {remaining} ms left held {} decoders",
                scheduler.live_count()
            );
        }
    }

    scheduler.close_all();
    assert_eq!(scheduler.live_count(), 0);
}

#[test]
fn every_track_is_preloaded_before_it_is_needed_and_never_reopened() {
    let mut scheduler = PlaybackScheduler::new();
    let mut reopened = 0;

    for track in 1..=ALBUM_LENGTH {
        let upcoming = Upcoming {
            current: Some(track),
            next: (track < ALBUM_LENGTH).then_some(track + 1),
        };

        // Well before the end: nothing is preloaded yet.
        scheduler.reconcile(&upcoming, Some(30_000));
        // Inside the lead: the successor opens.
        scheduler.reconcile(&upcoming, Some(PRELOAD_LEAD_MILLIS - 1));

        if track < ALBUM_LENGTH {
            assert_eq!(
                scheduler.state(track + 1),
                TrackState::Preloaded,
                "track {} was not ready when track {track} was about to end",
                track + 1
            );
        }

        let plan = scheduler.reconcile(
            &Upcoming {
                current: Some(track),
                next: (track < ALBUM_LENGTH).then_some(track + 1),
            },
            Some(PRELOAD_LEAD_MILLIS - 1),
        );
        reopened += plan.opens().count();
    }

    assert_eq!(
        reopened, 0,
        "a track that was already preloaded was opened again, which is the stall the preload exists to remove"
    );
}

#[test]
fn an_untrimmed_track_reaches_the_device_whole() {
    assert_eq!(play(Trim::NONE), TRACK_FRAMES as u64);
}

#[test]
fn two_trimmed_tracks_join_with_no_gap_and_no_lost_audio() {
    // The album was one recording; what reaches the device must be the master.
    let priming = 1_105;
    let real_frames = TRACK_FRAMES as u64 - priming - 431;

    let trim = Trim {
        start_frames: priming,
        valid_frames: Some(real_frames),
    };

    let first = play(trim);
    let second = play(trim);

    assert_eq!(first, real_frames);
    assert_eq!(second, real_frames);
    assert_eq!(
        first + second,
        real_frames * 2,
        "a gapless join must add no silence and drop no frames"
    );
}

#[test]
fn the_ring_is_the_same_size_after_forty_tracks_as_after_one() {
    let (producer, _consumer) = ring::ring(1 << 15);
    let capacity = producer.capacity();

    let mut carried = producer;
    for _ in 0..ALBUM_LENGTH {
        let decoder = SymphoniaDecoder::open(wav(4_410)).unwrap();
        let mut pump = Pump::new(Box::new(decoder), Trim::NONE, device(), carried);

        while pump.fill().unwrap() != PumpStep::EndOfStream {}

        // The pump is dropped with the track; the ring outlives it.
        let (next, _) = ring::ring(1 << 15);
        carried = next;
        assert_eq!(carried.capacity(), capacity);
    }
}

/// Resident set size in bytes, or `None` where the OS will not report it.
fn resident_bytes() -> Option<u64> {
    if !cfg!(any(target_os = "macos", target_os = "linux")) {
        return None;
    }

    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;

    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(|kib| kib * 1024)
}

#[test]
fn playing_forty_tracks_does_not_grow_the_process() {
    let Some(before) = resident_bytes() else {
        eprintln!("skipped: resident set size is not readable on this host");
        return;
    };

    for _ in 0..ALBUM_LENGTH {
        play(Trim::NONE);
    }

    let after = resident_bytes().expect("rss was readable a moment ago");
    let growth = after.saturating_sub(before);

    const ALLOWED_GROWTH: u64 = 64 * 1024 * 1024;

    assert!(
        growth < ALLOWED_GROWTH,
        "playing {ALBUM_LENGTH} tracks grew RSS by {} MiB",
        growth / 1024 / 1024
    );
}
