use std::sync::Arc;
use std::thread;
use std::time::Duration;

use futures::StreamExt as _;
use parking_lot::Mutex;
use rise_core::foundation::foundation_url::Url;

use super::av_asset::*;

#[derive(Default)]
struct Recorded {
    information: Vec<ContentInformation>,
    bytes: Vec<u8>,
    outcomes: Vec<Result<(), String>>,
}

#[derive(Clone, Default)]
struct RecordingBackend(Arc<Mutex<Recorded>>);

impl LoadingRequestBackend for RecordingBackend {
    fn set_content_information(&self, information: &ContentInformation) {
        self.0.lock().information.push(information.clone());
    }

    fn respond(&self, data: &[u8]) {
        self.0.lock().bytes.extend_from_slice(data);
    }

    fn finish(&self, outcome: Result<(), anyhow::Error>) {
        self.0.lock().outcomes.push(outcome.map_err(|error| error.to_string()));
    }
}

fn asset() -> AVURLAsset {
    AVURLAsset::new(Url::from_string("riseonlystream://media.example/clip.mp4").expect("url"))
}

fn serve(asset: &AVURLAsset, resource: Vec<u8>) -> thread::JoinHandle<()> {
    let mut events = asset.resource_loader().take_events().expect("events");
    thread::spawn(move || {
        while let Some(event) = futures::executor::block_on(events.next()) {
            let LoaderEvent::Load(request) = event else {
                continue;
            };
            let length = i64::try_from(resource.len()).expect("length");
            if let Some(information) = request.content_information_request() {
                information.set_is_byte_range_access_supported(true);
                information.set_content_length(length);
                information.set_content_type(Some("public.mpeg-4".to_owned()));
            }
            if let Some(data) = request.data_request() {
                let start = usize::try_from(data.current_offset().min(length)).expect("start");
                let end = match data.requested_offset().checked_add(data.requested_length()) {
                    Some(sum) if !data.requests_all_data_to_end_of_resource() => sum.min(length),
                    _ => length,
                };
                let end = usize::try_from(end).expect("end").max(start);
                data.respond(&resource[start..end]);
            }
            request.finish_loading();
        }
    })
}

#[test]
fn data_request_reports_its_range_and_advances_the_current_offset() {
    let backend = RecordingBackend::default();
    let request = AVAssetResourceLoadingRequest::new(false, Some(DataRange::bytes(100, 10)), Box::new(backend.clone()));
    assert!(request.content_information_request().is_none());
    let data = request.data_request().expect("data request");
    assert_eq!((data.requested_offset(), data.requested_length()), (100, 10));
    assert!(!data.requests_all_data_to_end_of_resource());
    assert_eq!(data.current_offset(), 100);
    data.respond(&[1, 2, 3]);
    data.respond(&[]);
    assert_eq!(data.current_offset(), 103);
    assert_eq!(backend.0.lock().bytes, vec![1, 2, 3]);
}

#[test]
fn open_range_reports_the_largest_length() {
    let range = DataRange::to_end(7);
    assert_eq!(range.requested_length, i64::MAX);
    assert!(range.requests_all_data_to_end_of_resource);
}

#[test]
fn content_information_reaches_the_backend_field_by_field() {
    let backend = RecordingBackend::default();
    let request = AVAssetResourceLoadingRequest::new(true, None, Box::new(backend.clone()));
    let information = request.content_information_request().expect("information request");
    information.set_is_byte_range_access_supported(true);
    information.set_content_length(4_096);
    information.set_content_type(Some("public.mpeg-4".to_owned()));
    let recorded = backend.0.lock();
    assert_eq!(recorded.information.len(), 3);
    assert_eq!(
        recorded.information.last(),
        Some(&ContentInformation {
            is_byte_range_access_supported: true,
            content_length: 4_096,
            content_type: Some("public.mpeg-4".to_owned()),
        })
    );
}

#[test]
fn a_request_finishes_once_and_drops_answers_afterwards() {
    let backend = RecordingBackend::default();
    let request = AVAssetResourceLoadingRequest::new(true, Some(DataRange::bytes(0, 4)), Box::new(backend.clone()));
    request.finish_loading();
    request.finish_loading_with_error(anyhow::anyhow!("late"));
    request.data_request().expect("data").respond(&[9]);
    request.content_information_request().expect("information").set_content_length(1);
    assert!(request.is_finished());
    let recorded = backend.0.lock();
    assert_eq!(recorded.outcomes, vec![Ok(())]);
    assert!(recorded.bytes.is_empty());
    assert!(recorded.information.is_empty());
}

#[test]
fn cancel_marks_the_request_and_queues_the_cancellation() {
    let asset = asset();
    let loader = asset.resource_loader();
    let mut events = loader.take_events().expect("events");
    let backend = RecordingBackend::default();
    let request = AVAssetResourceLoadingRequest::new(false, Some(DataRange::bytes(0, 4)), Box::new(backend.clone()));
    loader.submit(request.clone());
    loader.cancel(&request);
    loader.cancel(&request);
    assert!(request.is_cancelled());
    request.data_request().expect("data").respond(&[1]);
    request.finish_loading();
    assert!(backend.0.lock().bytes.is_empty());
    assert!(backend.0.lock().outcomes.is_empty());
    drop(asset);
    drop(loader);
    let kinds: Vec<&str> = futures::executor::block_on_stream(&mut events)
        .map(|event| match event {
            LoaderEvent::Load(_) => "load",
            LoaderEvent::Cancel(_) => "cancel",
        })
        .collect();
    assert_eq!(kinds, vec!["load", "cancel"]);
}

#[test]
fn identifiers_are_distinct_and_stable_across_clones() {
    let first = AVAssetResourceLoadingRequest::new(true, None, Box::new(RecordingBackend::default()));
    let second = AVAssetResourceLoadingRequest::new(true, None, Box::new(RecordingBackend::default()));
    assert_ne!(first.identifier(), second.identifier());
    assert_eq!(first.clone().identifier(), first.identifier());
}

#[test]
fn assets_compare_by_identity() {
    let first = asset();
    let second = asset();
    assert_eq!(first.clone(), first);
    assert_ne!(first, second);
    assert_eq!(first.url(), second.url());
}

#[test]
fn byte_source_reads_ranges_and_content_information_through_the_loader() {
    let asset = asset();
    let resource: Vec<u8> = (0..=255).collect();
    let server = serve(&asset, resource.clone());
    let source = asset.resource_loader().byte_source();
    let information = source.content_information().expect("information");
    assert_eq!(information.content_length, 256);
    assert!(information.is_byte_range_access_supported);
    assert_eq!(source.read_at(10, 5).expect("read"), resource[10..15].to_vec());
    assert_eq!(source.read_at(250, 64).expect("tail"), resource[250..].to_vec());
    assert!(source.read_at(256, 64).expect("past the end").is_empty());
    assert!(source.read_at(0, 0).expect("empty").is_empty());
    drop(source);
    drop(asset);
    server.join().expect("server");
}

#[test]
fn byte_source_times_out_and_cancels_when_nobody_answers() {
    let asset = asset();
    let mut events = asset.resource_loader().take_events().expect("events");
    let source = asset.resource_loader().byte_source().with_timeout(Duration::from_millis(30));
    assert_eq!(source.read_at(0, 8), Err(AVAssetReadError::TimedOut(Duration::from_millis(30))));
    drop(source);
    drop(asset);
    let events: Vec<LoaderEvent> = futures::executor::block_on_stream(&mut events).collect();
    assert_eq!(events.len(), 2);
    match &events[1] {
        LoaderEvent::Cancel(request) => assert!(request.is_cancelled()),
        LoaderEvent::Load(_) => panic!("the second event is the cancellation"),
    }
}

#[test]
fn byte_source_reports_a_failed_request() {
    let asset = asset();
    let mut events = asset.resource_loader().take_events().expect("events");
    let answering = thread::spawn(move || {
        if let Some(LoaderEvent::Load(request)) = futures::executor::block_on(events.next()) {
            request.finish_loading_with_error(anyhow::anyhow!("network down"));
        }
    });
    let source = asset.resource_loader().byte_source();
    assert_eq!(source.read_at(0, 8), Err(AVAssetReadError::Failed("network down".to_owned())));
    answering.join().expect("answering");
}

#[test]
fn seek_target_follows_the_avio_contract() {
    assert_eq!(avio_seek_target(100, SEEK_SET, 7, None), Some(100));
    assert_eq!(avio_seek_target(-3, SEEK_CUR, 7, None), Some(4));
    assert_eq!(avio_seek_target(-10, SEEK_END, 0, Some(256)), Some(246));
    assert_eq!(avio_seek_target(0, SEEK_END, 0, None), None);
    assert_eq!(avio_seek_target(0, AVSEEK_SIZE, 0, Some(256)), Some(256));
    assert_eq!(avio_seek_target(0, AVSEEK_SIZE, 0, None), None);
    assert_eq!(avio_seek_target(5, SEEK_SET | AVSEEK_FORCE, 0, None), Some(5));
    assert_eq!(avio_seek_target(-1, SEEK_SET, 0, None), None);
    assert_eq!(avio_seek_target(1, SEEK_CUR, i64::MAX, None), None);
    assert_eq!(avio_seek_target(0, 9, 0, None), None);
}
