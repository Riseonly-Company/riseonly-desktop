use std::time::Duration;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Diagnostics::Debug::IsDebuggerPresent;
use windows::Win32::System::SystemInformation::{
    GetPhysicallyInstalledSystemMemory, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

use super::process_info::*;

pub(super) struct NativeProcessInfo;

impl ProcessInfoSource for NativeProcessInfo {
    fn physical_memory(&self) -> Result<u64, ProcessInfoUnavailable> {
        let mut kilobytes = 0u64;
        if unsafe { GetPhysicallyInstalledSystemMemory(&mut kilobytes) }.is_ok() && kilobytes > 0 {
            return Ok(kilobytes.saturating_mul(1024));
        }
        let mut status = MEMORYSTATUSEX {
            dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>())
                .unwrap_or(u32::MAX),
            ..MEMORYSTATUSEX::default()
        };
        unsafe { GlobalMemoryStatusEx(&mut status) }.map_err(|error| {
            ProcessInfoUnavailable::SystemRefused {
                what: ProcessReading::PhysicalMemory,
                detail: format!("GlobalMemoryStatusEx: {error}"),
            }
        })?;
        if status.ullTotalPhys == 0 {
            return Err(ProcessInfoUnavailable::SystemRefused {
                what: ProcessReading::PhysicalMemory,
                detail: "GlobalMemoryStatusEx reported no physical memory".to_owned(),
            });
        }
        Ok(status.ullTotalPhys)
    }

    fn system_uptime(&self) -> Result<Duration, ProcessInfoUnavailable> {
        let milliseconds = unsafe { GetTickCount64() };
        Ok(Duration::from_millis(milliseconds))
    }

    fn is_debugger_attached(&self) -> Result<bool, ProcessInfoUnavailable> {
        Ok(unsafe { IsDebuggerPresent() }.as_bool())
    }

    fn process_cpu_seconds(&self) -> Result<f64, ProcessInfoUnavailable> {
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
        .map_err(|error| ProcessInfoUnavailable::SystemRefused {
            what: ProcessReading::ProcessCpuSeconds,
            detail: format!("GetProcessTimes: {error}"),
        })?;
        Ok(cpu_seconds_from_filetimes(
            kernel.dwHighDateTime,
            kernel.dwLowDateTime,
            user.dwHighDateTime,
            user.dwLowDateTime,
        ))
    }
}
