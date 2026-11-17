use std::ffi::{c_int, c_uint, c_void};
use std::path::Path;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, NSObject};
use objc2::{define_class, msg_send, AnyThread as _, ClassType, DefinedClass};
use objc2_foundation::{
    NSArray, NSData, NSError, NSNumber, NSString, NSURLIsExcludedFromBackupKey,
    NSURLVolumeAvailableCapacityForImportantUsageKey, NSURL,
};

use crate::crash_diagnostics::*;

#[link(name = "MetricKit", kind = "framework")]
unsafe extern "C" {}

const MACH_TASK_BASIC_INFO: u32 = 20;
const KERN_SUCCESS: c_int = 0;
const CTL_KERN: c_int = 1;
const KERN_PROC: c_int = 14;
const KERN_PROC_PID: c_int = 1;

/// mach_task_basic_info, declared under pragma pack 4 in mach task_info.h
#[repr(C, packed(4))]
#[derive(Clone, Copy, Default)]
struct MachTaskBasicInfo {
    _virtual_size: u64,
    resident_size: u64,
    _resident_size_max: u64,
    _user_time: [i32; 2],
    _system_time: [i32; 2],
    _policy: i32,
    _suspend_count: i32,
}

unsafe extern "C" {
    static mach_task_self_: u32;
    fn task_info(target_task: u32, flavor: u32, task_info_out: *mut i32, count: *mut u32) -> c_int;
    fn sysctl(
        name: *mut c_int,
        name_len: c_uint,
        old: *mut c_void,
        old_len: *mut usize,
        new: *mut c_void,
        new_len: usize,
    ) -> c_int;
    fn getpid() -> c_int;
}

pub(crate) struct NativeCrashDiagnostics;

impl CrashDiagnosticsSource for NativeCrashDiagnostics {
    fn subscribe_system_reports(
        &self,
        sink: PayloadSink,
    ) -> Result<DiagnosticsSubscription, CrashDiagnosticsUnavailable> {
        let manager = shared_manager()?;
        let subscriber = MetricSubscriber::new(sink);
        unsafe {
            let _: () = msg_send![&*manager, addSubscriber: &*subscriber];
        }
        Ok(DiagnosticsSubscription::new(Subscription { subscriber }))
    }

    fn exclude_from_backup(
        &self,
        directory: &Path,
    ) -> Result<BackupExclusion, CrashDiagnosticsUnavailable> {
        let url = file_url(directory, DiagnosticsReading::BackupExclusion)?;
        let flag = NSNumber::numberWithBool(true);
        let value: &AnyObject = &flag;
        unsafe { url.setResourceValue_forKey_error(Some(value), NSURLIsExcludedFromBackupKey) }
            .map_err(|error| refused(DiagnosticsReading::BackupExclusion, &error))?;
        Ok(BackupExclusion::Excluded)
    }

    fn resident_memory_bytes(&self) -> Result<u64, CrashDiagnosticsUnavailable> {
        let mut info = MachTaskBasicInfo::default();
        let mut count = (size_of::<MachTaskBasicInfo>() / size_of::<i32>()) as u32;
        let result = unsafe {
            task_info(
                mach_task_self_,
                MACH_TASK_BASIC_INFO,
                (&raw mut info).cast(),
                &raw mut count,
            )
        };
        if result != KERN_SUCCESS {
            return Err(CrashDiagnosticsUnavailable::SystemRefused {
                what: DiagnosticsReading::ResidentMemory,
                detail: format!("task_info returned {result}"),
            });
        }
        Ok(info.resident_size)
    }

    fn process_start_seconds_since_1970(&self) -> Result<f64, CrashDiagnosticsUnavailable> {
        let pid = unsafe { getpid() };
        let mut mib = [CTL_KERN, KERN_PROC, KERN_PROC_PID, pid];
        let mut size = 0usize;
        let sized = unsafe {
            sysctl(
                mib.as_mut_ptr(),
                4,
                std::ptr::null_mut(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if sized != 0 || size < 16 {
            return Err(start_refused(format!("sysctl size query returned {sized}, size {size}")));
        }
        let mut buffer = vec![0u64; size.div_ceil(8)];
        let mut written = buffer.len() * 8;
        let read = unsafe {
            sysctl(
                mib.as_mut_ptr(),
                4,
                buffer.as_mut_ptr().cast(),
                &raw mut written,
                std::ptr::null_mut(),
                0,
            )
        };
        if read != 0 || written < 16 {
            return Err(start_refused(format!("sysctl returned {read}, wrote {written} bytes")));
        }
        let seconds = buffer[0] as i64;
        let micros = (buffer[1] & 0xffff_ffff) as u32 as i32;
        if seconds <= 0 || !(0..1_000_000).contains(&micros) {
            return Err(start_refused(format!("p_starttime {seconds} s {micros} us")));
        }
        Ok(seconds as f64 + f64::from(micros) / 1_000_000.0)
    }

    fn available_capacity_for_important_usage(
        &self,
        path: &Path,
    ) -> Result<u64, CrashDiagnosticsUnavailable> {
        let url = file_url(path, DiagnosticsReading::AvailableCapacity)?;
        let mut value: Option<Retained<AnyObject>> = None;
        unsafe {
            url.getResourceValue_forKey_error(
                &mut value,
                NSURLVolumeAvailableCapacityForImportantUsageKey,
            )
        }
        .map_err(|error| refused(DiagnosticsReading::AvailableCapacity, &error))?;
        let number = value
            .ok_or_else(|| CrashDiagnosticsUnavailable::SystemRefused {
                what: DiagnosticsReading::AvailableCapacity,
                detail: "no value for the volume".to_owned(),
            })?
            .downcast::<NSNumber>()
            .map_err(|_| CrashDiagnosticsUnavailable::SystemRefused {
                what: DiagnosticsReading::AvailableCapacity,
                detail: "the value is not a number".to_owned(),
            })?;
        let bytes = number.longLongValue();
        u64::try_from(bytes).map_err(|_| CrashDiagnosticsUnavailable::SystemRefused {
            what: DiagnosticsReading::AvailableCapacity,
            detail: format!("{bytes} bytes"),
        })
    }
}

fn start_refused(detail: String) -> CrashDiagnosticsUnavailable {
    CrashDiagnosticsUnavailable::SystemRefused {
        what: DiagnosticsReading::ProcessStart,
        detail,
    }
}

fn refused(what: DiagnosticsReading, error: &NSError) -> CrashDiagnosticsUnavailable {
    CrashDiagnosticsUnavailable::SystemRefused {
        what,
        detail: error.localizedDescription().to_string(),
    }
}

/// file url for path, foundation paths are utf8
pub(crate) fn file_url(
    path: &Path,
    what: DiagnosticsReading,
) -> Result<Retained<NSURL>, CrashDiagnosticsUnavailable> {
    let text = path.to_str().ok_or_else(|| CrashDiagnosticsUnavailable::SystemRefused {
        what,
        detail: format!("{} is not UTF-8", path.display()),
    })?;
    Ok(NSURL::fileURLWithPath(&NSString::from_str(text)))
}

/// the shared metric manager, always there on macos 13 which is the minimum
/// the check names the gap if the framework ever goes missing
fn shared_manager() -> Result<Retained<AnyObject>, CrashDiagnosticsUnavailable> {
    let class = AnyClass::get(c"MXMetricManager").ok_or(
        CrashDiagnosticsUnavailable::SystemTooOld {
            what: DiagnosticsReading::SystemReports,
            needs: "MetricKit, macOS 12",
        },
    )?;
    let manager: Option<Retained<AnyObject>> = unsafe { msg_send![class, sharedManager] };
    manager.ok_or_else(|| CrashDiagnosticsUnavailable::SystemRefused {
        what: DiagnosticsReading::SystemReports,
        detail: "MXMetricManager.sharedManager is nil".to_owned(),
    })
}

/// hands one delivery to the sink without letting a panic unwind into metrickit (that aborts)
/// the delivery is lost and logged instead
fn deliver(sink: &PayloadSink, kind: PayloadKind, payloads: &NSArray) {
    let collected: Vec<DiagnosticPayload> = payloads
        .iter()
        .filter_map(|payload| {
            let selector = objc2::sel!(JSONRepresentation);
            let responds: bool = unsafe { msg_send![&*payload, respondsToSelector: selector] };
            if !responds {
                tracing::error!(target: "riseonly", kind = kind.name(), "diagnostics payload without JSONRepresentation");
                return None;
            }
            let data: Option<Retained<NSData>> = unsafe { msg_send![&*payload, JSONRepresentation] };
            data.map(|data| DiagnosticPayload { kind, json: data.to_vec() })
        })
        .collect();
    if collected.is_empty() {
        return;
    }
    let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(collected)));
    if delivered.is_err() {
        tracing::error!(target: "riseonly", kind = kind.name(), "diagnostics handler panicked");
    }
}

struct SubscriberIvars {
    sink: PayloadSink,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseMetricSubscriber"]
    #[ivars = SubscriberIvars]
    struct MetricSubscriber;

    impl MetricSubscriber {
        #[unsafe(method(didReceiveMetricPayloads:))]
        fn did_receive_metric_payloads(&self, payloads: &NSArray) {
            deliver(&self.ivars().sink, PayloadKind::Metric, payloads);
        }

        #[unsafe(method(didReceiveDiagnosticPayloads:))]
        fn did_receive_diagnostic_payloads(&self, payloads: &NSArray) {
            deliver(&self.ivars().sink, PayloadKind::Diagnostic, payloads);
        }
    }
);

static CONFORMANCE: OnceLock<bool> = OnceLock::new();

impl MetricSubscriber {
    fn new(sink: PayloadSink) -> Retained<Self> {
        declare_conformance();
        let this = Self::alloc().set_ivars(SubscriberIvars { sink });
        unsafe { msg_send![super(this), init] }
    }
}

fn declare_conformance() -> bool {
    *CONFORMANCE.get_or_init(|| {
        let Some(protocol) = AnyProtocol::get(c"MXMetricManagerSubscriber") else {
            tracing::warn!(target: "riseonly", "MXMetricManagerSubscriber is not registered");
            return false;
        };
        let class: *const AnyClass = MetricSubscriber::class();
        unsafe { objc2::ffi::class_addProtocol(class.cast_mut(), protocol).as_bool() }
    })
}

/// unsubscribes from metrickit when the subscription drops
struct Subscription {
    subscriber: Retained<MetricSubscriber>,
}

unsafe impl Send for Subscription {}

impl Drop for Subscription {
    fn drop(&mut self) {
        match shared_manager() {
            Ok(manager) => unsafe {
                let _: () = msg_send![&*manager, removeSubscriber: &*self.subscriber];
            },
            Err(reason) => {
                tracing::warn!(target: "riseonly", %reason, "metric subscriber not removed");
            }
        }
    }
}
