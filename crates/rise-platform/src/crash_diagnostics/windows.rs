use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

use super::crash_diagnostics::*;

pub(super) struct NativeCrashDiagnostics;

fn refused(what: DiagnosticsReading, error: &windows::core::Error) -> CrashDiagnosticsUnavailable {
    CrashDiagnosticsUnavailable::SystemRefused {
        what,
        detail: error.to_string(),
    }
}

impl CrashDiagnosticsSource for NativeCrashDiagnostics {
    fn resident_memory_bytes(&self) -> Result<u64, CrashDiagnosticsUnavailable> {
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters,
                size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
        }
        .map_err(|error| refused(DiagnosticsReading::ResidentMemory, &error))?;
        Ok(counters.WorkingSetSize as u64)
    }

    fn process_start_seconds_since_1970(&self) -> Result<f64, CrashDiagnosticsUnavailable> {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(|error| refused(DiagnosticsReading::ProcessStart, &error))?;
        process_start_from_filetime(creation.dwHighDateTime, creation.dwLowDateTime)
    }

    fn available_capacity_for_important_usage(
        &self,
        path: &Path,
    ) -> Result<u64, CrashDiagnosticsUnavailable> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut available: u64 = 0;
        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(wide.as_ptr()),
                Some(&mut available as *mut u64),
                None,
                None,
            )
        }
        .map_err(|error| refused(DiagnosticsReading::AvailableCapacity, &error))?;
        Ok(available)
    }
}
