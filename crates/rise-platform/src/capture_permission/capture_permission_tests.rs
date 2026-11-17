use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;

use crate::capture_permission::*;
use crate::host_os::HostOs;

/// source answering from fixed states
/// request records the kind and answers at once or keeps the answer for answer_later
struct FakeSource {
    camera: CaptureAccess,
    microphone: CaptureAccess,
    prompt_answer: bool,
    hold: bool,
    asked: Mutex<Vec<CaptureKind>>,
    held: Mutex<Option<AccessAnswer>>,
}

impl FakeSource {
    fn new(camera: CaptureAccess, microphone: CaptureAccess, prompt_answer: bool) -> Self {
        Self {
            camera,
            microphone,
            prompt_answer,
            hold: false,
            asked: Mutex::new(Vec::new()),
            held: Mutex::new(None),
        }
    }

    fn answer_later(&self, granted: bool) {
        let done = self.held.lock().take().expect("a request is held");
        done(granted);
    }
}

impl CaptureAccessSource for FakeSource {
    fn status(&self, kind: CaptureKind) -> Result<CaptureAccess, CaptureUnavailable> {
        Ok(match kind {
            CaptureKind::Camera => self.camera,
            CaptureKind::Microphone => self.microphone,
        })
    }

    fn request(&self, kind: CaptureKind, done: AccessAnswer) -> Result<(), CaptureUnavailable> {
        self.asked.lock().push(kind);
        if self.hold {
            *self.held.lock() = Some(done);
        } else {
            done(self.prompt_answer);
        }
        Ok(())
    }
}

struct Unwritten;

impl CaptureAccessSource for Unwritten {}

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the fake source answers synchronously"),
    }
}

#[test]
fn av_status_maps_like_the_ios_switch() {
    assert_eq!(CaptureAccess::from_av_status(0), CaptureAccess::NotDetermined);
    assert_eq!(CaptureAccess::from_av_status(1), CaptureAccess::Restricted);
    assert_eq!(CaptureAccess::from_av_status(2), CaptureAccess::Denied);
    assert_eq!(CaptureAccess::from_av_status(3), CaptureAccess::Granted);
    assert_eq!(
        CaptureAccess::from_av_status(9),
        CaptureAccess::Denied,
        "@unknown default returns false on iOS"
    );
    assert_eq!(CaptureAccess::from_av_status(-1), CaptureAccess::Denied);
}

#[test]
fn only_an_undecided_status_asks() {
    assert_eq!(next_step(CaptureAccess::Granted), CaptureStep::Proceed);
    assert_eq!(next_step(CaptureAccess::NotDetermined), CaptureStep::Ask);
    assert_eq!(next_step(CaptureAccess::Denied), CaptureStep::Refuse);
    assert_eq!(next_step(CaptureAccess::Restricted), CaptureStep::Refuse);
}

#[test]
fn a_refusal_before_offers_settings_and_a_refused_prompt_does_not() {
    assert_eq!(decision_after(CaptureAccess::Granted, None), CaptureDecision::Granted);
    assert_eq!(decision_after(CaptureAccess::NotDetermined, Some(true)), CaptureDecision::Granted);
    assert_eq!(decision_after(CaptureAccess::NotDetermined, Some(false)), CaptureDecision::Declined);
    assert_eq!(decision_after(CaptureAccess::Denied, None), CaptureDecision::Blocked);
    assert_eq!(decision_after(CaptureAccess::Restricted, Some(true)), CaptureDecision::Blocked);
    assert!(CaptureDecision::Blocked.offers_settings());
    assert!(!CaptureDecision::Declined.offers_settings());
    assert!(CaptureDecision::Granted.is_granted());
}

#[test]
fn ensure_access_prompts_only_when_undecided() {
    let granted = FakeSource::new(CaptureAccess::Granted, CaptureAccess::Granted, false);
    assert_eq!(ready(ensure_access_from(&granted, CaptureKind::Camera)), Ok(CaptureDecision::Granted));
    assert!(granted.asked.lock().is_empty());

    let undecided = FakeSource::new(CaptureAccess::NotDetermined, CaptureAccess::Granted, true);
    assert_eq!(ready(ensure_access_from(&undecided, CaptureKind::Camera)), Ok(CaptureDecision::Granted));
    assert_eq!(*undecided.asked.lock(), vec![CaptureKind::Camera]);

    let denied = FakeSource::new(CaptureAccess::Denied, CaptureAccess::Granted, true);
    assert_eq!(ready(ensure_access_from(&denied, CaptureKind::Camera)), Ok(CaptureDecision::Blocked));
    assert!(denied.asked.lock().is_empty());
}

#[test]
fn a_video_note_asks_the_microphone_first_and_the_camera_decides() {
    let source = FakeSource::new(CaptureAccess::NotDetermined, CaptureAccess::NotDetermined, false);
    assert_eq!(ready(ensure_video_note_access_from(&source)), Ok(CaptureDecision::Declined));
    assert_eq!(*source.asked.lock(), vec![CaptureKind::Microphone, CaptureKind::Camera]);

    let microphone_decided = FakeSource::new(CaptureAccess::Granted, CaptureAccess::Denied, false);
    assert_eq!(ready(ensure_video_note_access_from(&microphone_decided)), Ok(CaptureDecision::Granted));
    assert!(microphone_decided.asked.lock().is_empty(), "a decided microphone is not asked again");
}

#[test]
fn a_request_waits_for_the_os_answer() {
    let mut source = FakeSource::new(CaptureAccess::NotDetermined, CaptureAccess::Granted, true);
    source.hold = true;
    let mut request = pin!(request_from(&source, CaptureKind::Camera));
    let mut context = Context::from_waker(Waker::noop());
    assert!(request.as_mut().poll(&mut context).is_pending());
    source.answer_later(false);
    assert_eq!(request.as_mut().poll(&mut context), Poll::Ready(Ok(false)));
}

#[test]
fn an_unwritten_branch_names_the_host_and_reports_unsupported() {
    let reason = CaptureUnavailable::BranchNotWritten(HostOs::current());
    assert_eq!(Unwritten.status(CaptureKind::Camera), Err(reason));
    assert_eq!(ready(request_from(&Unwritten, CaptureKind::Microphone)), Err(reason));
    assert_eq!(ready(ensure_access_from(&Unwritten, CaptureKind::Camera)), Err(reason));
}

#[test]
fn every_host_and_kind_has_a_settings_answer() {
    for kind in CaptureKind::ALL {
        let mac = settings_url(HostOs::MacOs, kind).expect("macOS has a privacy pane");
        assert!(mac.starts_with("x-apple.systempreferences:com.apple.preference.security?Privacy_"));
        let windows = settings_url(HostOs::Windows, kind).expect("Windows has a privacy page");
        assert!(windows.starts_with("ms-settings:privacy-"));
        assert_eq!(settings_url(HostOs::Linux, kind), None);
    }
    assert_ne!(
        settings_url(HostOs::MacOs, CaptureKind::Camera),
        settings_url(HostOs::MacOs, CaptureKind::Microphone)
    );
}

#[test]
fn the_usage_keys_are_the_info_plist_ones() {
    assert_eq!(CaptureKind::Camera.usage_description_key(), "NSCameraUsageDescription");
    assert_eq!(CaptureKind::Microphone.usage_description_key(), "NSMicrophoneUsageDescription");
}
