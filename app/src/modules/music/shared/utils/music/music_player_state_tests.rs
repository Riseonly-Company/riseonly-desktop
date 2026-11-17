use crate::modules::music::shared::utils::music::music_player_state::{
    MusicAudioDownloadProgress, MusicAudioDownloadProgressCoalescer,
};

#[test]
fn fraction_is_authoritative_and_clamped() {
    assert_eq!(MusicAudioDownloadProgress::new(512, 1_024).fraction(), Some(0.5));
    assert_eq!(MusicAudioDownloadProgress::new(2_048, 1_024).fraction(), Some(1.0));
    assert_eq!(MusicAudioDownloadProgress::new(-1, 0).fraction(), None);
}

#[test]
fn byte_description_includes_received_and_expected_sizes() {
    let progress = MusicAudioDownloadProgress::new(1_572_864, 6_291_456);
    assert_eq!(progress.byte_description(), "1.5 MB / 6.0 MB");
}

#[test]
fn coalescer_bounds_callback_frequency_but_never_drops_completion() {
    let mut coalescer = MusicAudioDownloadProgressCoalescer::default();
    let initial = coalescer.should_publish(MusicAudioDownloadProgress::new(1_024, 100_000), 1.0);
    assert!(initial);
    let below_interval =
        coalescer.should_publish(MusicAudioDownloadProgress::new(20_000, 100_000), 1.05);
    assert!(!below_interval);
    let interval_elapsed =
        coalescer.should_publish(MusicAudioDownloadProgress::new(20_000, 100_000), 1.1);
    assert!(interval_elapsed);
    let small_increment =
        coalescer.should_publish(MusicAudioDownloadProgress::new(21_000, 100_000), 1.4);
    assert!(!small_increment);
    let silence_elapsed =
        coalescer.should_publish(MusicAudioDownloadProgress::new(21_000, 100_000), 1.6);
    assert!(silence_elapsed);
    let completion =
        coalescer.should_publish(MusicAudioDownloadProgress::new(100_000, 100_000), 1.61);
    assert!(completion);
}
