use std::ffi::{c_char, c_int, c_long, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::crash_diagnostics::*;

const SC_CLK_TCK: c_int = 2;
const SC_PAGESIZE: c_int = 30;

/// statvfs of glibc and musl on 64 bit linux (x86_64 and aarch64), the only targets
/// every unsigned long and fsblkcnt_t member is 64 bits there
#[repr(C)]
#[derive(Default)]
struct StatVfs {
    f_bsize: u64,
    f_frsize: u64,
    _f_blocks: u64,
    _f_bfree: u64,
    f_bavail: u64,
    _f_files: u64,
    _f_ffree: u64,
    _f_favail: u64,
    _f_fsid: u64,
    _f_flag: u64,
    _f_namemax: u64,
    _spare: [c_int; 6],
}

const _: () = assert!(size_of::<StatVfs>() == 112, "struct statvfs layout of 64-bit Linux");

unsafe extern "C" {
    fn sysconf(name: c_int) -> c_long;
    fn statvfs(path: *const c_char, buf: *mut StatVfs) -> c_int;
}

pub(crate) struct NativeCrashDiagnostics;

fn read_proc(path: &str, what: DiagnosticsReading) -> Result<String, CrashDiagnosticsUnavailable> {
    std::fs::read_to_string(path).map_err(|error| CrashDiagnosticsUnavailable::SystemRefused {
        what,
        detail: format!("{path}: {error}"),
    })
}

fn sysconf_value(
    name: c_int,
    label: &str,
    what: DiagnosticsReading,
) -> Result<u64, CrashDiagnosticsUnavailable> {
    let value = unsafe { sysconf(name) };
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CrashDiagnosticsUnavailable::SystemRefused {
            what,
            detail: format!("sysconf({label}) returned {value}"),
        })
}

impl CrashDiagnosticsSource for NativeCrashDiagnostics {
    fn resident_memory_bytes(&self) -> Result<u64, CrashDiagnosticsUnavailable> {
        let what = DiagnosticsReading::ResidentMemory;
        let page_size = sysconf_value(SC_PAGESIZE, "_SC_PAGESIZE", what)?;
        resident_bytes_from_statm(&read_proc("/proc/self/statm", what)?, page_size)
    }

    fn process_start_seconds_since_1970(&self) -> Result<f64, CrashDiagnosticsUnavailable> {
        let what = DiagnosticsReading::ProcessStart;
        let ticks = start_ticks_from_proc_pid_stat(&read_proc("/proc/self/stat", what)?)?;
        let boot = boot_seconds_from_proc_stat(&read_proc("/proc/stat", what)?)?;
        let ticks_per_second = sysconf_value(SC_CLK_TCK, "_SC_CLK_TCK", what)?;
        process_start_from_boot(boot, ticks, ticks_per_second)
    }

    fn available_capacity_for_important_usage(
        &self,
        path: &Path,
    ) -> Result<u64, CrashDiagnosticsUnavailable> {
        let what = DiagnosticsReading::AvailableCapacity;
        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            CrashDiagnosticsUnavailable::SystemRefused {
                what,
                detail: format!("{} holds a NUL byte", path.display()),
            }
        })?;
        let mut info = StatVfs::default();
        let result = unsafe { statvfs(c_path.as_ptr(), &raw mut info) };
        if result != 0 {
            return Err(CrashDiagnosticsUnavailable::SystemRefused {
                what,
                detail: format!("statvfs {}: {}", path.display(), std::io::Error::last_os_error()),
            });
        }
        available_bytes_from_statvfs(info.f_bavail, info.f_frsize, info.f_bsize)
    }
}
