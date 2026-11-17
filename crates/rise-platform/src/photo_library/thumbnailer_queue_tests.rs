use std::path::Path;

use super::folder_library::{FOLDER_VIDEO_EXTENSIONS, ThumbnailBox};
use super::thumbnailer_queue::*;

fn square(edge: u32) -> ThumbnailBox {
    ThumbnailBox { width: edge, height: edge, crop_to_square: true }
}

fn flavors(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn every_folder_video_extension_has_a_mime_type() {
    for extension in FOLDER_VIDEO_EXTENSIONS {
        let name = format!("clip.{extension}");
        assert!(video_mime_type(Path::new(&name)).is_some(), "{extension}");
    }
}

#[test]
fn the_mime_type_ignores_case_and_skips_other_files() {
    assert_eq!(video_mime_type(Path::new("trip.MOV")), Some("video/quicktime"));
    assert_eq!(video_mime_type(Path::new("trip.mp4")), Some("video/mp4"));
    assert_eq!(video_mime_type(Path::new("photo.jpg")), None);
    assert_eq!(video_mime_type(Path::new("noextension")), None);
}

#[test]
fn the_flavor_is_the_first_cache_folder_the_service_makes() {
    let all = flavors(&["normal", "large", "x-large", "xx-large"]);
    assert_eq!(thumbnailer_flavor(square(100), &all), Some("normal"));
    assert_eq!(thumbnailer_flavor(square(256), &all), Some("large"));
    let old = flavors(&["normal", "large"]);
    assert_eq!(thumbnailer_flavor(square(600), &old), Some("large"));
    assert_eq!(thumbnailer_flavor(square(100), &flavors(&["large"])), Some("large"));
    assert_eq!(thumbnailer_flavor(square(100), &flavors(&["huge"])), None);
    assert_eq!(thumbnailer_flavor(square(100), &[]), None);
}

#[test]
fn the_wait_ends_only_on_its_own_handle_and_file() {
    let uri = "file:///home/a/Videos/clip.mp4";
    let other = "file:///home/a/Videos/other.mp4".to_owned();
    let ready = ThumbnailerSignal::Ready { handle: 7, uris: vec![uri.to_owned()] };
    assert_eq!(thumbnailer_step(7, uri, &ready), ThumbnailerStep::LookInCache);
    assert_eq!(thumbnailer_step(8, uri, &ready), ThumbnailerStep::Wait);
    let ready_other = ThumbnailerSignal::Ready { handle: 7, uris: vec![other.clone()] };
    assert_eq!(thumbnailer_step(7, uri, &ready_other), ThumbnailerStep::Wait);
    let error = ThumbnailerSignal::Error { handle: 7, uris: vec![other, uri.to_owned()] };
    assert_eq!(thumbnailer_step(7, uri, &error), ThumbnailerStep::GiveUp);
    assert_eq!(thumbnailer_step(7, uri, &ThumbnailerSignal::Finished { handle: 7 }), ThumbnailerStep::LookInCache);
    assert_eq!(thumbnailer_step(7, uri, &ThumbnailerSignal::Finished { handle: 3 }), ThumbnailerStep::Wait);
    assert_eq!(thumbnailer_step(7, uri, &ThumbnailerSignal::Unreadable), ThumbnailerStep::Wait);
}
