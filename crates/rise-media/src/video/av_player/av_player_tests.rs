use std::time::Duration;

use futures::channel::mpsc;
use gpui::ObjectFit;
use rise_core::foundation::foundation_url::Url;
use rise_platform::HostOs;

use crate::video::av_player::*;

fn url(text: &str) -> Url {
    Url::from_string(text).expect("test URL parses")
}

#[test]
fn gravity_maps_to_the_fit_that_places_the_picture_the_same_way() {
    assert!(matches!(AVLayerVideoGravity::ResizeAspect.object_fit(), ObjectFit::Contain));
    assert!(matches!(AVLayerVideoGravity::ResizeAspectFill.object_fit(), ObjectFit::Cover));
    assert!(matches!(AVLayerVideoGravity::Resize.object_fit(), ObjectFit::Fill));
    assert_eq!(AVLayerVideoGravity::default(), AVLayerVideoGravity::ResizeAspect);
}

#[test]
fn action_codes_follow_avplayer_h() {
    assert_eq!(AVPlayerActionAtItemEnd::Advance.code(), 0);
    assert_eq!(AVPlayerActionAtItemEnd::Pause.code(), 1);
    assert_eq!(AVPlayerActionAtItemEnd::None.code(), 2);
    assert_eq!(AVPlayerActionAtItemEnd::default(), AVPlayerActionAtItemEnd::Advance);
}

#[test]
fn a_file_url_reaches_the_system_as_a_decoded_path() {
    assert_eq!(
        SystemUrl::of(&url("file:///tmp/guide%20clip.mp4")),
        Ok(SystemUrl::File("/tmp/guide clip.mp4".to_owned()))
    );
}

#[test]
fn a_remote_url_reaches_the_system_as_its_absolute_string() {
    assert_eq!(
        SystemUrl::of(&url("https://cdn.example.com/a.mp4")),
        Ok(SystemUrl::Remote("https://cdn.example.com/a.mp4".to_owned()))
    );
}

#[test]
fn a_url_without_a_scheme_is_refused() {
    if let Some(relative) = Url::from_string("clip.mp4") {
        assert!(matches!(SystemUrl::of(&relative), Err(AVPlayerError::BadUrl(_))));
    }
}

#[test]
fn an_end_matches_by_asset_url_not_item_identity() {
    let template = url("file:///tmp/clip.mp4");
    assert!(is_end_of(&template, "file:///tmp/clip.mp4"));
    assert!(is_end_of(&template, "file:///tmp/./clip.mp4"));
    assert!(!is_end_of(&template, "file:///tmp/other.mp4"));
    assert!(!is_end_of(&template, ""));
}

#[test]
fn the_sink_forwards_only_ends_of_its_own_clip() {
    let template = url("file:///tmp/clip.mp4");
    let (sender, mut receiver) = mpsc::unbounded();
    let sink = EndSink::new(template.clone(), sender);
    sink.deliver("file:///tmp/other.mp4");
    sink.deliver("file:///tmp/clip.mp4");
    assert_eq!(receiver.try_next().ok().flatten(), Some(template));
    assert!(receiver.try_next().is_err());
}

#[test]
fn a_closed_receiver_does_not_fail_the_sink() {
    let (sender, receiver) = mpsc::unbounded();
    drop(receiver);
    EndSink::new(url("file:///tmp/clip.mp4"), sender).deliver("file:///tmp/clip.mp4");
}

#[test]
fn every_host_names_why_its_player_did_not_open() {
    assert!(unwired_reason(HostOs::Linux).contains("FFmpeg"));
    assert!(unwired_reason(HostOs::Windows).contains("Media Engine"));
    assert!(!unwired_reason(HostOs::MacOs).is_empty());
}

#[test]
fn an_item_lets_the_system_choose_its_buffer_until_one_is_set() {
    let mut item = AVPlayerItem::new(url("https://cdn.example.com/a.mp4"));
    assert_eq!(item.preferred_forward_buffer_duration(), std::time::Duration::ZERO);
    item.set_preferred_forward_buffer_duration(std::time::Duration::from_secs(8));
    assert_eq!(item.preferred_forward_buffer_duration(), std::time::Duration::from_secs(8));
    assert_eq!(item.clone().preferred_forward_buffer_duration(), std::time::Duration::from_secs(8));
}

#[test]
fn a_cm_time_has_seconds_only_while_valid_and_numeric() {
    assert_eq!(cm_time_seconds(600, 600, 1), Some(1.0));
    assert_eq!(cm_time_seconds(90, 600, 1 | 2), Some(0.15));
    assert_eq!(cm_time_seconds(0, 0, 0), None);
    assert_eq!(cm_time_seconds(0, 0, 1 | 16), None);
    assert_eq!(cm_time_seconds(0, 0, 1 | 4), None);
    assert_eq!(cm_time_seconds(0, 0, 1 | 8), None);
    assert_eq!(cm_time_seconds(600, 0, 1), None);
}

#[test]
fn a_periodic_observer_fires_on_the_first_time_and_whenever_time_moves() {
    assert!(time_observer_fires(None, Some(0.0)));
    assert!(time_observer_fires(Some(0.0), Some(0.1)));
    assert!(time_observer_fires(Some(2.0), Some(0.0)));
    assert!(!time_observer_fires(Some(0.1), Some(0.1)));
    assert!(!time_observer_fires(None, None));
    assert!(!time_observer_fires(Some(0.1), None));
}

#[test]
fn an_item_end_sink_carries_its_generation() {
    let (sender, mut receiver) = mpsc::unbounded();
    let sink = ItemEndSink::new(7, sender);
    assert_eq!(sink.generation(), 7);
    sink.deliver();
    assert_eq!(receiver.try_next().ok().flatten(), Some(7));
    drop(receiver);
    sink.deliver();
}

#[test]
fn a_time_range_ends_where_cmtimerangegetend_puts_it() {
    let range = CMTimeRange::new(1.5, 2.25);
    assert_eq!(range.end(), 3.75);
    assert!(range.is_valid());
    assert!(!CMTimeRange::new(f64::NAN, 1.0).is_valid());
    assert!(!CMTimeRange::new(0.0, f64::INFINITY).is_valid());
    assert!(!CMTimeRange::new(0.0, -1.0).is_valid());
}

#[test]
fn a_seek_window_follows_the_tolerances() {
    let exact = SeekWindow::of(2.0, 0.0, 0.0).expect("a numeric target seeks");
    assert_eq!(exact, SeekWindow { target_us: 2_000_000, min_us: 2_000_000, max_us: 2_000_000 });
    assert!(exact.is_exact());
    assert!(exact.is_before(1_999_999));
    assert!(!exact.is_before(2_000_000));

    let loose = SeekWindow::of(2.0, 0.5, f64::INFINITY).expect("a numeric target seeks");
    assert_eq!(loose.min_us, 1_500_000);
    assert_eq!(loose.max_us, i64::MAX);
    assert!(!loose.is_exact());

    let open_before = SeekWindow::of(1.0, f64::INFINITY, 0.0).expect("a numeric target seeks");
    assert_eq!(open_before.min_us, 0);
}

#[test]
fn a_seek_window_clamps_what_avplayer_would_not_take() {
    assert_eq!(SeekWindow::of(f64::NAN, 0.0, 0.0), None);
    assert_eq!(SeekWindow::of(f64::INFINITY, 0.0, 0.0), None);
    let before_start = SeekWindow::of(-3.0, 1.0, 1.0).expect("a negative target seeks to the start");
    assert_eq!(before_start.target_us, 0);
    assert_eq!(before_start.min_us, 0);
    assert_eq!(before_start.max_us, 1_000_000);
    let bad_tolerance = SeekWindow::of(1.0, -1.0, f64::NAN).expect("a numeric target seeks");
    assert!(bad_tolerance.is_exact());
}

#[test]
fn presentation_size_turns_with_the_display_rotation() {
    let landscape = presentation_size_of(1920, 1080, (1, 1), 0.0);
    assert_eq!((landscape.width, landscape.height), (1920.0, 1080.0));
    let portrait = presentation_size_of(1920, 1080, (0, 1), -90.0);
    assert_eq!((portrait.width, portrait.height), (1080.0, 1920.0));
    let upside_down = presentation_size_of(1920, 1080, (0, 0), 180.0);
    assert_eq!((upside_down.width, upside_down.height), (1920.0, 1080.0));
    let no_matrix = presentation_size_of(640, 480, (0, 1), f64::NAN);
    assert_eq!((no_matrix.width, no_matrix.height), (640.0, 480.0));
}

#[test]
fn presentation_size_stretches_by_the_sample_aspect_and_is_zero_while_unknown() {
    let anamorphic = presentation_size_of(720, 576, (16, 15), 0.0);
    assert_eq!((anamorphic.width, anamorphic.height), (768.0, 576.0));
    let unknown = presentation_size_of(0, 1080, (1, 1), 0.0);
    assert_eq!((unknown.width, unknown.height), (0.0, 0.0));
}

#[test]
fn the_forward_buffer_is_the_preferred_one_or_the_players_choice() {
    assert_eq!(forward_buffer_target(Duration::from_secs(3)), Duration::from_secs(3));
    assert!(forward_buffer_target(Duration::ZERO) > Duration::ZERO);
}

#[test]
fn a_waiting_player_starts_once_it_is_likely_to_keep_up() {
    let target = Duration::from_secs(8);
    assert!(!likely_to_keep_up(Duration::from_millis(500), false, target));
    assert!(likely_to_keep_up(Duration::from_secs(2), false, target));
    assert!(likely_to_keep_up(Duration::ZERO, true, target));
    assert!(likely_to_keep_up(Duration::from_millis(500), false, Duration::from_millis(500)));
}

#[test]
fn a_seek_sink_delivers_its_id_and_outcome() {
    let (sender, mut receiver) = mpsc::unbounded();
    let sink = SeekSink::new(7, sender);
    assert_eq!(sink.id(), 7);
    sink.deliver(true);
    assert_eq!(receiver.try_next().ok().flatten(), Some((7, true)));
}

#[test]
fn the_volume_is_clamped_to_silence_and_full_and_nan_changes_nothing() {
    assert_eq!(player_volume(0.4), Some(0.4));
    assert_eq!(player_volume(-0.2), Some(0.0));
    assert_eq!(player_volume(1.7), Some(1.0));
    assert_eq!(player_volume(f32::INFINITY), Some(1.0));
    assert_eq!(player_volume(f32::NAN), None);
}
