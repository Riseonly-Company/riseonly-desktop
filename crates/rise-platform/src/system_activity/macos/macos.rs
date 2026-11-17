use std::ffi::{c_char, c_int, c_ulong, c_void};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AnyThread as _, DefinedClass};
use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceWillSleepNotification};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use parking_lot::Mutex;

use crate::system_activity::*;

type NwObject = *mut c_void;
type DispatchQueue = *mut c_void;

const NW_PATH_STATUS_SATISFIED: c_int = 1;

#[link(name = "Network", kind = "framework")]
unsafe extern "C" {
    fn nw_path_monitor_create() -> NwObject;
    fn nw_path_monitor_set_queue(monitor: NwObject, queue: DispatchQueue);
    fn nw_path_monitor_set_update_handler(monitor: NwObject, handler: *const PathUpdateBlock);
    fn nw_path_monitor_start(monitor: NwObject);
    fn nw_path_monitor_cancel(monitor: NwObject);
    fn nw_path_get_status(path: NwObject) -> c_int;
    fn nw_release(object: NwObject);
}

unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attributes: *const c_void) -> DispatchQueue;
    fn dispatch_release(object: DispatchQueue);
    static _NSConcreteGlobalBlock: [u8; 0];
}

pub(crate) struct NativeSystemActivity;

impl SystemActivitySource for NativeSystemActivity {
    fn watch_sleep(&self, sink: EventSink) -> Result<ActivityWatch, SystemActivityUnavailable> {
        let observer = SleepObserver::new(sink);
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(willSleep:),
                Some(NSWorkspaceWillSleepNotification),
                None,
            );
            center.addObserver_selector_name_object(
                &observer,
                sel!(didWake:),
                Some(NSWorkspaceDidWakeNotification),
                None,
            );
        }
        Ok(ActivityWatch::new(SleepRegistration { center, observer }))
    }

    fn watch_network_path(
        &self,
        sink: EventSink,
    ) -> Result<ActivityWatch, SystemActivityUnavailable> {
        let id = path_subscribe(sink)?;
        Ok(ActivityWatch::new(PathSubscription(id)))
    }
}

/// calls the sink but catches panics so they dont unwind into appkit and abort
/// event is lost and logged
fn deliver(sink: &EventSink, event: SystemActivityEvent) {
    let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(event)));
    if delivered.is_err() {
        tracing::error!(target: "riseonly", ?event, "system activity handler panicked");
    }
}

struct ObserverIvars {
    sink: EventSink,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseSystemSleepObserver"]
    #[ivars = ObserverIvars]
    struct SleepObserver;

    impl SleepObserver {
        #[unsafe(method(willSleep:))]
        fn will_sleep(&self, _notification: &NSNotification) {
            deliver(&self.ivars().sink, SystemActivityEvent::WillSleep);
        }

        #[unsafe(method(didWake:))]
        fn did_wake(&self, _notification: &NSNotification) {
            deliver(&self.ivars().sink, SystemActivityEvent::DidWake);
        }
    }
);

impl SleepObserver {
    fn new(sink: EventSink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ObserverIvars { sink });
        unsafe { msg_send![super(this), init] }
    }
}

struct SleepRegistration {
    center: Retained<NSNotificationCenter>,
    observer: Retained<SleepObserver>,
}

unsafe impl Send for SleepRegistration {}

impl Drop for SleepRegistration {
    fn drop(&mut self) {
        let observer: &AnyObject = &self.observer;
        unsafe { self.center.removeObserver(observer) };
    }
}

const BLOCK_IS_GLOBAL: c_int = 1 << 28;
const BLOCK_HAS_SIGNATURE: c_int = 1 << 30;

#[repr(C)]
struct BlockDescriptor {
    reserved: c_ulong,
    size: c_ulong,
    signature: *const c_char,
}

#[repr(C)]
struct PathUpdateBlock {
    isa: *const c_void,
    flags: c_int,
    reserved: c_int,
    invoke: unsafe extern "C" fn(block: *const PathUpdateBlock, path: NwObject),
    descriptor: *const BlockDescriptor,
}

unsafe impl Sync for BlockDescriptor {}
unsafe impl Sync for PathUpdateBlock {}

static PATH_UPDATE_DESCRIPTOR: BlockDescriptor = BlockDescriptor {
    reserved: 0,
    size: std::mem::size_of::<PathUpdateBlock>() as c_ulong,
    signature: c"v16@?0@8".as_ptr(),
};

static PATH_UPDATE_BLOCK: PathUpdateBlock = PathUpdateBlock {
    isa: (&raw const _NSConcreteGlobalBlock).cast(),
    flags: BLOCK_IS_GLOBAL | BLOCK_HAS_SIGNATURE,
    reserved: 0,
    invoke: path_updated,
    descriptor: &raw const PATH_UPDATE_DESCRIPTOR,
};

/// the one monitor plus its subscribers, monitor runs while anyone is subscribed
struct PathRegistry {
    next_id: u64,
    sinks: Vec<(u64, EventSink)>,
    last_satisfied: Option<bool>,
    monitor: Option<RunningMonitor>,
}

static PATH_REGISTRY: Mutex<PathRegistry> = Mutex::new(PathRegistry {
    next_id: 0,
    sinks: Vec::new(),
    last_satisfied: None,
    monitor: None,
});

/// a started monitor and the serial queue its handler runs on
struct RunningMonitor {
    monitor: NwObject,
    queue: DispatchQueue,
}

unsafe impl Send for RunningMonitor {}

impl RunningMonitor {
    fn start() -> Result<Self, SystemActivityUnavailable> {
        let queue = unsafe {
            dispatch_queue_create(
                c"com.riseonly.system-activity.network-path".as_ptr(),
                std::ptr::null(),
            )
        };
        if queue.is_null() {
            return Err(path_refused("dispatch_queue_create returned NULL"));
        }
        let monitor = unsafe { nw_path_monitor_create() };
        if monitor.is_null() {
            unsafe { dispatch_release(queue) };
            return Err(path_refused("nw_path_monitor_create returned NULL"));
        }
        unsafe {
            nw_path_monitor_set_queue(monitor, queue);
            nw_path_monitor_set_update_handler(monitor, &raw const PATH_UPDATE_BLOCK);
            nw_path_monitor_start(monitor);
        }
        Ok(Self { monitor, queue })
    }
}

impl Drop for RunningMonitor {
    fn drop(&mut self) {
        unsafe {
            nw_path_monitor_cancel(self.monitor);
            nw_release(self.monitor);
            dispatch_release(self.queue);
        }
    }
}

fn path_refused(detail: &str) -> SystemActivityUnavailable {
    SystemActivityUnavailable::SystemRefused {
        what: SystemActivityKind::NetworkPath,
        detail: detail.to_owned(),
    }
}

fn path_subscribe(sink: EventSink) -> Result<u64, SystemActivityUnavailable> {
    let mut registry = PATH_REGISTRY.lock();
    if registry.monitor.is_none() {
        registry.monitor = Some(RunningMonitor::start()?);
        registry.last_satisfied = None;
    }
    let id = registry.next_id;
    registry.next_id = registry.next_id.wrapping_add(1);
    let known = registry.last_satisfied;
    registry.sinks.push((id, Arc::clone(&sink)));
    drop(registry);
    if let Some(satisfied) = known {
        deliver(&sink, SystemActivityEvent::NetworkPath { satisfied });
    }
    Ok(id)
}

struct PathSubscription(u64);

impl Drop for PathSubscription {
    fn drop(&mut self) {
        let stopped = {
            let mut registry = PATH_REGISTRY.lock();
            registry.sinks.retain(|(id, _)| *id != self.0);
            if registry.sinks.is_empty() {
                registry.last_satisfied = None;
                registry.monitor.take()
            } else {
                None
            }
        };
        drop(stopped);
    }
}

unsafe extern "C" fn path_updated(_block: *const PathUpdateBlock, path: NwObject) {
    let satisfied = unsafe { nw_path_get_status(path) } == NW_PATH_STATUS_SATISFIED;
    let sinks: Vec<EventSink> = {
        let mut registry = PATH_REGISTRY.lock();
        if registry.monitor.is_none() {
            return;
        }
        registry.last_satisfied = Some(satisfied);
        registry.sinks.iter().map(|(_, sink)| Arc::clone(sink)).collect()
    };
    for sink in &sinks {
        deliver(sink, SystemActivityEvent::NetworkPath { satisfied });
    }
}
