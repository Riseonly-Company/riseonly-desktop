use std::path::Path;
use std::time::Duration;

use super::{decoder_format, FolderWatcher};
use crate::photo_library::xdg_library::*;

#[test]
fn decoders_follow_the_extension() {
    assert_eq!(decoder_format(Path::new("/p/a.JPG")), Some(gpui::ImageFormat::Jpeg));
    assert_eq!(decoder_format(Path::new("/p/a.tiff")), Some(gpui::ImageFormat::Tiff));
    assert_eq!(decoder_format(Path::new("/p/a.heic")), None);
    assert_eq!(decoder_format(Path::new("/p/a")), None);
}

#[test]
fn inotify_reports_a_new_file_and_watches_a_new_folder() {
    let root = std::env::temp_dir().join(format!("rise-photo-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp folder");
    let mut watcher = FolderWatcher::open().expect("inotify works without a display");
    watcher.add_tree(&root, 0);
    assert_eq!(watcher.dirs.len(), 1);
    let mut buffer = vec![0u8; 64 * 1024];

    std::fs::write(root.join("IMG_0001.jpg"), b"x").expect("write");
    let records = watcher.wait(Duration::from_secs(2), &mut buffer).expect("poll");
    assert!(records
        .iter()
        .any(|record| record.name == "IMG_0001.jpg" && record.mask & IN_CLOSE_WRITE != 0));

    std::fs::create_dir(root.join("2026")).expect("mkdir");
    let records = watcher.wait(Duration::from_secs(2), &mut buffer).expect("poll");
    assert!(records.iter().any(|record| record.is_new_dir() && record.name == "2026"));

    let _ = std::fs::remove_dir_all(&root);
}
