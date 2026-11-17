use std::path::PathBuf;

use super::photo_library::*;
use super::picked_files_library::*;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rise-picked-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).ok();
    root
}

#[test]
fn registers_media_files_in_order_once() {
    let root = scratch("order");
    let picture = root.join("shot.png");
    let clip = root.join("clip.mp4");
    let note = root.join("note.txt");
    for path in [&picture, &clip, &note] {
        std::fs::write(path, b"not a real header").ok();
    }
    let missing = root.join("gone.jpg");

    let assets = register_picked_files(&[clip.clone(), note.clone(), picture.clone(), clip.clone(), missing.clone()]);

    let ids: Vec<&str> = assets.iter().map(|asset| asset.id.as_str()).collect();
    assert_eq!(ids, vec![clip.to_str().unwrap_or_default(), picture.to_str().unwrap_or_default()]);
    assert_eq!(assets[0].kind, LibraryAssetKind::Video);
    assert_eq!(assets[1].kind, LibraryAssetKind::Image);
    assert!(is_picked_asset(picture.to_str().unwrap_or_default()));
    assert!(!is_picked_asset(note.to_str().unwrap_or_default()));
    assert!(!is_picked_asset(missing.to_str().unwrap_or_default()));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn picked_files_need_no_library_consent() {
    let root = scratch("consent");
    let picture = root.join("face.jpg");
    std::fs::write(&picture, b"bytes").ok();
    let id = picture.to_str().unwrap_or_default().to_owned();
    assert_eq!(register_picked_files(std::slice::from_ref(&picture)).len(), 1);

    let resources = asset_resources(&id);

    assert_eq!(resources.len(), 1);
    std::fs::remove_dir_all(&root).ok();
}
