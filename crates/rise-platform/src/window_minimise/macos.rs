use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationDidHideNotification, NSApplicationDidUnhideNotification, NSView, NSWindow,
    NSWindowDidDeminiaturizeNotification, NSWindowDidMiniaturizeNotification,
};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::window_minimise::*;
use crate::host_os::HostOs;

pub(super) struct NativeWindowMinimise;

impl WindowMinimiseSource for NativeWindowMinimise {
    fn watch(
        &self,
        window: &dyn HasWindowHandle,
        sink: MinimisedSink,
    ) -> Result<MinimiseWatch, WindowMinimiseUnavailable> {
        let mtm = MainThreadMarker::new().ok_or_else(|| no_window("an AppKit window is observed from the main thread only"))?;
        let handle = window.window_handle().map_err(|error| no_window(&error.to_string()))?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return Err(no_window("the handle is not an AppKit view"));
        };
        let view: &NSView = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
        let ns_window = view.window().ok_or_else(|| no_window("the view is not in a window yet"))?;

        let observer = MinimiseObserver::new(mtm, sink, &ns_window);
        let center = NSNotificationCenter::defaultCenter();
        let window_object: &AnyObject = &ns_window;
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(stateChanged:),
                Some(NSWindowDidMiniaturizeNotification),
                Some(window_object),
            );
            center.addObserver_selector_name_object(
                &observer,
                sel!(stateChanged:),
                Some(NSWindowDidDeminiaturizeNotification),
                Some(window_object),
            );
            center.addObserver_selector_name_object(
                &observer,
                sel!(stateChanged:),
                Some(NSApplicationDidHideNotification),
                None,
            );
            center.addObserver_selector_name_object(
                &observer,
                sel!(stateChanged:),
                Some(NSApplicationDidUnhideNotification),
                None,
            );
        }
        observer.report(mtm);
        Ok(MinimiseWatch::new(MinimiseRegistration { center, observer }))
    }
}

fn no_window(detail: &str) -> WindowMinimiseUnavailable {
    WindowMinimiseUnavailable::NoNativeWindow {
        host: HostOs::MacOs,
        detail: detail.to_owned(),
    }
}

struct ObserverIvars {
    sink: MinimisedSink,
    window: Weak<NSWindow>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "RiseWindowMinimiseObserver"]
    #[ivars = ObserverIvars]
    struct MinimiseObserver;

    impl MinimiseObserver {
        #[unsafe(method(stateChanged:))]
        fn state_changed(&self, _notification: &NSNotification) {
            self.report(self.mtm());
        }
    }
);

impl MinimiseObserver {
    fn new(mtm: MainThreadMarker, sink: MinimisedSink, window: &Retained<NSWindow>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ObserverIvars {
            sink,
            window: Weak::from_retained(window),
        });
        unsafe { msg_send![super(this), init] }
    }

    /// reads the state back from appkit and not from the notification name
    /// so hide while miniaturized then unhide still answers right
    fn report(&self, mtm: MainThreadMarker) {
        let app_hidden = NSApplication::sharedApplication(mtm).isHidden();
        let miniaturized = self.ivars().window.load().is_some_and(|window| window.isMiniaturized());
        let minimised = app_hidden || miniaturized;
        let sink = &self.ivars().sink;
        let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(minimised)));
        if delivered.is_err() {
            tracing::error!(target: "riseonly", minimised, "window minimise handler panicked");
        }
    }
}

struct MinimiseRegistration {
    center: Retained<NSNotificationCenter>,
    observer: Retained<MinimiseObserver>,
}

impl Drop for MinimiseRegistration {
    fn drop(&mut self) {
        let observer: &AnyObject = &self.observer;
        unsafe { self.center.removeObserver(observer) };
    }
}
