use std::ffi::{c_int, c_short, c_ulong};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rise_core::foundation::foundation_date::RiseClock;

use super::memory_pressure::*;
use super::psi::*;

const O_NONBLOCK: c_int = 0o4000;
const POLLPRI: c_short = 0x002;
const POLLERR: c_short = 0x008;
const POLLNVAL: c_short = 0x020;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

unsafe extern "C" {
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
}

pub(super) struct NativeMemoryPressure;

impl MemoryPressureSource for NativeMemoryPressure {
    fn start(&self, sink: LevelSink) -> Result<MemoryPressureWatch, MemoryPressureUnavailable> {
        let first = read_level()?;
        let mut armed = Vec::with_capacity(PsiTrigger::ALL.len());
        for trigger in PsiTrigger::ALL {
            armed.push((arm(&trigger)?, trigger.level));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("rise-memory-pressure".to_owned())
            .spawn(move || watch_loop(&armed, first, &stopped, &sink))
            .map_err(|error| MemoryPressureUnavailable::SourceRefused(format!("watch thread: {error}")))?;
        Ok(MemoryPressureWatch::new(StopOnDrop(stop)))
    }
}

struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn refused(step: PsiStep, error: &std::io::Error) -> MemoryPressureUnavailable {
    MemoryPressureUnavailable::SourceRefused(psi_refusal(step, error.raw_os_error(), &error.to_string()))
}

fn read_level() -> Result<MemoryPressureLevel, MemoryPressureUnavailable> {
    let text = std::fs::read_to_string(PSI_MEMORY_PATH).map_err(|error| refused(PsiStep::Open, &error))?;
    parse_psi(&text)
        .map(level_from_reading)
        .ok_or_else(|| MemoryPressureUnavailable::SourceRefused(psi_refusal(PsiStep::Read, None, "no some line")))
}

/// opens a descriptor and writes one trigger, the trigger lives until the descriptor closes
fn arm(trigger: &PsiTrigger) -> Result<File, MemoryPressureUnavailable> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_NONBLOCK)
        .open(PSI_MEMORY_PATH)
        .map_err(|error| refused(PsiStep::Open, &error))?;
    let mut command = trigger.command().into_bytes();
    command.push(0);
    file.write_all(&command).map_err(|error| refused(PsiStep::Trigger, &error))?;
    Ok(file)
}

fn watch_loop(
    armed: &[(File, MemoryPressureLevel)],
    first: MemoryPressureLevel,
    stop: &AtomicBool,
    sink: &LevelSink,
) {
    let mut tracker = PsiLevelTracker::default();
    if let Some(level) = tracker.on_reading(first, RiseClock::monotonic()) {
        deliver(sink, level);
    }
    let mut fds: Vec<PollFd> = armed
        .iter()
        .map(|(file, _)| PollFd {
            fd: file.as_raw_fd(),
            events: POLLPRI,
            revents: 0,
        })
        .collect();
    let count = c_ulong::try_from(fds.len()).unwrap_or(0);
    let tick = c_int::try_from(PSI_STOP_TICK.as_millis()).unwrap_or(c_int::MAX);
    while !stop.load(Ordering::Acquire) {
        for fd in &mut fds {
            fd.revents = 0;
        }
        let ready = unsafe { poll(fds.as_mut_ptr(), count, tick) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == ErrorKind::Interrupted {
                continue;
            }
            tracing::error!(target: "riseonly", %error, "memory pressure poll failed; warnings stop");
            return;
        }
        let now = RiseClock::monotonic();
        for (fd, (_, level)) in fds.iter().zip(armed) {
            if fd.revents & (POLLERR | POLLNVAL) != 0 {
                tracing::error!(target: "riseonly", revents = fd.revents, "memory pressure trigger failed; warnings stop");
                return;
            }
            if fd.revents & POLLPRI != 0
                && let Some(raised) = tracker.on_event(*level, now)
            {
                deliver(sink, raised);
            }
        }
        if tracker.recheck_due(now) {
            let level = match read_level() {
                Ok(level) => level,
                Err(error) => {
                    tracing::warn!(target: "riseonly", %error, "memory pressure reread failed");
                    tracker.level()
                }
            };
            if let Some(changed) = tracker.on_reading(level, now) {
                deliver(sink, changed);
            }
        }
    }
}

fn deliver(sink: &LevelSink, level: MemoryPressureLevel) {
    if catch_unwind(AssertUnwindSafe(|| sink(level))).is_err() {
        tracing::error!(target: "riseonly", ?level, "memory warning handler panicked");
    }
}
