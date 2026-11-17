use std::ffi::{c_long, c_ulong, c_void};
use std::ptr::NonNull;

use crate::memory_pressure::*;

#[repr(C)]
struct DispatchSourceType {
    _opaque: [u8; 0],
}

type DispatchObject = *mut c_void;
type DispatchQueue = *mut c_void;
type DispatchFunction = unsafe extern "C" fn(*mut c_void);

const QOS_CLASS_UTILITY: c_long = 0x11;

unsafe extern "C" {
    #[link_name = "_dispatch_source_type_memorypressure"]
    static DISPATCH_SOURCE_TYPE_MEMORYPRESSURE: DispatchSourceType;

    fn dispatch_get_global_queue(identifier: c_long, flags: c_ulong) -> DispatchQueue;
    fn dispatch_source_create(
        kind: *const DispatchSourceType,
        handle: usize,
        mask: c_ulong,
        queue: DispatchQueue,
    ) -> DispatchObject;
    fn dispatch_set_context(object: DispatchObject, context: *mut c_void);
    fn dispatch_source_set_event_handler_f(source: DispatchObject, handler: DispatchFunction);
    fn dispatch_source_set_cancel_handler_f(source: DispatchObject, handler: DispatchFunction);
    fn dispatch_source_get_data(source: DispatchObject) -> c_ulong;
    fn dispatch_resume(object: DispatchObject);
    fn dispatch_source_cancel(source: DispatchObject);
    fn dispatch_release(object: DispatchObject);
}

/// what the event handler reads through the source context
struct Context {
    source: DispatchObject,
    sink: LevelSink,
}

pub(crate) struct NativeMemoryPressure;

impl MemoryPressureSource for NativeMemoryPressure {
    fn start(&self, sink: LevelSink) -> Result<MemoryPressureWatch, MemoryPressureUnavailable> {
        let mask: c_ulong = DISPATCH_MEMORYPRESSURE_NORMAL
            | DISPATCH_MEMORYPRESSURE_WARN
            | DISPATCH_MEMORYPRESSURE_CRITICAL;
        let source = unsafe {
            let queue = dispatch_get_global_queue(QOS_CLASS_UTILITY, 0);
            dispatch_source_create(&raw const DISPATCH_SOURCE_TYPE_MEMORYPRESSURE, 0, mask, queue)
        };
        let Some(source) = NonNull::new(source) else {
            return Err(MemoryPressureUnavailable::SourceRefused(
                "dispatch_source_create returned NULL".to_owned(),
            ));
        };
        let source = source.as_ptr();
        let context = Box::into_raw(Box::new(Context { source, sink }));
        unsafe {
            dispatch_set_context(source, context.cast());
            dispatch_source_set_event_handler_f(source, pressure_changed);
            dispatch_source_set_cancel_handler_f(source, source_cancelled);
            dispatch_resume(source);
        }
        Ok(MemoryPressureWatch::new(ArmedSource(source)))
    }
}

struct ArmedSource(DispatchObject);

unsafe impl Send for ArmedSource {}

impl Drop for ArmedSource {
    fn drop(&mut self) {
        unsafe {
            dispatch_source_cancel(self.0);
            dispatch_release(self.0);
        }
    }
}

unsafe extern "C" fn pressure_changed(context: *mut c_void) {
    let context = unsafe { &*context.cast::<Context>() };
    let data = unsafe { dispatch_source_get_data(context.source) };
    let Some(level) = level_from_dispatch_data(data) else {
        return;
    };
    let delivered =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (context.sink)(level)));
    if delivered.is_err() {
        tracing::error!(target: "riseonly", ?level, "memory warning handler panicked");
    }
}

unsafe extern "C" fn source_cancelled(context: *mut c_void) {
    drop(unsafe { Box::from_raw(context.cast::<Context>()) });
}
