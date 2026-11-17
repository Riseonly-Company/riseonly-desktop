use super::folder_library::*;
use super::photo_library::*;

fn asset(id: &str, created_at_ms: Option<i64>) -> LibraryAsset {
    LibraryAsset {
        id: id.to_owned(),
        kind: LibraryAssetKind::Image,
        pixel_width: 10,
        pixel_height: 10,
        duration_secs: 0.0,
        created_at_ms,
    }
}

#[test]
fn folder_files_are_classified_by_extension() {
    use std::path::Path;
    assert_eq!(folder_asset_kind(Path::new("a/IMG_1.JPG")), Some(LibraryAssetKind::Image));
    assert_eq!(folder_asset_kind(Path::new("clip.MOV")), Some(LibraryAssetKind::Video));
    assert_eq!(folder_asset_kind(Path::new("b.webm")), Some(LibraryAssetKind::Video));
    assert_eq!(folder_asset_kind(Path::new("desktop.ini")), None);
    assert_eq!(folder_asset_kind(Path::new("noext")), None);
}

#[test]
fn a_copied_file_keeps_the_date_of_the_shot() {
    use std::time::{Duration, UNIX_EPOCH};
    let shot = UNIX_EPOCH + Duration::from_secs(1_000);
    let copied = UNIX_EPOCH + Duration::from_secs(5_000);
    assert_eq!(folder_file_date_ms(Some(copied), Some(shot)), Some(1_000_000));
    assert_eq!(folder_file_date_ms(None, Some(shot)), Some(1_000_000));
    assert_eq!(folder_file_date_ms(None, None), None);
    assert_eq!(
        system_time_ms(UNIX_EPOCH - Duration::from_millis(1_500)),
        Some(-1_500)
    );
}

#[test]
fn durations_in_100ns_become_seconds() {
    assert_eq!(duration_secs_from_100ns(25_000_000), 2.5);
    assert_eq!(duration_secs_from_100ns(0), 0.0);
}

#[test]
fn nested_and_repeated_roots_are_walked_once() {
    use std::path::PathBuf;
    let roots = folder_roots([
        PathBuf::from("/u/Pictures/Camera"),
        PathBuf::from("/u/Videos"),
        PathBuf::from("/u/Pictures"),
        PathBuf::from("/u/Videos"),
        PathBuf::new(),
    ]);
    assert_eq!(roots, vec![PathBuf::from("/u/Videos"), PathBuf::from("/u/Pictures")]);
}

#[test]
fn only_files_under_the_roots_are_answered() {
    use std::path::{Path, PathBuf};
    let root = std::env::temp_dir().join("rise-roots");
    let roots = vec![root.clone()];
    assert!(is_inside_roots(&root.join("a.jpg"), &roots));
    assert!(!is_inside_roots(&root.join("..").join("a.jpg"), &roots));
    assert!(!is_inside_roots(Path::new("a.jpg"), &roots));
    assert!(!is_inside_roots(&PathBuf::from("/elsewhere/a.jpg"), &[]));
}

#[test]
fn the_folder_walk_lists_media_newest_first_and_skips_hidden() {
    use std::time::{Duration, SystemTime};
    let root = std::env::temp_dir().join(format!("rise-photo-walk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(root.join(".hidden")).unwrap();
    let old = root.join("old.jpg");
    let new = root.join("sub").join("new.mp4");
    std::fs::write(&old, b"x").unwrap();
    std::fs::write(&new, b"x").unwrap();
    std::fs::write(root.join("notes.txt"), b"x").unwrap();
    std::fs::write(root.join(".hidden").join("h.jpg"), b"x").unwrap();
    let past = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(past)
        .unwrap();

    let assets = scan_folder_library(std::slice::from_ref(&root));
    let ids: Vec<_> = assets.iter().map(|asset| asset.id.clone()).collect();
    assert_eq!(
        ids,
        vec![new.to_str().unwrap().to_owned(), old.to_str().unwrap().to_owned()]
    );
    assert_eq!(assets[0].kind, LibraryAssetKind::Video);
    assert_eq!(count_of_kind(&assets, LibraryMediaKind::Images), 1);
    assert_eq!(count_of_kind(&assets, LibraryMediaKind::All), 2);
    assert_eq!(slice_of_kind(&assets, LibraryMediaKind::Images, 0..5)[0].id, ids[1]);
    assert!(slice_of_kind(&assets, LibraryMediaKind::Videos, 1..3).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_snapshot_is_reused_while_fresh_and_after_invalidate_walks_again() {
    use std::time::Duration;
    let cache = FolderSnapshotCache::new();
    let now = Duration::from_secs(100);
    let first = cache.get_or_scan(now, || vec![asset("a", Some(1))]);
    let again = cache.get_or_scan(now + Duration::from_millis(10), || vec![asset("b", Some(1))]);
    assert_eq!(again[0].id, first[0].id);
    let stale = cache.get_or_scan(now + FOLDER_SNAPSHOT_TTL, || vec![asset("c", Some(1))]);
    assert_eq!(stale[0].id, "c");
    cache.invalidate();
    let fresh = cache.get_or_scan(now + FOLDER_SNAPSHOT_TTL, || vec![asset("d", Some(1))]);
    assert_eq!(fresh[0].id, "d");
}

fn spec(width: u32, height: u32, mode: ImageContentMode) -> ImageSpec {
    ImageSpec {
        width,
        height,
        mode,
        delivery: ImageDeliveryMode::HighQuality,
        allow_network: true,
    }
}

#[test]
fn the_thumbnail_box_covers_the_target_on_fill() {
    let fit = thumbnail_box(spec(300, 200, ImageContentMode::AspectFit), Some((4000, 3000)));
    assert_eq!((fit.width, fit.height, fit.crop_to_square), (300, 200, false));
    let grid = thumbnail_box(ImageSpec::grid(), None);
    assert_eq!((grid.width, grid.height, grid.crop_to_square), (256, 256, true));
    let tall_grid = thumbnail_box(ImageSpec::grid(), Some((900, 1100)));
    assert_eq!((tall_grid.width, tall_grid.height, tall_grid.crop_to_square), (256, 313, false));
    let hero = thumbnail_box(spec(1000, 500, ImageContentMode::AspectFill), Some((3000, 3000)));
    assert_eq!((hero.width, hero.height, hero.crop_to_square), (1000, 1000, false));
    let unknown = thumbnail_box(spec(1000, 500, ImageContentMode::AspectFill), None);
    assert_eq!((unknown.width, unknown.height), (1000, 1000));
    let panorama = thumbnail_box(spec(400, 800, ImageContentMode::AspectFill), Some((100_000, 100)));
    assert_eq!(panorama.width, THUMBNAIL_BOX_MAX_EDGE);
    assert_eq!(panorama.height, 800);
}

#[test]
fn shell_bitmaps_without_alpha_are_opaque_and_others_get_straight() {
    let mut opaque = vec![10, 20, 30, 0, 40, 50, 60, 0];
    straighten_shell_bgra(&mut opaque);
    assert_eq!(opaque, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    let mut half = vec![64, 64, 64, 128, 0, 0, 0, 0];
    straighten_shell_bgra(&mut half);
    assert_eq!(half, vec![128, 128, 128, 128, 0, 0, 0, 0]);
}

#[test]
fn a_cancelled_request_finishes_cancelled_once() {
    let book = PendingImageRequests::new();
    let id = ImageRequestId(7);
    book.start(id);
    assert!(book.is_live(id));
    assert!(book.cancel(id));
    assert!(!book.is_live(id));
    assert!(!book.finish(id));
    assert!(!book.cancel(id));
    let other = ImageRequestId(8);
    book.start(other);
    assert!(book.finish(other));
    assert!(cancelled_delivery(id).is_final());
    assert!(failed_delivery(id, "x".into()).is_final());
}

#[test]
fn a_burst_of_events_is_one_change() {
    use std::time::Duration;
    let start = Duration::from_secs(100);
    let mut coalescer = ChangeCoalescer::default();
    assert_eq!(coalescer.wait(start), None);
    assert!(!coalescer.take_due(start));
    coalescer.note(start);
    coalescer.note(start + Duration::from_millis(100));
    assert_eq!(
        coalescer.wait(start + Duration::from_millis(100)),
        Some(FOLDER_CHANGE_QUIET)
    );
    assert!(!coalescer.take_due(start + Duration::from_millis(300)));
    assert!(coalescer.take_due(start + Duration::from_millis(500)));
    assert!(!coalescer.take_due(start + Duration::from_millis(600)));

    let mut steady = ChangeCoalescer::default();
    for step in 0..30 {
        steady.note(start + Duration::from_millis(step * 100));
    }
    assert!(steady.take_due(start + FOLDER_CHANGE_MAX_DELAY));
}

fn notify_record(next: u32, name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut record = Vec::new();
    record.extend_from_slice(&next.to_le_bytes());
    record.extend_from_slice(&1u32.to_le_bytes());
    record.extend_from_slice(&((units.len() * 2) as u32).to_le_bytes());
    for unit in units {
        record.extend_from_slice(&unit.to_le_bytes());
    }
    while record.len() % 4 != 0 {
        record.push(0);
    }
    record
}

#[test]
fn notify_records_are_read_and_filtered() {
    let head_len = notify_record(0, "desktop.ini").len() as u32;
    let mut buffer = notify_record(head_len, "desktop.ini");
    buffer.extend_from_slice(&notify_record(0, "Camera\\IMG_1.jpg"));
    let names = notify_record_names(&buffer);
    assert_eq!(names, vec!["desktop.ini".to_owned(), "Camera\\IMG_1.jpg".to_owned()]);
    assert!(change_touches_media(&names, false));
    assert!(!change_touches_media(&["Thumbs.db".to_owned()], false));
    assert!(change_touches_media(&["New folder".to_owned()], false));
    assert!(change_touches_media(&[], true));
    assert!(notify_record_names(&buffer[..10]).is_empty());
    let mut truncated = notify_record(0, "abcdef.jpg");
    truncated.truncate(14);
    assert!(notify_record_names(&truncated).is_empty());
}

fn resource_request(
    maximum_bytes: usize,
) -> (
    std::sync::Arc<ResourceDataRequest>,
    std::sync::mpsc::Receiver<Result<Vec<u8>, PhotoLibraryUnavailable>>,
) {
    let (sender, answers) = std::sync::mpsc::channel();
    let request = ResourceDataRequest::new(
        maximum_bytes,
        Box::new(move |result| {
            let _ = sender.send(result);
        }),
    );
    (request, answers)
}

#[test]
fn a_folder_asset_has_the_file_as_its_one_resource() {
    use std::path::Path;
    let photo = folder_asset_resources(Path::new("/p/IMG_1.JPG"), LibraryAssetKind::Image);
    assert_eq!(
        photo,
        vec![LibraryAssetResource {
            resource_type: LibraryAssetResourceType::Photo,
            uniform_type_identifier: "public.jpeg".to_owned(),
            original_filename: "IMG_1.JPG".to_owned(),
            index: 0,
        }]
    );
    let clip = folder_asset_resources(Path::new("/v/clip.mov"), LibraryAssetKind::Video);
    assert_eq!(clip[0].resource_type, LibraryAssetResourceType::Video);
    assert_eq!(clip[0].uniform_type_identifier, "com.apple.quicktime-movie");
    let undeclared = folder_asset_resources(Path::new("/v/clip.mkv"), LibraryAssetKind::Video);
    assert_eq!(undeclared[0].uniform_type_identifier, "public.movie");
    let bitmap = folder_asset_resources(Path::new("/p/scan.bmp"), LibraryAssetKind::Image);
    assert_eq!(bitmap[0].uniform_type_identifier, "public.image");
}

#[test]
fn only_a_gif_image_is_animated() {
    use std::path::Path;
    assert!(folder_is_animated_image(Path::new("a/loop.GIF"), LibraryAssetKind::Image));
    assert!(!folder_is_animated_image(Path::new("a/still.png"), LibraryAssetKind::Image));
    assert!(!folder_is_animated_image(Path::new("a/odd.gif"), LibraryAssetKind::Video));
}

#[test]
fn a_folder_asset_has_no_resource_past_index_zero() {
    let mut resource = folder_asset_resources(
        std::path::Path::new("/p/a.png"),
        LibraryAssetKind::Image,
    )
    .remove(0);
    assert_eq!(folder_resource_exists("/p/a.png", &resource), Ok(()));
    resource.index = 1;
    assert_eq!(
        folder_resource_exists("/p/a.png", &resource),
        Err(PhotoLibraryUnavailable::AssetNotFound("/p/a.png".to_owned()))
    );
}

#[test]
fn a_resource_read_hands_over_every_chunk_once() {
    let bytes: Vec<u8> =
        (0..FOLDER_RESOURCE_CHUNK_BYTES * 2 + 7).map(|i| (i % 251) as u8).collect();
    let (request, answers) = resource_request(bytes.len());
    feed_resource_request(&mut std::io::Cursor::new(bytes.clone()), &request);
    assert_eq!(answers.try_recv(), Ok(Ok(bytes)));
    assert!(answers.try_recv().is_err());
    assert!(request.is_finished());
}

#[test]
fn a_resource_over_the_ceiling_is_refused_and_the_read_stops() {
    let bytes = vec![7u8; FOLDER_RESOURCE_CHUNK_BYTES * 3];
    let (request, answers) = resource_request(10);
    let mut reader = std::io::Cursor::new(bytes);
    feed_resource_request(&mut reader, &request);
    assert!(matches!(answers.try_recv(), Ok(Err(PhotoLibraryUnavailable::SystemRefused(_)))));
    assert!(answers.try_recv().is_err());
    assert_eq!(reader.position() as usize, FOLDER_RESOURCE_CHUNK_BYTES);
}

#[test]
fn a_cancelled_resource_read_does_not_answer_again() {
    let (request, answers) = resource_request(1024);
    assert!(request.cancel());
    let mut reader = std::io::Cursor::new(vec![1u8; 16]);
    feed_resource_request(&mut reader, &request);
    assert_eq!(answers.try_recv(), Ok(Err(PhotoLibraryUnavailable::Cancelled)));
    assert!(answers.try_recv().is_err());
    assert_eq!(reader.position(), 0);
}

#[test]
fn a_failing_resource_read_answers_its_error() {
    struct Broken;
    impl std::io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"))
        }
    }
    let (request, answers) = resource_request(1024);
    feed_resource_request(&mut Broken, &request);
    assert!(matches!(answers.try_recv(), Ok(Err(PhotoLibraryUnavailable::SystemRefused(_)))));
}

#[test]
fn a_video_has_no_image_data() {
    let answer =
        read_folder_image_data(std::path::Path::new("/v/clip.mov"), LibraryAssetKind::Video);
    assert!(matches!(answer, Err(PhotoLibraryUnavailable::SystemRefused(_))));
}
