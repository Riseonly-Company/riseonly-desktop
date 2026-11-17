use std::sync::mpsc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{define_class, msg_send, sel, AnyThread as _, DefinedClass};
use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
use objc2_foundation::{NSNotification, NSNotificationCenter};

use super::accessibility::*;

pub(super) struct NativeAccessibility;

impl AccessibilitySettings for NativeAccessibility {
    fn reduce_transparency(&self) -> Result<bool, AccessibilityUnavailable> {
        Ok(read_reduce_transparency())
    }

    fn watch_reduce_transparency(
        &self,
        sink: ReduceTransparencySink,
    ) -> Result<AccessibilityWatch, AccessibilityUnavailable> {
        watch_display_option(DisplayOption::ReduceTransparency, sink)
    }

    fn reduce_motion(&self) -> Result<bool, AccessibilityUnavailable> {
        Ok(read_reduce_motion())
    }

    fn watch_reduce_motion(
        &self,
        sink: ReduceMotionSink,
    ) -> Result<AccessibilityWatch, AccessibilityUnavailable> {
        watch_display_option(DisplayOption::ReduceMotion, sink)
    }
}

/// display option one observer reads anew on each appkit notification
#[derive(Clone, Copy, Debug)]
enum DisplayOption {
    ReduceTransparency,
    ReduceMotion,
}

impl DisplayOption {
    fn read(self) -> bool {
        match self {
            Self::ReduceTransparency => read_reduce_transparency(),
            Self::ReduceMotion => read_reduce_motion(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ReduceTransparency => "reduce transparency",
            Self::ReduceMotion => "reduce motion",
        }
    }
}

/// one observer and one delivery thread per watched option, so dropping one watch never stops the other
fn watch_display_option(
    option: DisplayOption,
    sink: Box<dyn Fn(bool) + Send + Sync>,
) -> Result<AccessibilityWatch, AccessibilityUnavailable> {
    let (sender, values) = mpsc::channel::<bool>();
    std::thread::Builder::new()
        .name("rise-accessibility-watch".into())
        .spawn(move || deliver_all(option, values, sink))
        .map_err(|error| {
            AccessibilityUnavailable::SourceRefused(format!("delivery thread: {error}"))
        })?;
    let observer = DisplayOptionsObserver::new(option, sender);
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    unsafe {
        center.addObserver_selector_name_object(
            &observer,
            sel!(displayOptionsDidChange:),
            Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
            None,
        );
    }
    Ok(AccessibilityWatch::new(Registration { center, observer }))
}

fn read_reduce_transparency() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
}

fn read_reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

fn deliver_all(
    option: DisplayOption,
    values: mpsc::Receiver<bool>,
    sink: Box<dyn Fn(bool) + Send + Sync>,
) {
    for reduce in values {
        let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(reduce)));
        if delivered.is_err() {
            tracing::error!(target: "riseonly", option = option.label(), reduce, "accessibility handler panicked");
        }
    }
}

struct ObserverIvars {
    option: DisplayOption,
    values: mpsc::Sender<bool>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseAccessibilityDisplayObserver"]
    #[ivars = ObserverIvars]
    struct DisplayOptionsObserver;

    impl DisplayOptionsObserver {
        #[unsafe(method(displayOptionsDidChange:))]
        fn display_options_did_change(&self, _notification: &NSNotification) {
            let option = self.ivars().option;
            let reduce = option.read();
            if self.ivars().values.send(reduce).is_err() {
                tracing::warn!(target: "riseonly", option = option.label(), reduce, "accessibility delivery thread gone");
            }
        }
    }
);

impl DisplayOptionsObserver {
    fn new(option: DisplayOption, values: mpsc::Sender<bool>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ObserverIvars { option, values });
        unsafe { msg_send![super(this), init] }
    }
}

/// unregisters the observer when the watch drops
/// releasing it drops the sender which ends the delivery thread
struct Registration {
    center: Retained<NSNotificationCenter>,
    observer: Retained<DisplayOptionsObserver>,
}

unsafe impl Send for Registration {}

impl Drop for Registration {
    fn drop(&mut self) {
        let observer: &AnyObject = &self.observer;
        unsafe { self.center.removeObserver(observer) };
    }
}
