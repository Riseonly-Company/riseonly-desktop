use std::collections::HashMap;
use std::ffi::{c_int, c_ulong, c_void};

use objc2::encode::{Encoding, RefEncode};
use objc2::msg_send;
use objc2::runtime::{AnyClass, Bool};
use objc2_foundation::NSString;
use parking_lot::Mutex;

use super::capture_permission::*;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeVideo: &'static NSString;
    static AVMediaTypeAudio: &'static NSString;
}

unsafe extern "C" {
    static _NSConcreteStackBlock: [u8; 0];
}

#[repr(C)]
struct AnswerBlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
}

#[repr(C)]
struct AnswerBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(*const AnswerBlock, Bool),
    descriptor: *const AnswerBlockDescriptor,
    request: u64,
}

unsafe impl RefEncode for AnswerBlock {
    const ENCODING_REF: Encoding = Encoding::Block;
}

static ANSWER_DESCRIPTOR: AnswerBlockDescriptor = AnswerBlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<AnswerBlock>() as c_ulong,
};

struct Waiting {
    answers: HashMap<u64, AccessAnswer>,
    next: u64,
}

static WAITING: Mutex<Option<Waiting>> = Mutex::new(None);

fn park(done: AccessAnswer) -> u64 {
    let mut guard = WAITING.lock();
    let waiting = guard.get_or_insert_with(|| Waiting { answers: HashMap::new(), next: 1 });
    let request = waiting.next;
    waiting.next += 1;
    waiting.answers.insert(request, done);
    request
}

fn unpark(request: u64) -> Option<AccessAnswer> {
    WAITING.lock().as_mut().and_then(|waiting| waiting.answers.remove(&request))
}

fn media_type(kind: CaptureKind) -> &'static NSString {
    unsafe {
        match kind {
            CaptureKind::Camera => AVMediaTypeVideo,
            CaptureKind::Microphone => AVMediaTypeAudio,
        }
    }
}

fn capture_device_class() -> Result<&'static AnyClass, CaptureUnavailable> {
    AnyClass::get(c"AVCaptureDevice").ok_or(CaptureUnavailable::FrameworkMissing("AVCaptureDevice"))
}

pub(super) struct NativeCaptureAccess;

impl CaptureAccessSource for NativeCaptureAccess {
    fn status(&self, kind: CaptureKind) -> Result<CaptureAccess, CaptureUnavailable> {
        let device = capture_device_class()?;
        let raw: isize = unsafe { msg_send![device, authorizationStatusForMediaType: media_type(kind)] };
        Ok(CaptureAccess::from_av_status(raw as i64))
    }

    fn request(&self, kind: CaptureKind, done: AccessAnswer) -> Result<(), CaptureUnavailable> {
        let device = capture_device_class()?;
        let block = AnswerBlock {
            isa: (&raw const _NSConcreteStackBlock).cast(),
            flags: 0,
            reserved: 0,
            invoke: access_answered,
            descriptor: &raw const ANSWER_DESCRIPTOR,
            request: park(done),
        };
        unsafe {
            let _: () = msg_send![
                device,
                requestAccessForMediaType: media_type(kind),
                completionHandler: &raw const block
            ];
        }
        Ok(())
    }
}

unsafe extern "C" fn access_answered(block: *const AnswerBlock, granted: Bool) {
    let request = unsafe { (*block).request };
    let Some(done) = unpark(request) else {
        return;
    };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || done(granted.as_bool()))).is_err() {
        tracing::error!(target: "riseonly", "capture access answer panicked");
    }
}
