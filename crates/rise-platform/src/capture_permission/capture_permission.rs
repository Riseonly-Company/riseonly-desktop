use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use parking_lot::Mutex;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// what is being captured, video or audio
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CaptureKind {
    Camera,
    Microphone,
}

impl CaptureKind {
    pub const ALL: [Self; 2] = [Self::Camera, Self::Microphone];

    /// info.plist key whose text the macos prompt shows
    /// without it tcc kills the process on the request so Info.plist.in must carry both
    pub const fn usage_description_key(self) -> &'static str {
        match self {
            Self::Camera => "NSCameraUsageDescription",
            Self::Microphone => "NSMicrophoneUsageDescription",
        }
    }
}

/// authorization status of capture access
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CaptureAccess {
    Granted,
    Denied,
    Restricted,
    NotDetermined,
}

impl CaptureAccess {
    /// raw status values, not determined 0 and restricted 1 and denied 2 and authorized 3
    /// an unknown value counts as a refusal
    pub const fn from_av_status(raw: i64) -> Self {
        match raw {
            0 => Self::NotDetermined,
            1 => Self::Restricted,
            3 => Self::Granted,
            _ => Self::Denied,
        }
    }

    pub const fn is_granted(self) -> bool {
        matches!(self, Self::Granted)
    }
}

/// what a caller does next for a given status
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureStep {
    Proceed,
    Ask,
    Refuse,
}

pub const fn next_step(access: CaptureAccess) -> CaptureStep {
    match access {
        CaptureAccess::Granted => CaptureStep::Proceed,
        CaptureAccess::NotDetermined => CaptureStep::Ask,
        CaptureAccess::Denied | CaptureAccess::Restricted => CaptureStep::Refuse,
    }
}

/// how an access check ended
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureDecision {
    Granted,
    Declined,
    Blocked,
}

impl CaptureDecision {
    pub const fn is_granted(self) -> bool {
        matches!(self, Self::Granted)
    }

    /// true when the screen should offer the system settings link
    pub const fn offers_settings(self) -> bool {
        matches!(self, Self::Blocked)
    }
}

/// decision for a status read before and the prompt answer when one was shown
pub const fn decision_after(before: CaptureAccess, prompt_answer: Option<bool>) -> CaptureDecision {
    match (next_step(before), prompt_answer) {
        (CaptureStep::Proceed, _) => CaptureDecision::Granted,
        (CaptureStep::Refuse, _) => CaptureDecision::Blocked,
        (CaptureStep::Ask, Some(true)) => CaptureDecision::Granted,
        (CaptureStep::Ask, Some(false) | None) => CaptureDecision::Declined,
    }
}

/// why access could not be read or asked
#[derive(Clone, Copy, PartialEq, Eq, Debug, Error)]
pub enum CaptureUnavailable {
    #[error("the {0:?} layer does not read camera and microphone access yet")]
    BranchNotWritten(HostOs),
    #[error("the system framework class {0} is not available")]
    FrameworkMissing(&'static str),
}

/// called once with the prompt answer, on whatever thread the os answers on
pub type AccessAnswer = Box<dyn FnOnce(bool) + Send>;

/// os side, every method defaults to branch not written for the running host
/// sync so the access futures can run on the background executor
pub trait CaptureAccessSource: Sync {
    fn status(&self, _kind: CaptureKind) -> Result<CaptureAccess, CaptureUnavailable> {
        Err(CaptureUnavailable::BranchNotWritten(HostOs::current()))
    }

    /// shows the system prompt when undecided
    /// when decided the os answers with the current state right away
    /// done is called exactly once when this returns ok
    fn request(&self, _kind: CaptureKind, _done: AccessAnswer) -> Result<(), CaptureUnavailable> {
        Err(CaptureUnavailable::BranchNotWritten(HostOs::current()))
    }
}

#[derive(Default)]
struct AnswerSlot {
    answer: Option<Result<bool, CaptureUnavailable>>,
    waker: Option<Waker>,
}

fn fill(slot: &Mutex<AnswerSlot>, answer: Result<bool, CaptureUnavailable>) {
    let waker = {
        let mut slot = slot.lock();
        slot.answer = Some(answer);
        slot.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// the prompt answer, awaited
/// runs on any executor and the os answer wakes it
pub struct AccessRequest {
    slot: Arc<Mutex<AnswerSlot>>,
}

impl Future for AccessRequest {
    type Output = Result<bool, CaptureUnavailable>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self.slot.lock();
        match slot.answer.take() {
            Some(answer) => Poll::Ready(answer),
            None => {
                slot.waker = Some(context.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// asks source, the entry points of the seam pass the os file
pub fn request_from(source: &dyn CaptureAccessSource, kind: CaptureKind) -> AccessRequest {
    let slot = Arc::new(Mutex::new(AnswerSlot::default()));
    let answered = Arc::clone(&slot);
    let done: AccessAnswer = Box::new(move |granted| fill(&answered, Ok(granted)));
    if let Err(reason) = source.request(kind, done) {
        fill(&slot, Err(reason));
    }
    AccessRequest { slot }
}

/// proceeds when granted, prompts when undecided and refuses otherwise
pub async fn ensure_access_from(
    source: &dyn CaptureAccessSource,
    kind: CaptureKind,
) -> Result<CaptureDecision, CaptureUnavailable> {
    let before = source.status(kind)?;
    let answer = match next_step(before) {
        CaptureStep::Ask => Some(request_from(source, kind).await?),
        CaptureStep::Proceed | CaptureStep::Refuse => None,
    };
    Ok(decision_after(before, answer))
}

/// mic is asked first when undecided and its answer doesnt stop the recording
/// the note records without sound, the camera decides
pub async fn ensure_video_note_access_from(
    source: &dyn CaptureAccessSource,
) -> Result<CaptureDecision, CaptureUnavailable> {
    if source.status(CaptureKind::Microphone)? == CaptureAccess::NotDetermined {
        let microphone = request_from(source, CaptureKind::Microphone).await?;
        tracing::debug!(granted = microphone, "microphone prompt answered before a video note");
    }
    ensure_access_from(source, CaptureKind::Camera).await
}

static NATIVE_ACCESS: super::os::NativeCaptureAccess = super::os::NativeCaptureAccess;
static TESTER_SOURCES: AtomicBool = AtomicBool::new(false);

/// tester stand-in, camera and mic are test sources that need no consent
/// so every status reads granted and no prompt shows
struct TesterCaptureAccess;

impl CaptureAccessSource for TesterCaptureAccess {
    fn status(&self, _kind: CaptureKind) -> Result<CaptureAccess, CaptureUnavailable> {
        Ok(CaptureAccess::Granted)
    }

    fn request(&self, _kind: CaptureKind, done: AccessAnswer) -> Result<(), CaptureUnavailable> {
        done(true);
        Ok(())
    }
}

static TESTER_ACCESS: TesterCaptureAccess = TesterCaptureAccess;

pub fn set_tester_sources(on: bool) {
    TESTER_SOURCES.store(on, Ordering::Release);
}

pub fn uses_tester_sources() -> bool {
    TESTER_SOURCES.load(Ordering::Acquire)
}

fn source() -> &'static dyn CaptureAccessSource {
    if uses_tester_sources() { &TESTER_ACCESS } else { &NATIVE_ACCESS }
}

pub fn status(kind: CaptureKind) -> Result<CaptureAccess, CaptureUnavailable> {
    source().status(kind)
}

pub fn request(kind: CaptureKind) -> AccessRequest {
    request_from(source(), kind)
}

pub async fn ensure_access(kind: CaptureKind) -> Result<CaptureDecision, CaptureUnavailable> {
    ensure_access_from(source(), kind).await
}

pub async fn ensure_video_note_access() -> Result<CaptureDecision, CaptureUnavailable> {
    ensure_video_note_access_from(source()).await
}

/// performed when this host reads the access state, the screen hides capture otherwise
pub fn capture_support(kind: CaptureKind) -> PlatformSupport {
    match status(kind) {
        Ok(_) => PlatformSupport::Performed,
        Err(reason) => {
            tracing::debug!(%reason, ?kind, "capture access unreadable");
            PlatformSupport::Unsupported
        }
    }
}

/// privacy pane for the kind, linux desktops have no common one so no link there
pub const fn settings_url(host: HostOs, kind: CaptureKind) -> Option<&'static str> {
    match (host, kind) {
        (HostOs::MacOs, CaptureKind::Camera) => {
            Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Camera")
        }
        (HostOs::MacOs, CaptureKind::Microphone) => {
            Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        }
        (HostOs::Windows, CaptureKind::Camera) => Some("ms-settings:privacy-webcam"),
        (HostOs::Windows, CaptureKind::Microphone) => Some("ms-settings:privacy-microphone"),
        (HostOs::Linux, _) => None,
    }
}

pub fn open_settings(app: &mut gpui::App, kind: CaptureKind) -> PlatformSupport {
    match settings_url(HostOs::current(), kind) {
        Some(url) => {
            app.open_url(url);
            PlatformSupport::Performed
        }
        None => PlatformSupport::Unsupported,
    }
}
