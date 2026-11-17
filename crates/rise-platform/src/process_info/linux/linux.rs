use std::ffi::c_int;
use std::time::Duration;

use crate::process_info::*;

pub(crate) struct NativeProcessInfo;

const RUSAGE_SELF: c_int = 0;

/// timeval layout on the two 64 bit linux targets, both fields are long
#[repr(C)]
#[derive(Default)]
struct TimeVal {
    tv_sec: i64,
    tv_usec: i64,
}

/// getrusage struct, the two times this reading uses then 14 longs it ignores
#[repr(C)]
#[derive(Default)]
struct RUsage {
    ru_utime: TimeVal,
    ru_stime: TimeVal,
    _counters: [i64; 14],
}

unsafe extern "C" {
    fn getrusage(who: c_int, usage: *mut RUsage) -> c_int;
}

fn read_proc(path: &str, what: ProcessReading) -> Result<String, ProcessInfoUnavailable> {
    std::fs::read_to_string(path).map_err(|error| ProcessInfoUnavailable::SystemRefused {
        what,
        detail: format!("{path}: {error}"),
    })
}

impl ProcessInfoSource for NativeProcessInfo {
    fn physical_memory(&self) -> Result<u64, ProcessInfoUnavailable> {
        physical_memory_from_meminfo(&read_proc("/proc/meminfo", ProcessReading::PhysicalMemory)?)
    }

    fn system_uptime(&self) -> Result<Duration, ProcessInfoUnavailable> {
        system_uptime_from_proc_uptime(&read_proc("/proc/uptime", ProcessReading::SystemUptime)?)
    }

    fn is_debugger_attached(&self) -> Result<bool, ProcessInfoUnavailable> {
        debugger_attached_from_proc_status(&read_proc(
            "/proc/self/status",
            ProcessReading::DebuggerAttached,
        )?)
    }

    fn process_cpu_seconds(&self) -> Result<f64, ProcessInfoUnavailable> {
        let mut usage = RUsage::default();
        if unsafe { getrusage(RUSAGE_SELF, &mut usage) } != 0 {
            return Err(ProcessInfoUnavailable::SystemRefused {
                what: ProcessReading::ProcessCpuSeconds,
                detail: format!("getrusage: {}", std::io::Error::last_os_error()),
            });
        }
        Ok(cpu_seconds_from_timevals(
            usage.ru_utime.tv_sec,
            usage.ru_utime.tv_usec,
            usage.ru_stime.tv_sec,
            usage.ru_stime.tv_usec,
        ))
    }
}
