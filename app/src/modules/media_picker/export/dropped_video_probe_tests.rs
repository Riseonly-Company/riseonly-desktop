use std::path::Path;

use rise_core::foundation::foundation_url::Url;
use rise_media::video_export::{Affine, FrameSize, VideoProbe};

use super::dropped_video_probe::DroppedVideoProbe;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportAttachmentFactory;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};

fn video_probe(duration_seconds: f64) -> VideoProbe {
    VideoProbe {
        duration_seconds,
        video_start_seconds: 0.0,
        video_duration_seconds: 0.0,
        natural_size: FrameSize::new(1920.0, 1080.0),
        preferred_transform: Affine::IDENTITY,
        nominal_frame_rate: 30.0,
        has_video: true,
        has_audio: true,
        byte_count: 1024,
    }
}

fn dropped_video() -> MediaPickerItem {
    let url = Url::from_file_path(Path::new("/clips/clip.mov")).expect("absolute path");
    MediaPickerItem::new(
        "clip".to_owned(),
        MediaPickerItemKind::Video,
        url,
        "clip.mov".to_owned(),
        None,
        None,
        None,
        None,
        false,
    )
}

#[test]
fn unreadable_length_is_no_duration() {
    assert_eq!(DroppedVideoProbe::readable_seconds(&video_probe(0.0)), None);
    assert_eq!(DroppedVideoProbe::readable_seconds(&video_probe(f64::NAN)), None);
    assert_eq!(DroppedVideoProbe::readable_seconds(&video_probe(90.5)), Some(90.5));
}

#[test]
fn probed_video_reaches_the_ceiling_check_untrimmed() {
    let mut item = dropped_video();
    DroppedVideoProbe::apply_duration(&mut item, 90.5);
    let attachment = BugReportAttachmentFactory::make(&item).expect("video attachment");
    assert_eq!(attachment.duration_ms, Some(90_500));
    assert!(!attachment.was_trimmed_to_ceiling);
    assert_eq!(attachment.url, Some(item.url.clone()));
}

#[test]
fn unprobed_video_stays_without_duration() {
    let attachment = BugReportAttachmentFactory::make(&dropped_video()).expect("video attachment");
    assert_eq!(attachment.duration_ms, None);
}
