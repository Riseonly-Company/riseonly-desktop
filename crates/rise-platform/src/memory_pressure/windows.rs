use std::thread::JoinHandle;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Memory::{
    CreateMemoryResourceNotification, HighMemoryResourceNotification, LowMemoryResourceNotification,
    MEMORY_RESOURCE_NOTIFICATION_TYPE,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects};
use windows::core::PCWSTR;

use super::memory_pressure::*;

pub(super) struct NativeMemoryPressure;

impl MemoryPressureSource for NativeMemoryPressure {
    fn start(&self, sink: LevelSink) -> Result<MemoryPressureWatch, MemoryPressureUnavailable> {
        let low = notification(LowMemoryResourceNotification, "low")?;
        let high = notification(HighMemoryResourceNotification, "high")?;
        let stop = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map(OwnedHandle)
            .map_err(|error| MemoryPressureUnavailable::SourceRefused(format!("CreateEventW: {error}")))?;
        let handles = Handles { low, high, stop };
        let waited = handles.raw();
        let thread = std::thread::Builder::new()
            .name("rise-memory-pressure".to_owned())
            .spawn(move || wait_loop(waited, &sink))
            .map_err(|error| MemoryPressureUnavailable::SourceRefused(format!("wait thread: {error}")))?;
        Ok(MemoryPressureWatch::new(ArmedWatch {
            handles: Some(handles),
            thread: Some(thread),
        }))
    }
}

fn notification(
    kind: MEMORY_RESOURCE_NOTIFICATION_TYPE,
    name: &str,
) -> Result<OwnedHandle, MemoryPressureUnavailable> {
    unsafe { CreateMemoryResourceNotification(kind) }
        .map(OwnedHandle)
        .map_err(|error| {
            MemoryPressureUnavailable::SourceRefused(format!(
                "CreateMemoryResourceNotification({name}): {error}"
            ))
        })
}

/// one kernel handle this layer made, drop closes it
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct Handles {
    low: OwnedHandle,
    high: OwnedHandle,
    stop: OwnedHandle,
}

impl Handles {
    fn raw(&self) -> RawHandles {
        RawHandles {
            low: self.low.0,
            high: self.high.0,
            stop: self.stop.0,
        }
    }
}

/// handle values the wait thread borrows
/// the watch keeps the owning handles open until the thread is joined
#[derive(Clone, Copy)]
struct RawHandles {
    low: HANDLE,
    high: HANDLE,
    stop: HANDLE,
}

unsafe impl Send for RawHandles {}

unsafe impl Send for OwnedHandle {}

fn wait_loop(handles: RawHandles, sink: &LevelSink) {
    let mut level = MemoryPressureLevel::Normal;
    loop {
        let crossing = match level {
            MemoryPressureLevel::Normal => handles.low,
            MemoryPressureLevel::Warning | MemoryPressureLevel::Critical => handles.high,
        };
        let woke = unsafe { WaitForMultipleObjects(&[handles.stop, crossing], false, INFINITE) };
        if woke == WAIT_OBJECT_0 {
            return;
        }
        if woke.0 != WAIT_OBJECT_0.0 + 1 {
            tracing::error!(target: "riseonly", code = woke.0, "memory pressure wait failed; warnings stop");
            return;
        }
        level = match level {
            MemoryPressureLevel::Normal => MemoryPressureLevel::Warning,
            MemoryPressureLevel::Warning | MemoryPressureLevel::Critical => MemoryPressureLevel::Normal,
        };
        let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(level)));
        if delivered.is_err() {
            tracing::error!(target: "riseonly", ?level, "memory warning handler panicked");
        }
    }
}

struct ArmedWatch {
    handles: Option<Handles>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for ArmedWatch {
    fn drop(&mut self) {
        let Some(handles) = self.handles.take() else {
            return;
        };
        if let Err(error) = unsafe { SetEvent(handles.stop.0) } {
            tracing::error!(target: "riseonly", %error, "memory pressure stop event failed");
            std::mem::forget(handles);
            return;
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        drop(handles);
    }
}
