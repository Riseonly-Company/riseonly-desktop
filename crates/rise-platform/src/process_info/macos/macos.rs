use std::ffi::{c_int, c_uint, c_void};
use std::ptr;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AnyThread as _, DefinedClass};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSProcessInfo, NSProcessInfoThermalState,
    NSProcessInfoThermalStateDidChangeNotification,
};

use crate::process_info::*;

pub(crate) struct NativeProcessInfo;

impl ProcessInfoSource for NativeProcessInfo {
    fn physical_memory(&self) -> Result<u64, ProcessInfoUnavailable> {
        let bytes = NSProcessInfo::processInfo().physicalMemory();
        if bytes == 0 {
            return Err(ProcessInfoUnavailable::SystemRefused {
                what: ProcessReading::PhysicalMemory,
                detail: "NSProcessInfo.physicalMemory answered 0".to_owned(),
            });
        }
        Ok(bytes)
    }

    fn system_uptime(&self) -> Result<std::time::Duration, ProcessInfoUnavailable> {
        uptime_from_seconds(NSProcessInfo::processInfo().systemUptime())
    }

    fn thermal_state(&self) -> Result<ThermalState, ProcessInfoUnavailable> {
        thermal_state_from(NSProcessInfo::processInfo().thermalState())
    }

    fn watch_thermal_state(
        &self,
        sink: ThermalSink,
    ) -> Result<ThermalStateWatch, ProcessInfoUnavailable> {
        let observer = ThermalObserver::new(sink);
        let center = NSNotificationCenter::defaultCenter();
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(thermalStateDidChange:),
                Some(NSProcessInfoThermalStateDidChangeNotification),
                None,
            );
        }
        Ok(ThermalStateWatch::new(Registration { center, observer }))
    }

    fn is_debugger_attached(&self) -> Result<bool, ProcessInfoUnavailable> {
        debugger_attached_from_sysctl()
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
            i64::from(usage.ru_utime.tv_usec),
            usage.ru_stime.tv_sec,
            i64::from(usage.ru_stime.tv_usec),
        ))
    }
}

const CTL_KERN: c_int = 1;
const KERN_PROC: c_int = 14;
const KERN_PROC_PID: c_int = 1;
const P_TRACED: c_int = 0x0000_0800;
const P_FLAG_OFFSET: usize = 32;
const RUSAGE_SELF: c_int = 0;

/// timeval layout on mac lp64, seconds are long and microseconds are 32 bits
#[repr(C)]
#[derive(Default)]
struct TimeVal {
    tv_sec: i64,
    tv_usec: i32,
}

/// rusage struct, the two times this reading uses then 14 longs it ignores
#[repr(C)]
#[derive(Default)]
struct RUsage {
    ru_utime: TimeVal,
    ru_stime: TimeVal,
    _counters: [i64; 14],
}

unsafe extern "C" {
    fn sysctl(
        name: *mut c_int,
        namelen: c_uint,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
    fn getrusage(who: c_int, usage: *mut RUsage) -> c_int;
}

fn debugger_attached_from_sysctl() -> Result<bool, ProcessInfoUnavailable> {
    let refused = |detail: String| ProcessInfoUnavailable::SystemRefused {
        what: ProcessReading::DebuggerAttached,
        detail,
    };
    let pid = c_int::try_from(std::process::id())
        .map_err(|_| refused(format!("pid {} does not fit pid_t", std::process::id())))?;
    let mut query = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid];
    let mut size = 0_usize;
    let sized = unsafe {
        sysctl(
            query.as_mut_ptr(),
            query.len() as c_uint,
            ptr::null_mut(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if sized != 0 {
        return Err(refused(format!(
            "sysctl kern.proc.pid size: {}",
            std::io::Error::last_os_error()
        )));
    }
    let mut info = vec![0_u64; size.div_ceil(8)];
    let mut info_size = info.len() * 8;
    let result = unsafe {
        sysctl(
            query.as_mut_ptr(),
            query.len() as c_uint,
            info.as_mut_ptr().cast(),
            &mut info_size,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(refused(format!(
            "sysctl kern.proc.pid: {}",
            std::io::Error::last_os_error()
        )));
    }
    if info_size < P_FLAG_OFFSET + std::mem::size_of::<c_int>() {
        return Err(refused(format!(
            "sysctl kern.proc.pid returned {info_size} bytes, too short for kp_proc.p_flag"
        )));
    }
    let bytes: Vec<u8> = info.iter().flat_map(|word| word.to_ne_bytes()).collect();
    let mut flag = [0_u8; 4];
    flag.copy_from_slice(&bytes[P_FLAG_OFFSET..P_FLAG_OFFSET + 4]);
    Ok(c_int::from_ne_bytes(flag) & P_TRACED != 0)
}

/// documented raw values 0..3, anything else is a state this build doesnt know
pub(crate) fn thermal_state_from(
    raw: NSProcessInfoThermalState,
) -> Result<ThermalState, ProcessInfoUnavailable> {
    ThermalState::from_raw_value(raw.0 as i64).ok_or_else(|| ProcessInfoUnavailable::SystemRefused {
        what: ProcessReading::ThermalState,
        detail: format!("unknown NSProcessInfoThermalState {}", raw.0),
    })
}

struct ObserverIvars {
    sink: ThermalSink,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseThermalStateObserver"]
    #[ivars = ObserverIvars]
    struct ThermalObserver;

    impl ThermalObserver {
        #[unsafe(method(thermalStateDidChange:))]
        fn thermal_state_did_change(&self, _notification: &NSNotification) {
            if let Ok(state) = thermal_state_from(NSProcessInfo::processInfo().thermalState()) {
                (self.ivars().sink)(state);
            }
        }
    }
);

impl ThermalObserver {
    fn new(sink: ThermalSink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ObserverIvars { sink });
        unsafe { msg_send![super(this), init] }
    }
}

/// unregisters the observer when the watch drops
struct Registration {
    center: Retained<NSNotificationCenter>,
    observer: Retained<ThermalObserver>,
}

unsafe impl Send for Registration {}

impl Drop for Registration {
    fn drop(&mut self) {
        let observer: &AnyObject = &self.observer;
        unsafe { self.center.removeObserver(observer) };
    }
}
