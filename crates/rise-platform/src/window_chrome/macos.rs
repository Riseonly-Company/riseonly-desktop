use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_foundation::NSString;
#[cfg(feature = "dev-drive")]
use objc2::runtime::NSObject;
#[cfg(feature = "dev-drive")]
use objc2::{AnyThread as _, DefinedClass, define_class};
#[cfg(feature = "dev-drive")]
use objc2_foundation::{NSNotification, NSNotificationCenter};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::window_chrome::*;
use crate::gpui_shim::PlatformSupport;

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    static kCACornerCurveCircular: &'static NSString;
    static kCACornerCurveContinuous: &'static NSString;
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSAppearanceNameAqua: &'static NSString;
    static NSAppearanceNameDarkAqua: &'static NSString;
    static NSAppearanceNameVibrantLight: &'static NSString;
    static NSAppearanceNameVibrantDark: &'static NSString;
}

const STANDARD_WINDOW_BUTTONS: [usize; 3] = [0, 1, 2];

pub(super) fn round_window_corner(window: &gpui::Window) -> PlatformSupport {
    let Some(corner) = current_window_corner() else {
        return PlatformSupport::Unsupported;
    };

    if let Some(radius) = tuned_corner_radius(std::env::var(CORNER_RADIUS_VAR).ok().as_deref()) {
        tracing::info!(
            target: "riseonly",
            "{CORNER_RADIUS_VAR} overrides the window corner: {radius}pt"
        );
    }

    apply_corner(window, corner)
}

pub(super) fn place_traffic_lights(window: &gpui::Window, chrome: &WindowChrome) -> PlatformSupport {
    match chrome.traffic_lights {
        Some(origin) => {
            window.set_traffic_light_position(traffic_light_point(origin));
            PlatformSupport::Performed
        }
        None => PlatformSupport::Unsupported,
    }
}

pub(super) fn set_system_controls_appearance(
    window: &gpui::Window,
    appearance: Option<gpui::WindowAppearance>,
) -> PlatformSupport {
    if MainThreadMarker::new().is_none() {
        return PlatformSupport::Unsupported;
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return PlatformSupport::Unsupported;
    };

    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return PlatformSupport::Unsupported;
    };

    let Some(appearance_class) = AnyClass::get(c"NSAppearance") else {
        return PlatformSupport::Unsupported;
    };

    unsafe {
        let view = appkit.ns_view.cast::<AnyObject>().as_ref();

        let native: Option<Retained<AnyObject>> = msg_send![view, window];
        let Some(native) = native else {
            return PlatformSupport::Unsupported;
        };

        let named: Option<Retained<AnyObject>> = match appearance {
            None => None,
            Some(appearance) => {
                let name: &NSString = match appearance {
                    gpui::WindowAppearance::Light => NSAppearanceNameAqua,
                    gpui::WindowAppearance::Dark => NSAppearanceNameDarkAqua,
                    gpui::WindowAppearance::VibrantLight => NSAppearanceNameVibrantLight,
                    gpui::WindowAppearance::VibrantDark => NSAppearanceNameVibrantDark,
                };
                let resolved: Option<Retained<AnyObject>> =
                    msg_send![appearance_class, appearanceNamed: name];
                if resolved.is_none() {
                    return PlatformSupport::Unsupported;
                }
                resolved
            }
        };

        let mut styled = 0;
        for kind in STANDARD_WINDOW_BUTTONS {
            let button: Option<Retained<AnyObject>> =
                msg_send![&*native, standardWindowButton: kind];
            let Some(button) = button else { continue };

            let _: () = msg_send![&*button, setAppearance: named.as_deref()];
            let _: () = msg_send![&*button, setNeedsDisplay: Bool::YES];
            styled += 1;
        }

        if styled == 0 {
            return PlatformSupport::Unsupported;
        }
    }

    PlatformSupport::Performed
}

fn apply_corner(window: &gpui::Window, corner: WindowCorner) -> PlatformSupport {
    if MainThreadMarker::new().is_none() {
        return PlatformSupport::Unsupported;
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return PlatformSupport::Unsupported;
    };

    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return PlatformSupport::Unsupported;
    };

    unsafe {
        let view = appkit.ns_view.cast::<AnyObject>().as_ref();

        let native: Option<Retained<AnyObject>> = msg_send![view, window];
        let Some(native) = native else {
            return PlatformSupport::Unsupported;
        };

        if !shape_native_window(&native, corner) || !mask_view(view, corner) {
            return PlatformSupport::Unsupported;
        }

        let _: () = msg_send![&*native, displayIfNeeded];
        let _: () = msg_send![&*native, setHasShadow: Bool::NO];
        let _: () = msg_send![&*native, setHasShadow: Bool::YES];
        let _: () = msg_send![&*native, invalidateShadow];
    }

    PlatformSupport::Performed
}

unsafe fn shape_native_window(window: &AnyObject, corner: WindowCorner) -> bool {
    unsafe {
        let setter: Bool = msg_send![window, respondsToSelector: sel!(_setCornerRadius:)];
        let getter: Bool = msg_send![window, respondsToSelector: sel!(_cornerRadius)];
        if !setter.as_bool() || !getter.as_bool() {
            return false;
        }

        let requested = corner.radius as f64;
        let _: () = msg_send![window, _setCornerRadius: requested];
        let granted: f64 = msg_send![window, _cornerRadius];

        (granted - requested).abs() <= f64::EPSILON
    }
}

unsafe fn mask_view(view: &AnyObject, corner: WindowCorner) -> bool {
    unsafe {
        let clips: Bool = msg_send![view, respondsToSelector: sel!(setClipsToBounds:)];
        if clips.as_bool() {
            let _: () = msg_send![view, setClipsToBounds: Bool::YES];
        }

        let mut layer: Option<Retained<AnyObject>> = msg_send![view, layer];
        if layer.is_none() {
            let wants_layer: Bool = msg_send![view, respondsToSelector: sel!(setWantsLayer:)];
            if wants_layer.as_bool() {
                let _: () = msg_send![view, setWantsLayer: Bool::YES];
                layer = msg_send![view, layer];
            }
        }
        let Some(layer) = layer else { return false };

        let Some(transaction) = AnyClass::get(c"CATransaction") else {
            return false;
        };

        let curve: &NSString = if corner.continuous {
            kCACornerCurveContinuous
        } else {
            kCACornerCurveCircular
        };

        let radius = corner.radius as f64;

        let _: () = msg_send![transaction, begin];
        let _: () = msg_send![transaction, setDisableActions: Bool::YES];
        let _: () = msg_send![&*layer, setCornerCurve: curve];
        let _: () = msg_send![&*layer, setCornerRadius: radius];
        let _: () = msg_send![&*layer, setMasksToBounds: Bool::YES];
        let _: () = msg_send![transaction, commit];
        true
    }
}

#[cfg(feature = "dev-drive")]
const CONCEALED_WINDOW_LEVEL: isize = -1;
#[cfg(feature = "dev-drive")]
const CONCEALED_COLLECTION_BEHAVIOR: usize = (1 << 3) | (1 << 6);
#[cfg(feature = "dev-drive")]
const ACCESSORY_ACTIVATION_POLICY: isize = 1;

#[cfg(feature = "dev-drive")]
pub(super) fn conceal_window_for_dev_drive(window: &gpui::Window) -> PlatformSupport {
    if MainThreadMarker::new().is_none() {
        return PlatformSupport::Unsupported;
    }

    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return PlatformSupport::Unsupported;
    };

    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return PlatformSupport::Unsupported;
    };

    unsafe {
        let view = appkit.ns_view.cast::<AnyObject>().as_ref();

        let native: Option<Retained<AnyObject>> = msg_send![view, window];
        let Some(native) = native else {
            return PlatformSupport::Unsupported;
        };

        let alpha: f64 = msg_send![&*native, alphaValue];
        if alpha != 0.0 {
            let _: () = msg_send![&*native, setAlphaValue: 0.0_f64];
        }

        let ignores: Bool = msg_send![&*native, ignoresMouseEvents];
        if !ignores.as_bool() {
            let _: () = msg_send![&*native, setIgnoresMouseEvents: Bool::YES];
        }

        let level: isize = msg_send![&*native, level];
        if level != CONCEALED_WINDOW_LEVEL {
            let _: () = msg_send![&*native, setLevel: CONCEALED_WINDOW_LEVEL];
        }

        let behavior: usize = msg_send![&*native, collectionBehavior];
        if behavior & CONCEALED_COLLECTION_BEHAVIOR != CONCEALED_COLLECTION_BEHAVIOR {
            let concealed = behavior | CONCEALED_COLLECTION_BEHAVIOR;
            let _: () = msg_send![&*native, setCollectionBehavior: concealed];
        }

        let in_menu: Bool = msg_send![&*native, isExcludedFromWindowsMenu];
        if !in_menu.as_bool() {
            let _: () = msg_send![&*native, setExcludedFromWindowsMenu: Bool::YES];
        }

        let visible: Bool = msg_send![&*native, isVisible];
        if visible.as_bool() {
            let _: () = msg_send![&*native, orderOut: None::<&AnyObject>];
        }
    }

    PlatformSupport::Performed
}

#[cfg(feature = "dev-drive")]
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSWindowDidChangeOcclusionStateNotification: &'static NSString;
    static NSApplicationDidBecomeActiveNotification: &'static NSString;
    static NSApplicationDidHideNotification: &'static NSString;
}

#[cfg(feature = "dev-drive")]
struct WindowEventObserverIvars {
    wake: Box<dyn Fn() + Send + Sync>,
}

#[cfg(feature = "dev-drive")]
define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseDevDriveWindowEventObserver"]
    #[ivars = WindowEventObserverIvars]
    struct WindowEventObserver;

    impl WindowEventObserver {
        #[unsafe(method(windowEventDidHappen:))]
        fn window_event_did_happen(&self, _notification: &NSNotification) {
            (self.ivars().wake)();
        }
    }
);

#[cfg(feature = "dev-drive")]
impl WindowEventObserver {
    fn new(wake: Box<dyn Fn() + Send + Sync>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(WindowEventObserverIvars { wake });
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(feature = "dev-drive")]
struct WindowEventRegistration {
    center: Retained<NSNotificationCenter>,
    observer: Retained<WindowEventObserver>,
}

#[cfg(feature = "dev-drive")]
impl Drop for WindowEventRegistration {
    fn drop(&mut self) {
        let observer: &AnyObject = &self.observer;
        unsafe { self.center.removeObserver(observer) };
    }
}

#[cfg(feature = "dev-drive")]
pub(super) fn watch_windows_for_dev_drive(
    wake: Box<dyn Fn() + Send + Sync>,
) -> Option<Box<dyn std::any::Any>> {
    MainThreadMarker::new()?;
    let observer = WindowEventObserver::new(wake);
    let center = NSNotificationCenter::defaultCenter();
    unsafe {
        center.addObserver_selector_name_object(
            &observer,
            sel!(windowEventDidHappen:),
            Some(NSWindowDidChangeOcclusionStateNotification),
            None,
        );
        center.addObserver_selector_name_object(
            &observer,
            sel!(windowEventDidHappen:),
            Some(NSApplicationDidBecomeActiveNotification),
            None,
        );
        center.addObserver_selector_name_object(
            &observer,
            sel!(windowEventDidHappen:),
            Some(NSApplicationDidHideNotification),
            None,
        );
    }
    Some(Box::new(WindowEventRegistration { center, observer }))
}

#[cfg(feature = "dev-drive")]
pub(super) fn keep_app_in_background_for_dev_drive() -> PlatformSupport {
    if MainThreadMarker::new().is_none() {
        return PlatformSupport::Unsupported;
    }

    let Some(application_class) = AnyClass::get(c"NSApplication") else {
        return PlatformSupport::Unsupported;
    };

    unsafe {
        let application: Option<Retained<AnyObject>> =
            msg_send![application_class, sharedApplication];
        let Some(application) = application else {
            return PlatformSupport::Unsupported;
        };

        let policy: isize = msg_send![&*application, activationPolicy];
        if policy != ACCESSORY_ACTIVATION_POLICY {
            let accepted: Bool =
                msg_send![&*application, setActivationPolicy: ACCESSORY_ACTIVATION_POLICY];
            if !accepted.as_bool() {
                return PlatformSupport::Unsupported;
            }
        }

        let active: Bool = msg_send![&*application, isActive];
        if active.as_bool() {
            let _: () = msg_send![&*application, deactivate];
        }

        let hidden: Bool = msg_send![&*application, isHidden];
        if hidden.as_bool() {
            let _: () = msg_send![&*application, unhideWithoutActivation];
        }
    }

    PlatformSupport::Performed
}
