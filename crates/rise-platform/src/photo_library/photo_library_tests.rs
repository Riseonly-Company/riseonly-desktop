use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use crate::host_os::HostOs;
use crate::photo_library::*;

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
fn ph_status_maps_like_the_swift_initialiser() {
    use PhotoLibraryAccessLevel::*;
    assert_eq!(PhotoLibraryAccessLevel::from_ph_status(0), NotDetermined);
    assert_eq!(PhotoLibraryAccessLevel::from_ph_status(1), Restricted);
    assert_eq!(PhotoLibraryAccessLevel::from_ph_status(2), Denied);
    assert_eq!(PhotoLibraryAccessLevel::from_ph_status(3), Authorized);
    assert_eq!(PhotoLibraryAccessLevel::from_ph_status(4), Limited);
    assert_eq!(PhotoLibraryAccessLevel::from_ph_status(17), Denied);
    assert_eq!(PhotoLibraryAccessLevel::from_ph_status(-1), Denied);
}

#[test]
fn only_full_and_limited_browse_and_only_undecided_asks() {
    use PhotoLibraryAccessLevel::*;
    let browse: Vec<_> = [Authorized, Limited, Denied, NotDetermined, Restricted]
        .into_iter()
        .filter(|level| level.can_browse())
        .collect();
    assert_eq!(browse, vec![Authorized, Limited]);
    assert!(NotDetermined.can_request());
    assert!(!Denied.can_request());
    assert!(!Restricted.can_request());
}

#[test]
fn media_kinds_match_the_fetch_predicates() {
    assert_eq!(LibraryMediaKind::Images.ph_media_types(), &[PH_MEDIA_TYPE_IMAGE]);
    assert_eq!(LibraryMediaKind::Videos.ph_media_types(), &[PH_MEDIA_TYPE_VIDEO]);
    assert_eq!(
        LibraryMediaKind::All.ph_media_types(),
        &[PH_MEDIA_TYPE_IMAGE, PH_MEDIA_TYPE_VIDEO]
    );
    assert!(LibraryMediaKind::Images.admits(LibraryAssetKind::Image));
    assert!(!LibraryMediaKind::Images.admits(LibraryAssetKind::Video));
    assert!(LibraryMediaKind::All.admits(LibraryAssetKind::Video));
}

#[test]
fn anything_but_video_is_shown_as_an_image() {
    assert_eq!(LibraryAssetKind::from_ph_media_type(2), LibraryAssetKind::Video);
    assert_eq!(LibraryAssetKind::from_ph_media_type(1), LibraryAssetKind::Image);
    assert_eq!(LibraryAssetKind::from_ph_media_type(3), LibraryAssetKind::Image);
    assert_eq!(LibraryAssetKind::from_ph_media_type(0), LibraryAssetKind::Image);
}

#[test]
fn pages_clamp_and_end() {
    assert_eq!(page_range(10, 0, 4), 0..4);
    assert_eq!(page_range(10, 8, 4), 8..10);
    assert_eq!(page_range(10, 12, 4), 10..10);
    assert_eq!(page_range(0, 0, 4), 0..0);
    assert_eq!(page_range(10, 3, usize::MAX), 3..10);
    assert_eq!(next_offset(10, &(0..4)), Some(4));
    assert_eq!(next_offset(10, &(8..10)), None);
    assert_eq!(next_offset(10, &(10..10)), None);
    assert_eq!(next_offset(10, &(0..0)), None);
}

#[test]
fn newest_first_with_undated_last_and_stable_ties() {
    let mut assets = vec![
        asset("b", Some(5)),
        asset("undated", None),
        asset("a", Some(5)),
        asset("new", Some(9)),
    ];
    sort_newest_first(&mut assets);
    let ids: Vec<&str> = assets.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, vec!["new", "a", "b", "undated"]);
}

#[test]
fn epoch_seconds_become_milliseconds() {
    assert_eq!(epoch_ms(1_700_000_000.25), Some(1_700_000_000_250));
    assert_eq!(epoch_ms(-1.0), Some(-1000));
    assert_eq!(epoch_ms(f64::NAN), None);
    assert_eq!(epoch_ms(f64::INFINITY), None);
}

#[test]
fn grid_and_hero_specs_follow_the_fetcher() {
    let grid = ImageSpec::grid();
    assert_eq!((grid.width, grid.height), (256, 256));
    assert_eq!(grid.mode, ImageContentMode::AspectFill);
    assert_eq!(grid.delivery, ImageDeliveryMode::Opportunistic);
    assert!(grid.allow_network);
    let hero = ImageSpec::hero(2880, 0);
    assert_eq!((hero.width, hero.height), (2880, 1));
    assert_eq!(hero.delivery, ImageDeliveryMode::HighQuality);
}

#[test]
fn unpremultiply_restores_straight_alpha() {
    let mut pixels = vec![
        0, 0, 0, 0,
        10, 20, 30, 255,
        64, 32, 0, 128,
    ];
    unpremultiply_bgra(&mut pixels);
    assert_eq!(&pixels[..8], &[0, 0, 0, 0, 10, 20, 30, 255]);
    assert_eq!(&pixels[8..], &[128, 64, 0, 128]);
}

#[test]
fn deliveries_end_on_final_cancel_or_missing_image() {
    let image = LibraryImage {
        width: 1,
        height: 1,
        bgra: vec![0, 0, 0, 255],
    };
    let delivery = |image: Option<LibraryImage>, degraded, cancelled| ImageDelivery {
        request: ImageRequestId(1),
        image,
        degraded,
        cancelled,
        error: None,
    };
    assert!(!delivery(Some(image.clone()), true, false).is_final());
    assert!(delivery(Some(image), false, false).is_final());
    assert!(delivery(None, true, true).is_final());
    assert!(delivery(None, true, false).is_final());
}

#[test]
fn request_ids_are_unique() {
    let a = ImageRequestId::next();
    let b = ImageRequestId::next();
    assert_ne!(a, b);
}

fn recorder() -> (Arc<StdMutex<Vec<ExportOutcome>>>, ExportDone) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let sink = seen.clone();
    let done: ExportDone = Box::new(move |outcome| sink.lock().unwrap().push(outcome));
    (seen, done)
}

#[test]
fn os_answers_become_the_three_export_ends() {
    let path = PathBuf::from("/cache/x.mov");
    assert_eq!(
        ExportOutcome::from_result(Ok(path.clone())),
        ExportOutcome::Exported(path)
    );
    assert_eq!(
        ExportOutcome::from_result(Err(PhotoLibraryUnavailable::Cancelled)),
        ExportOutcome::Cancelled
    );
    assert_eq!(
        ExportOutcome::from_result(Err(PhotoLibraryUnavailable::VideoUnavailable("a".into()))),
        ExportOutcome::Failed(PhotoLibraryUnavailable::VideoUnavailable("a".into()))
    );
}

#[test]
fn a_finished_export_answers_once() {
    let ledger = ExportLedger::new();
    let (seen, done) = recorder();
    let id = ImageRequestId(7);
    ledger.start(id, done);
    assert!(ledger.is_live(id));
    let path = PathBuf::from("/cache/a.mov");
    assert!(matches!(ledger.finish(id, Ok(path.clone())), ExportFinish::Delivered));
    assert!(!ledger.is_live(id));
    assert!(!ledger.cancel(id));
    assert_eq!(*seen.lock().unwrap(), vec![ExportOutcome::Exported(path)]);
}

#[test]
fn a_cancel_answers_now_and_the_late_file_is_handed_back_for_removal() {
    let ledger = ExportLedger::new();
    let (seen, done) = recorder();
    let id = ImageRequestId(8);
    ledger.start(id, done);
    assert!(ledger.cancel(id));
    assert!(!ledger.cancel(id));
    let late = PathBuf::from("/cache/late.mov");
    match ledger.finish(id, Ok(late.clone())) {
        ExportFinish::Discarded(Some(path)) => assert_eq!(path, late),
        other => panic!("late file not handed back: {other:?}"),
    }
    assert!(matches!(
        ledger.finish(id, Err(PhotoLibraryUnavailable::Cancelled)),
        ExportFinish::Discarded(None)
    ));
    assert_eq!(*seen.lock().unwrap(), vec![ExportOutcome::Cancelled]);
}

#[test]
fn a_forgotten_export_never_answers() {
    let ledger = ExportLedger::new();
    let (seen, done) = recorder();
    let id = ImageRequestId(9);
    ledger.start(id, done);
    ledger.forget(id);
    assert!(!ledger.is_live(id));
    assert!(matches!(
        ledger.finish(id, Err(PhotoLibraryUnavailable::NotAVideo("x".into()))),
        ExportFinish::Discarded(None)
    ));
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_page_names_its_total_and_the_next_offset() {
    let page = page_of(vec![asset("a", Some(1))], 3, &(0..1));
    assert_eq!(page.total, 3);
    assert_eq!(page.next_offset, Some(1));
    assert_eq!(page_of(Vec::new(), 3, &(3..3)).next_offset, None);
}

#[test]
fn export_names_are_safe_and_keep_the_extension() {
    assert_eq!(
        export_file_name("ABC-1/L0/001", "IMG_0001.MOV"),
        "ABC-1_L0_001.mov"
    );
    assert_eq!(export_file_name("", "clip"), "video.mov");
    assert_eq!(export_file_name("x", "a.m p4"), "x.mov");
    assert_eq!(export_file_name("x", "clip.mp4"), "x.mp4");
    assert_eq!(export_file_name(&"a".repeat(200), "c.mp4").len(), 96 + 4);
}

#[test]
fn unwritten_branches_name_the_host() {
    for host in [HostOs::MacOs, HostOs::Linux, HostOs::Windows] {
        assert_eq!(
            unavailable_by_default(host),
            PhotoLibraryUnavailable::BranchNotWritten(host)
        );
    }
}

fn save_scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rise-photo-save-{name}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn save_outcomes_match_media_save_to_library_result() {
    assert_eq!(LibrarySaveOutcome::from_result(Ok(())), LibrarySaveOutcome::Saved);
    assert_eq!(
        LibrarySaveOutcome::from_result(Err(PhotoLibraryUnavailable::NotAuthorized(
            PhotoLibraryAccessLevel::Denied
        ))),
        LibrarySaveOutcome::Denied
    );
    assert_eq!(
        LibrarySaveOutcome::from_result(Err(PhotoLibraryUnavailable::SystemRefused("x".into()))),
        LibrarySaveOutcome::Failed(PhotoLibraryUnavailable::SystemRefused("x".into()))
    );
}

#[test]
fn resource_kinds_carry_the_photokit_values() {
    assert_eq!(LibraryResourceKind::Photo.ph_resource_type(), 1);
    assert_eq!(LibraryResourceKind::Video.ph_resource_type(), 2);
}

#[test]
fn an_empty_batch_fails_before_any_prompt() {
    let outcome = Arc::new(StdMutex::new(None));
    let seen = Arc::clone(&outcome);
    save_originals(Vec::new(), move |result| {
        *seen.lock().unwrap() = Some(result);
    });
    assert_eq!(
        *outcome.lock().unwrap(),
        Some(LibrarySaveOutcome::Failed(PhotoLibraryUnavailable::NothingToSave))
    );
}

#[test]
fn a_missing_source_is_named_and_never_reaches_the_library() {
    let gone = save_scratch("missing").join("gone.jpg");
    let outcome = Arc::new(StdMutex::new(None));
    let seen = Arc::clone(&outcome);
    save_image(&gone, move |result| {
        *seen.lock().unwrap() = Some(result);
    });
    assert_eq!(
        *outcome.lock().unwrap(),
        Some(LibrarySaveOutcome::Failed(PhotoLibraryUnavailable::SourceMissing(gone)))
    );
}

#[test]
fn a_taken_name_gets_the_next_free_number() {
    let taken = ["photo.jpg", "photo (2).jpg", "notes"];
    let is_taken = |name: &str| taken.contains(&name);
    assert_eq!(unique_save_name("clip.mp4", is_taken), "clip.mp4");
    assert_eq!(unique_save_name("photo.jpg", is_taken), "photo (3).jpg");
    assert_eq!(unique_save_name("notes", is_taken), "notes (2)");
}

#[test]
fn folder_save_sorts_by_kind_keeps_names_with_spaces_and_unicode() {
    let root = save_scratch("folders");
    let pictures = root.join("Pictures");
    let videos = root.join("Videos");
    let image = root.join("summer \u{e9}t\u{e9}.jpg");
    let video = root.join("clip one.mp4");
    std::fs::write(&image, b"jpeg").unwrap();
    std::fs::write(&video, b"mp4").unwrap();
    let request = LibrarySaveRequest::Originals(vec![
        LibrarySaveFile {
            path: image.clone(),
            kind: LibraryResourceKind::Photo,
            content_type: None,
        },
        LibrarySaveFile {
            path: video.clone(),
            kind: LibraryResourceKind::Video,
            content_type: None,
        },
    ]);

    let written = save_into_folders(&request, Some(pictures.as_path()), Some(videos.as_path())).unwrap();
    assert_eq!(
        written,
        vec![pictures.join("summer \u{e9}t\u{e9}.jpg"), videos.join("clip one.mp4")]
    );

    let again = save_into_folders(&LibrarySaveRequest::Image(image), Some(pictures.as_path()), None).unwrap();
    assert_eq!(again, vec![pictures.join("summer \u{e9}t\u{e9} (2).jpg")]);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_failed_copy_rolls_back_the_files_saved_before_it() {
    let root = save_scratch("rollback");
    let pictures = root.join("Pictures");
    let image = root.join("a.png");
    let video = root.join("b.mov");
    std::fs::write(&image, b"png").unwrap();
    std::fs::write(&video, b"mov").unwrap();
    let request = LibrarySaveRequest::Originals(vec![
        LibrarySaveFile {
            path: image,
            kind: LibraryResourceKind::Photo,
            content_type: Some("public.png".to_owned()),
        },
        LibrarySaveFile {
            path: video,
            kind: LibraryResourceKind::Video,
            content_type: None,
        },
    ]);

    let result = save_into_folders(&request, Some(pictures.as_path()), None);
    assert!(matches!(result, Err(PhotoLibraryUnavailable::SystemRefused(_))));
    assert!(!pictures.join("a.png").exists(), "a half-saved batch must leave nothing behind");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn resource_type_maps_like_ph_asset_resource_type() {
    use LibraryAssetResourceType::*;
    assert_eq!(LibraryAssetResourceType::from_ph_resource_type(1), Photo);
    assert_eq!(LibraryAssetResourceType::from_ph_resource_type(2), Video);
    assert_eq!(LibraryAssetResourceType::from_ph_resource_type(3), Other);
    assert_eq!(LibraryAssetResourceType::from_ph_resource_type(4), Other);
    assert_eq!(LibraryAssetResourceType::from_ph_resource_type(5), FullSizePhoto);
    assert_eq!(LibraryAssetResourceType::from_ph_resource_type(6), FullSizeVideo);
    assert_eq!(LibraryAssetResourceType::from_ph_resource_type(9), Other);
    assert_eq!(LibraryAssetResourceType::from_ph_resource_type(0), Other);
}

type DataAnswers = Arc<StdMutex<Vec<Result<Vec<u8>, PhotoLibraryUnavailable>>>>;

fn data_request(maximum_bytes: usize) -> (Arc<ResourceDataRequest>, DataAnswers) {
    let answers: DataAnswers = Arc::new(StdMutex::new(Vec::new()));
    let sink = answers.clone();
    let request = ResourceDataRequest::new(
        maximum_bytes,
        Box::new(move |result| sink.lock().unwrap().push(result)),
    );
    (request, answers)
}

#[test]
fn resource_data_collects_chunks_and_answers_once() {
    let (request, answers) = data_request(8);
    assert!(request.receive(&[1, 2, 3]));
    assert!(request.receive(&[4, 5]));
    assert!(!request.is_finished());
    request.complete(None);
    request.complete(None);
    assert!(!request.cancel());
    assert!(request.is_finished());
    assert_eq!(*answers.lock().unwrap(), vec![Ok(vec![1, 2, 3, 4, 5])]);
}

#[test]
fn resource_data_at_the_ceiling_is_kept() {
    let (request, answers) = data_request(4);
    assert!(request.receive(&[1, 2]));
    assert!(request.receive(&[3, 4]));
    request.complete(None);
    assert_eq!(*answers.lock().unwrap(), vec![Ok(vec![1, 2, 3, 4])]);
}

#[test]
fn resource_data_over_the_ceiling_refuses_and_stops() {
    let (request, answers) = data_request(4);
    assert!(request.receive(&[1, 2, 3]));
    assert!(!request.receive(&[4, 5]));
    assert!(!request.receive(&[6]));
    request.complete(None);
    let answers = answers.lock().unwrap();
    assert_eq!(answers.len(), 1);
    assert!(matches!(answers[0], Err(PhotoLibraryUnavailable::SystemRefused(_))));
}

#[test]
fn resource_data_cancel_answers_cancelled_once() {
    let (request, answers) = data_request(16);
    assert!(request.receive(&[1]));
    assert!(request.cancel());
    assert!(!request.cancel());
    assert!(!request.receive(&[2]));
    request.complete(Some(PhotoLibraryUnavailable::SystemRefused("late".into())));
    assert_eq!(*answers.lock().unwrap(), vec![Err(PhotoLibraryUnavailable::Cancelled)]);
}

#[test]
fn resource_data_error_reaches_the_feature() {
    let (request, answers) = data_request(16);
    assert!(request.receive(&[1]));
    request.complete(Some(PhotoLibraryUnavailable::SystemRefused("offline".into())));
    assert_eq!(
        *answers.lock().unwrap(),
        vec![Err(PhotoLibraryUnavailable::SystemRefused("offline".into()))]
    );
}

#[test]
fn resource_data_forgotten_never_answers() {
    let (request, answers) = data_request(16);
    request.forget();
    assert!(request.is_finished());
    assert!(!request.receive(&[1]));
    request.complete(None);
    assert!(!request.cancel());
    assert!(answers.lock().unwrap().is_empty());
}

#[test]
fn progress_drops_errors_and_holds_the_fraction_to_the_bar() {
    assert_eq!(library_progress_fraction(0.25, false), Some(0.25));
    assert_eq!(library_progress_fraction(-0.5, false), Some(0.0));
    assert_eq!(library_progress_fraction(1.5, false), Some(1.0));
    assert_eq!(library_progress_fraction(0.25, true), None);
    assert_eq!(library_progress_fraction(f64::NAN, false), None);
}

#[derive(Default)]
struct LocalFiles {
    started: StdMutex<Vec<&'static str>>,
}

impl PhotoLibrarySource for LocalFiles {
    fn request_image(
        &self,
        _id: ImageRequestId,
        _asset_id: &str,
        _spec: ImageSpec,
        _sink: ImageSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        self.started.lock().unwrap().push("image");
        Ok(())
    }

    fn start_export(
        &self,
        _id: ImageRequestId,
        _asset_id: &str,
        _directory: &std::path::Path,
        _done: ExportSink,
    ) -> Result<(), PhotoLibraryUnavailable> {
        self.started.lock().unwrap().push("export");
        Ok(())
    }
}

#[test]
fn a_library_of_local_files_runs_the_request_and_never_reports_progress() {
    let library = LocalFiles::default();
    let reports = Arc::new(StdMutex::new(0_u32));
    let counter = reports.clone();
    let export_counter = reports.clone();
    let progress: LibraryProgress = Arc::new(move |_| *counter.lock().unwrap() += 1);
    let export_progress: ExportProgressSink = Arc::new(move |_| *export_counter.lock().unwrap() += 1);
    let id = ImageRequestId::next();
    library
        .request_image_with_progress(id, "a", ImageSpec::grid(), Box::new(|_| {}), Some(progress))
        .unwrap();
    library
        .start_export_with_progress(id, "a", std::path::Path::new("/tmp"), Box::new(|_| {}), Some(export_progress))
        .unwrap();
    assert_eq!(*library.started.lock().unwrap(), vec!["image", "export"]);
    assert_eq!(*reports.lock().unwrap(), 0);
}

#[test]
fn encode_progress_reports_by_steps_and_stays_below_the_end_like_the_ios_monitor() {
    let mut gate = EncodeProgressGate::default();
    assert_eq!(gate.report(0.0), Some(0.0));
    assert_eq!(gate.report(0.004), None);
    assert_eq!(gate.report(0.2), Some(0.2));
    assert_eq!(gate.report(0.1), None);
    assert_eq!(gate.report(1.0), Some(0.99));
    assert_eq!(gate.report(1.0), None);
    assert_eq!(gate.report(f64::NAN), None);
}

#[test]
fn smart_album_subtypes_are_photokit_raw_values() {
    let all = [
        (LibrarySmartAlbum::Favorites, 203),
        (LibrarySmartAlbum::Videos, 202),
        (LibrarySmartAlbum::SelfPortraits, 210),
        (LibrarySmartAlbum::LivePhotos, 213),
        (LibrarySmartAlbum::DepthEffect, 212),
        (LibrarySmartAlbum::Panoramas, 201),
        (LibrarySmartAlbum::Timelapses, 204),
        (LibrarySmartAlbum::SlomoVideos, 208),
        (LibrarySmartAlbum::Bursts, 207),
        (LibrarySmartAlbum::Screenshots, 211),
    ];
    for (album, raw) in all {
        assert_eq!(album.ph_subtype(), raw, "{album:?}");
    }
}
