use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rise_platform::HostOs;

use crate::rtc_media::{
    screen_share_shelf, ScreenShareLiveness, ScreenShareQuality, VideoCeiling, PRESET_H1080, PRESET_H1440,
    SCREEN_SHARE_FIRST_FRAME_WINDOW,
};
use crate::screen_capture::screen_capture::*;

fn source(width: u32, height: u32) -> CaptureSource {
    CaptureSource { id: "display-1".into(), kind: CaptureSourceKind::Display, name: String::new(), width, height }
}

fn frame() -> ScreenFrame {
    ScreenFrame {
        width: 2,
        height: 2,
        pixels: ScreenPixels::Bgra { pixels: vec![0u8; 16].into(), stride: 8 },
        captured_at: Duration::ZERO,
    }
}

/// what the fake capture handed to the share so a test can drive it like a capture thread
#[derive(Default)]
struct Wires {
    frames: Mutex<Option<ScreenFrameSink>>,
    ended: Mutex<Option<ScreenEndSink>>,
    stops: Arc<AtomicUsize>,
}

struct FakeSession {
    source: CaptureSource,
    stops: Arc<AtomicUsize>,
}

impl ScreenCaptureSession for FakeSession {
    fn source(&self) -> &CaptureSource {
        &self.source
    }

    fn stop(&mut self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeBackend {
    wires: Arc<Wires>,
}

impl ScreenCaptureBackend for FakeBackend {
    fn support(&self) -> Result<(), ScreenCaptureError> {
        Ok(())
    }

    fn start(
        &self,
        source: &CaptureSource,
        _options: CaptureOptions,
        frames: ScreenFrameSink,
        ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        *self.wires.frames.lock() = Some(frames);
        *self.wires.ended.lock() = Some(ended);
        Ok(Box::new(FakeSession { source: source.clone(), stops: self.wires.stops.clone() }))
    }
}

struct Started {
    share: ScreenShareCapture,
    wires: Arc<Wires>,
    delivered: Arc<AtomicUsize>,
    ends: Arc<Mutex<Vec<ScreenCaptureEnd>>>,
}

fn start() -> Started {
    let wires = Arc::new(Wires::default());
    let backend = FakeBackend { wires: wires.clone() };
    let delivered = Arc::new(AtomicUsize::new(0));
    let ends = Arc::new(Mutex::new(Vec::new()));
    let counter = delivered.clone();
    let sink_ends = ends.clone();
    let share = ScreenShareCapture::start(
        &backend,
        &source(2560, 1600),
        CaptureOptions { max_fps: 30, max_long_side: 1920, max_short_side: 1080 },
        Arc::new(move |_frame: ScreenFrame| {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(move |end: ScreenCaptureEnd| sink_ends.lock().push(end)),
    )
    .unwrap();
    Started { share, wires, delivered, ends }
}

fn deliver(wires: &Wires) {
    let sink = wires.frames.lock().clone().unwrap();
    sink(frame());
}

fn end(wires: &Wires, end: ScreenCaptureEnd) {
    let sink = wires.ended.lock().clone().unwrap();
    sink(end);
}

#[test]
fn shelf_reaches_1440p_only_for_a_source_that_large() {
    let ceiling = VideoCeiling { max_height: 1440, max_fps: 30 };
    let large = screen_share_shelf(ScreenShareQuality::High, ceiling, source(2560, 1600).pixel_size());
    assert_eq!((large.width, large.height), (PRESET_H1440.width, PRESET_H1440.height));
    let laptop = screen_share_shelf(ScreenShareQuality::High, ceiling, source(1920, 1200).pixel_size());
    assert_eq!((laptop.width, laptop.height), (PRESET_H1080.width, PRESET_H1080.height));
}

#[test]
fn options_follow_the_shelf_and_keep_the_aspect() {
    let shelf = screen_share_shelf(ScreenShareQuality::High, VideoCeiling { max_height: 1080, max_fps: 30 }, (3024, 1964));
    let options = CaptureOptions::from_shelf(&shelf);
    assert_eq!(options.max_fps, 30);
    assert_eq!(options.output_size(3024, 1964), (1662, 1080));
    assert_eq!(options.output_size(1280, 800), (1280, 800));
    assert_eq!(options.output_size(1080, 1920), (1080, 1920));
    assert_eq!(options.output_size(1201, 701), (1200, 700));
    assert_eq!(options.output_size(0, 10), (2, 2));
    assert_eq!(options.frame_interval(), Duration::from_secs(1) / 30);
}

#[test]
fn first_frame_window_tells_a_dead_pipe_from_a_still_screen() {
    let mut started = start();
    let now = rise_core::foundation::foundation_date::RiseClock::monotonic();
    assert_eq!(started.share.liveness(now), ScreenShareLiveness::Waiting);
    assert_eq!(started.share.liveness(now + SCREEN_SHARE_FIRST_FRAME_WINDOW * 2), ScreenShareLiveness::DeadPipe);
    deliver(&started.wires);
    assert_eq!(started.share.frames_seen(), 1);
    assert_eq!(started.share.liveness(now + SCREEN_SHARE_FIRST_FRAME_WINDOW * 2), ScreenShareLiveness::Alive);
    started.share.stop();
}

#[test]
fn a_lost_source_stops_the_capture_once_and_reports_once() {
    let started = start();
    deliver(&started.wires);
    end(&started.wires, ScreenCaptureEnd::SourceLost);
    end(&started.wires, ScreenCaptureEnd::StoppedBySystem);
    assert_eq!(started.wires.stops.load(Ordering::SeqCst), 1);
    assert_eq!(*started.ends.lock(), vec![ScreenCaptureEnd::SourceLost]);
    assert!(!started.share.is_running());
    deliver(&started.wires);
    assert_eq!(started.delivered.load(Ordering::SeqCst), 1);
    drop(started.share);
    assert_eq!(started.wires.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn the_app_stop_is_silent_and_idempotent() {
    let mut started = start();
    started.share.stop();
    started.share.stop();
    end(&started.wires, ScreenCaptureEnd::SourceLost);
    assert_eq!(started.wires.stops.load(Ordering::SeqCst), 1);
    assert!(started.ends.lock().is_empty());
}

#[test]
fn bgra_converts_to_limited_range_nv12() {
    let white = [255u8, 255, 255, 255].repeat(4);
    let (luma, chroma) = bgra_to_nv12(&white, 2, 2, 8).unwrap();
    assert_eq!(luma, vec![235; 4]);
    assert_eq!(chroma, vec![128, 128]);
    let black = [0u8, 0, 0, 255].repeat(6);
    let (luma, chroma) = bgra_to_nv12(&black, 3, 2, 12).unwrap();
    assert_eq!(luma, vec![16; 6]);
    assert_eq!(chroma, vec![128; 4]);
    assert!(bgra_to_nv12(&black, 3, 2, 8).is_err());
    let host = frame().into_host_frame(7, 1).unwrap();
    assert_eq!((host.width, host.height, host.luma_stride, host.chroma_stride), (2, 2, 2, 2));
}

#[test]
fn errors_tell_unsupported_hosts_and_user_choices_apart() {
    assert!(ScreenCaptureError::BranchNotWritten(HostOs::Linux).is_unsupported());
    assert!(ScreenCaptureError::Unsupported("X11 without the ScreenCast portal".into()).is_unsupported());
    assert!(!ScreenCaptureError::AccessDenied.is_unsupported());
    assert!(ScreenCaptureError::Cancelled.is_user_choice());
    assert_eq!(settings_url(HostOs::MacOs), Some(MACOS_SCREEN_CAPTURE_SETTINGS_URL));
    assert_eq!(settings_url(HostOs::Windows), None);
}

#[test]
fn portal_parent_names_the_window() {
    assert_eq!(portal_parent_window(&PickerParent::X11(0x3c00007)), "x11:3c00007");
    assert_eq!(portal_parent_window(&PickerParent::Wayland("abc".into())), "wayland:abc");
    assert_eq!(portal_parent_window(&PickerParent::Wayland(String::new())), "");
    assert_eq!(portal_parent_window(&PickerParent::None), "");
}

#[test]
fn bgra_scales_to_the_shelf_by_nearest_sample() {
    let mut pixels = vec![0u8; 4 * 2 * 4];
    for y in 0..2 {
        for x in 0..4 {
            pixels[y * 16 + x * 4] = (y * 4 + x) as u8;
        }
    }
    let same = scale_bgra(&pixels, 4, 2, 16, 4, 2).unwrap();
    assert_eq!(same, pixels);
    let half = scale_bgra(&pixels, 4, 2, 16, 2, 1).unwrap();
    assert_eq!(half.len(), 8);
    assert_eq!((half[0], half[4]), (0, 2));
    assert!(scale_bgra(&pixels, 4, 2, 8, 2, 1).is_err());
    assert!(scale_bgra(&pixels[..20], 4, 2, 16, 2, 1).is_err());
}

#[test]
fn screen_access_maps_to_one_privacy_status() {
    use rise_platform::privacy_access::AccessStatus;
    assert_eq!(AccessStatus::from(ScreenCaptureAccess::Granted), AccessStatus::Authorized);
    assert_eq!(AccessStatus::from(ScreenCaptureAccess::AskedOnPick), AccessStatus::Authorized);
    assert_eq!(AccessStatus::from(ScreenCaptureAccess::Denied), AccessStatus::Denied);
}

#[derive(Default)]
struct AudioWires {
    frames_only_starts: AtomicUsize,
    audio: Mutex<Option<ScreenAudioSink>>,
}

struct AudioBackend {
    supported: bool,
    wires: Arc<AudioWires>,
}

struct AudioSession {
    source: CaptureSource,
}

impl ScreenCaptureSession for AudioSession {
    fn source(&self) -> &CaptureSource {
        &self.source
    }

    fn stop(&mut self) {}
}

impl ScreenCaptureBackend for AudioBackend {
    fn system_audio_support(&self) -> Result<(), ScreenCaptureError> {
        if self.supported {
            Ok(())
        } else {
            Err(ScreenCaptureError::Unsupported("no sound service".into()))
        }
    }

    fn start(
        &self,
        source: &CaptureSource,
        _options: CaptureOptions,
        _frames: ScreenFrameSink,
        _ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        self.wires.frames_only_starts.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(AudioSession { source: source.clone() }))
    }

    fn start_with_system_audio(
        &self,
        source: &CaptureSource,
        _options: CaptureOptions,
        _frames: ScreenFrameSink,
        audio: ScreenAudioSink,
        _ended: ScreenEndSink,
    ) -> Result<Box<dyn ScreenCaptureSession>, ScreenCaptureError> {
        *self.wires.audio.lock() = Some(audio);
        Ok(Box::new(AudioSession { source: source.clone() }))
    }
}

fn start_with_audio(backend: &AudioBackend, heard: Arc<AtomicUsize>) -> ScreenShareCapture {
    ScreenShareCapture::start_with_system_audio(
        backend,
        &source(1920, 1080),
        CaptureOptions { max_fps: 30, max_long_side: 1920, max_short_side: 1080 },
        Arc::new(|_| {}),
        Arc::new(move |_| {
            heard.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(|_| {}),
    )
    .expect("the fake backend starts")
}

fn block() -> ScreenAudioBlock {
    ScreenAudioBlock { samples: vec![0; 4], sample_rate: 48_000, channels: 2 }
}

#[test]
fn a_host_without_system_sound_still_shares_the_picture() {
    let wires = Arc::new(AudioWires::default());
    let backend = AudioBackend { supported: false, wires: wires.clone() };
    let share = start_with_audio(&backend, Arc::new(AtomicUsize::new(0)));
    assert!(share.is_running());
    assert_eq!(wires.frames_only_starts.load(Ordering::SeqCst), 1);
    assert!(wires.audio.lock().is_none());
}

#[test]
fn system_sound_flows_until_the_share_stops() {
    let wires = Arc::new(AudioWires::default());
    let backend = AudioBackend { supported: true, wires: wires.clone() };
    let heard = Arc::new(AtomicUsize::new(0));
    let mut share = start_with_audio(&backend, heard.clone());
    assert_eq!(wires.frames_only_starts.load(Ordering::SeqCst), 0);
    let sink = wires.audio.lock().clone().expect("the sink reached the backend");
    sink(block());
    assert_eq!(heard.load(Ordering::SeqCst), 1);
    share.stop();
    sink(block());
    assert_eq!(heard.load(Ordering::SeqCst), 1, "no sound after the stop");
}

#[test]
fn planes_interleave_frame_by_frame_and_stop_at_the_shortest() {
    let left = [0.1_f32, 0.2, 0.3];
    let right = [0.4_f32, 0.5];
    assert_eq!(interleaved_from_planes(&[&left, &right]), vec![0.1, 0.4, 0.2, 0.5]);
    assert_eq!(interleaved_from_planes(&[&left]), vec![0.1, 0.2, 0.3]);
    assert!(interleaved_from_planes(&[]).is_empty());
}

#[test]
fn float_samples_become_pcm_and_clip() {
    assert_eq!(pcm16_from_float(&[0.0, 1.0, -1.0, 2.0, -2.0]), vec![0, 32767, -32767, 32767, -32767]);
}
