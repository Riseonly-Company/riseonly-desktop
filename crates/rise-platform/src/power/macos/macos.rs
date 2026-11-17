use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AnyThread as _, DefinedClass};
use objc2_foundation::{
    ns_string, NSArray, NSDictionary, NSNotification, NSNotificationCenter, NSNumber,
    NSProcessInfo, NSProcessInfoPowerStateDidChangeNotification, NSString,
};

use crate::power::*;

type CfTypeRef = *const c_void;
type CfRunLoopRef = *mut c_void;
type CfRunLoopSourceRef = *mut c_void;
type CfStringRef = *const c_void;
type PowerSourcesCallback = unsafe extern "C" fn(context: *mut c_void);

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    #[link_name = "IOPSCopyPowerSourcesInfo"]
    fn iops_copy_power_sources_info() -> CfTypeRef;
    #[link_name = "IOPSCopyPowerSourcesList"]
    fn iops_copy_power_sources_list(blob: CfTypeRef) -> CfTypeRef;
    #[link_name = "IOPSGetPowerSourceDescription"]
    fn iops_get_power_source_description(blob: CfTypeRef, source: CfTypeRef) -> CfTypeRef;
    #[link_name = "IOPSNotificationCreateRunLoopSource"]
    fn iops_notification_create_run_loop_source(
        callback: PowerSourcesCallback,
        context: *mut c_void,
    ) -> CfRunLoopSourceRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    #[link_name = "kCFRunLoopDefaultMode"]
    static CF_RUN_LOOP_DEFAULT_MODE: CfStringRef;
    #[link_name = "CFRunLoopGetCurrent"]
    fn cf_run_loop_get_current() -> CfRunLoopRef;
    #[link_name = "CFRunLoopAddSource"]
    fn cf_run_loop_add_source(
        run_loop: CfRunLoopRef,
        source: CfRunLoopSourceRef,
        mode: CfStringRef,
    );
    #[link_name = "CFRunLoopRunInMode"]
    fn cf_run_loop_run_in_mode(mode: CfStringRef, seconds: f64, return_after_source: u8) -> i32;
    #[link_name = "CFRunLoopStop"]
    fn cf_run_loop_stop(run_loop: CfRunLoopRef);
    #[link_name = "CFRunLoopSourceInvalidate"]
    fn cf_run_loop_source_invalidate(source: CfRunLoopSourceRef);
    #[link_name = "CFRetain"]
    fn cf_retain(object: CfTypeRef) -> CfTypeRef;
    #[link_name = "CFRelease"]
    fn cf_release(object: CfTypeRef);
}

pub(crate) struct NativePower;

impl PowerSource for NativePower {
    fn low_power_mode(&self) -> Result<bool, PowerUnavailable> {
        Ok(NSProcessInfo::processInfo().isLowPowerModeEnabled())
    }

    fn battery(&self) -> Result<BatteryReading, PowerUnavailable> {
        let blob = unsafe {
            Retained::from_raw(iops_copy_power_sources_info().cast_mut().cast::<AnyObject>())
        }
        .ok_or_else(|| {
            refused(PowerReadingKind::Battery, "IOPSCopyPowerSourcesInfo returned NULL")
        })?;
        let blob_ref: CfTypeRef = Retained::as_ptr(&blob).cast();
        let list = unsafe {
            Retained::from_raw(
                iops_copy_power_sources_list(blob_ref)
                    .cast_mut()
                    .cast::<NSArray<AnyObject>>(),
            )
        }
        .ok_or_else(|| {
            refused(PowerReadingKind::Battery, "IOPSCopyPowerSourcesList returned NULL")
        })?;
        for source in list.iter() {
            let description = unsafe {
                iops_get_power_source_description(blob_ref, Retained::as_ptr(&source).cast())
            };
            if description.is_null() {
                continue;
            }
            let description =
                unsafe { &*description.cast::<NSDictionary<NSString, AnyObject>>() };
            if let Some(reading) = internal_battery(description) {
                return Ok(reading);
            }
        }
        Ok(BatteryReading::NO_BATTERY)
    }

    fn watch(&self, sink: ChangeSink) -> Result<PowerWatch, PowerUnavailable> {
        let sink = Arc::new(sink);
        let battery = PowerSourcesThread::start(Arc::clone(&sink))?;
        let observer = PowerStateObserver::new(sink);
        let center = NSNotificationCenter::defaultCenter();
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(powerStateDidChange:),
                Some(NSProcessInfoPowerStateDidChangeNotification),
                None,
            );
        }
        Ok(PowerWatch::new(Registration {
            center,
            observer,
            _battery: battery,
        }))
    }
}

fn refused(what: PowerReadingKind, detail: &str) -> PowerUnavailable {
    PowerUnavailable::SystemRefused {
        what,
        detail: detail.to_owned(),
    }
}

fn internal_battery(description: &NSDictionary<NSString, AnyObject>) -> Option<BatteryReading> {
    if !string_is(description, ns_string!("Type"), ns_string!("InternalBattery")) {
        return None;
    }
    if flag(description, ns_string!("Is Present")) == Some(false) {
        return None;
    }
    let percent = match (
        number(description, ns_string!("Current Capacity")),
        number(description, ns_string!("Max Capacity")),
    ) {
        (Some(current), Some(max)) => battery_percent_from_capacity(current, max),
        _ => None,
    };
    let on_external_power =
        string_is(description, ns_string!("Power Source State"), ns_string!("AC Power"));
    let is_charging = flag(description, ns_string!("Is Charging")).unwrap_or(false);
    let is_charged = flag(description, ns_string!("Is Charged")).unwrap_or(false)
        || (percent == Some(100) && !is_charging);
    Some(BatteryReading {
        percent,
        state: BatteryState::from_power_source(on_external_power, is_charged),
    })
}

fn string_is(
    description: &NSDictionary<NSString, AnyObject>,
    key: &NSString,
    expected: &NSString,
) -> bool {
    description.objectForKey(key).is_some_and(|value| {
        value
            .downcast_ref::<NSString>()
            .is_some_and(|text| text.isEqualToString(expected))
    })
}

fn number(description: &NSDictionary<NSString, AnyObject>, key: &NSString) -> Option<i64> {
    let value = description.objectForKey(key)?;
    let number = value.downcast_ref::<NSNumber>()?;
    i64::try_from(number.integerValue()).ok()
}

fn flag(description: &NSDictionary<NSString, AnyObject>, key: &NSString) -> Option<bool> {
    let value = description.objectForKey(key)?;
    Some(value.downcast_ref::<NSNumber>()?.boolValue())
}

/// calls the sink and stops a panic from unwinding into foundation (that aborts the process)
/// the change is lost and logged instead
fn deliver(sink: &ChangeSink) {
    let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink()));
    if delivered.is_err() {
        tracing::error!(target: "riseonly", "power change handler panicked");
    }
}

/// this thread run loop, retained so the watch can stop it from any thread
struct RunLoopHandle(CfRunLoopRef);

unsafe impl Send for RunLoopHandle {}

impl Drop for RunLoopHandle {
    fn drop(&mut self) {
        unsafe { cf_release(self.0.cast_const()) };
    }
}

/// thread whose run loop carries the power sources notification
/// so the callback never waits on main and the context is freed on the only thread that reads it
struct PowerSourcesThread {
    run_loop: RunLoopHandle,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PowerSourcesThread {
    fn start(sink: Arc<ChangeSink>) -> Result<Self, PowerUnavailable> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("rise-power-sources".to_owned())
            .spawn(move || run_power_sources(sink, &thread_stop, &ready_tx))
            .map_err(|error| {
                refused(
                    PowerReadingKind::Changes,
                    &format!("could not start the power sources thread: {error}"),
                )
            })?;
        match ready_rx.recv() {
            Ok(Ok(run_loop)) => Ok(Self {
                run_loop,
                stop,
                thread: Some(thread),
            }),
            Ok(Err(detail)) => {
                join_logged(thread);
                Err(refused(PowerReadingKind::Changes, &detail))
            }
            Err(_) => {
                join_logged(thread);
                Err(refused(
                    PowerReadingKind::Changes,
                    "the power sources thread ended before it was ready",
                ))
            }
        }
    }
}

impl Drop for PowerSourcesThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        unsafe { cf_run_loop_stop(self.run_loop.0) };
        if let Some(thread) = self.thread.take() {
            join_logged(thread);
        }
    }
}

fn join_logged(thread: JoinHandle<()>) {
    if thread.join().is_err() {
        tracing::error!(target: "riseonly", "the power sources thread panicked");
    }
}

fn run_power_sources(
    sink: Arc<ChangeSink>,
    stop: &AtomicBool,
    ready: &mpsc::Sender<Result<RunLoopHandle, String>>,
) {
    let context = Box::into_raw(Box::new(sink));
    let source = unsafe {
        iops_notification_create_run_loop_source(power_sources_changed, context.cast())
    };
    if source.is_null() {
        drop(unsafe { Box::from_raw(context) });
        if ready
            .send(Err("IOPSNotificationCreateRunLoopSource returned NULL".to_owned()))
            .is_err()
        {
            tracing::error!(target: "riseonly", "power sources thread lost its starter");
        }
        return;
    }
    let run_loop = unsafe {
        let run_loop = cf_run_loop_get_current();
        cf_retain(run_loop.cast_const());
        cf_run_loop_add_source(run_loop, source, CF_RUN_LOOP_DEFAULT_MODE);
        run_loop
    };
    if ready.send(Ok(RunLoopHandle(run_loop))).is_err() {
        stop.store(true, Ordering::Release);
    }
    while !stop.load(Ordering::Acquire) {
        unsafe { cf_run_loop_run_in_mode(CF_RUN_LOOP_DEFAULT_MODE, 1.0, 0) };
    }
    unsafe {
        cf_run_loop_source_invalidate(source);
        cf_release(source.cast_const());
        drop(Box::from_raw(context));
    }
}

unsafe extern "C" fn power_sources_changed(context: *mut c_void) {
    let sink = unsafe { &*context.cast::<Arc<ChangeSink>>() };
    deliver(sink);
}

struct ObserverIvars {
    sink: Arc<ChangeSink>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RisePowerStateObserver"]
    #[ivars = ObserverIvars]
    struct PowerStateObserver;

    impl PowerStateObserver {
        #[unsafe(method(powerStateDidChange:))]
        fn power_state_did_change(&self, _notification: &NSNotification) {
            deliver(&self.ivars().sink);
        }
    }
);

impl PowerStateObserver {
    fn new(sink: Arc<ChangeSink>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ObserverIvars { sink });
        unsafe { msg_send![super(this), init] }
    }
}

/// unregisters the observer and stops the power sources thread when the watch drops
struct Registration {
    center: Retained<NSNotificationCenter>,
    observer: Retained<PowerStateObserver>,
    _battery: PowerSourcesThread,
}

unsafe impl Send for Registration {}

impl Drop for Registration {
    fn drop(&mut self) {
        let observer: &AnyObject = &self.observer;
        unsafe { self.center.removeObserver(observer) };
    }
}
