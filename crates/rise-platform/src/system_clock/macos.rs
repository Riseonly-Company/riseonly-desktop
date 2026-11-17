use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AnyThread as _, DefinedClass, define_class, msg_send, sel};
use objc2_foundation::{
    NSCurrentLocaleDidChangeNotification, NSNotification, NSNotificationCenter,
    NSSystemTimeZoneDidChangeNotification,
};
use parking_lot::Mutex;

use super::system_clock::*;
use crate::system_activity::ActivityWatch;

pub(super) struct NativeSystemClock;

impl SystemClockSource for NativeSystemClock {
    fn watch(&self, sink: SystemClockSink) -> Result<ActivityWatch, SystemClockUnavailable> {
        let observer = ClockObserver::alloc().set_ivars(ClockObserverIvars { sink: Mutex::new(sink) });
        let observer: Retained<ClockObserver> = unsafe { msg_send![super(observer), init] };
        let center = NSNotificationCenter::defaultCenter();
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(timeZoneChanged:),
                Some(NSSystemTimeZoneDidChangeNotification),
                None,
            );
            center.addObserver_selector_name_object(
                &observer,
                sel!(localeChanged:),
                Some(NSCurrentLocaleDidChangeNotification),
                None,
            );
        }
        Ok(ActivityWatch::new(ClockRegistration { center, observer }))
    }
}

struct ClockObserverIvars {
    sink: Mutex<SystemClockSink>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseSystemClockObserver"]
    #[ivars = ClockObserverIvars]
    struct ClockObserver;

    impl ClockObserver {
        #[unsafe(method(timeZoneChanged:))]
        fn time_zone_changed(&self, _notification: &NSNotification) {
            (self.ivars().sink.lock())(SystemClockChange::TimeZone);
        }

        #[unsafe(method(localeChanged:))]
        fn locale_changed(&self, _notification: &NSNotification) {
            (self.ivars().sink.lock())(SystemClockChange::Locale);
        }
    }
);

struct ClockRegistration {
    center: Retained<NSNotificationCenter>,
    observer: Retained<ClockObserver>,
}

unsafe impl Send for ClockRegistration {}

impl Drop for ClockRegistration {
    fn drop(&mut self) {
        let observer: &AnyObject = &self.observer;
        unsafe { self.center.removeObserver(observer) };
    }
}
