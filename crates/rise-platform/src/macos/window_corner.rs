//! Putting the window's own corner back, because gpui leaves it square.
//!
//! AppKit rounds a `NSTitledWindowMask` window, but that rounding does not reach
//! the one Metal layer gpui renders the whole app into: measured side by side,
//! the window comes out with four square corners while every AppKit app next to
//! it is rounded. So the mask is ours to set.
//!
//! It has to go on TWO independent surfaces: the `NSWindow` corner mask that
//! AppKit gives WindowServer, and the `CAMetalLayer` gpui renders into. Masking
//! only Metal rounds the black app background but leaves the native window at
//! AppKit's smaller default contour. Setting `cornerRadius` on `NSThemeFrame` is
//! not equivalent: AppKit owns that layer and restores its radius during layout.
//! The window mask and Metal mask must describe the same curve.
//! Two things were measured to get to that, and both are worth keeping:
//!
//! - **An ancestor's mask does not shape a metal layer.** Its surface goes to
//!   the compositor rather than being drawn into its parent. On a borderless
//!   window, where AppKit contributes no corner of its own to confuse the
//!   reading, masking the content view left the window square at 0pt while
//!   masking the metal layer rounded it.
//! - **A frame-layer radius is not the window-server shape.** `NSWindow` keeps a
//!   separate corner mask for hit testing, the surface and its shadow. The
//!   private `_setCornerRadius:` setter rebuilds that mask; a CALayer property
//!   does not. This is an intentional private-API bridge and must be rechecked
//!   on every major macOS release.
//!
//! The consequence for tier 1: a native material region from
//! `crate::macos::glass` is an ordinary AppKit view below the Metal layer, so
//! the NSWindow mask DOES cover it. `RiseGlass.swift` still applies each
//! region's own inner radius, but it cannot square off the window corner.
//!
//! The radius is policy and lives in [`crate::window_chrome`]; this file only
//! carries it to AppKit.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_foundation::NSString;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::gpui_shim::PlatformSupport;
use crate::window_chrome::WindowCorner;

// The two `CALayerCornerCurve` values. Linked rather than spelled out as Rust
// strings: the constants are the documented API, their contents are not.
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    static kCACornerCurveCircular: &'static NSString;
    static kCACornerCurveContinuous: &'static NSString;
}

/// Masks `window` to `corner`.
///
/// The mask only becomes visible if the window is not opaque — a rounded corner
/// is a piece of the window cut away, and an opaque window has nowhere to cut it
/// out of. [`crate::materials::preferred_window_material`] is what keeps that
/// true.
///
/// Refuses rather than panics off the main thread, and refuses on a window gpui
/// will not name an NSView for; in both cases the corner stays the square one it
/// already was.
pub fn apply(window: &gpui::Window, corner: WindowCorner) -> PlatformSupport {
    if MainThreadMarker::new().is_none() {
        return PlatformSupport::Unsupported;
    }

    // Spelled through the trait: `Window` has an inherent `window_handle` of its
    // own that answers gpui's handle, not AppKit's, and it wins the plain call.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return PlatformSupport::Unsupported;
    };

    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return PlatformSupport::Unsupported;
    };

    // SAFETY: gpui hands out a pointer to the live NSView it renders this
    // window into, borrowed for no longer than the handle it came from, and the
    // main-thread check above is the one AppKit requires to touch it at all.
    unsafe {
        let view = appkit.ns_view.cast::<AnyObject>().as_ref();

        let native: Option<Retained<AnyObject>> = msg_send![view, window];
        let Some(native) = native else {
            return PlatformSupport::Unsupported;
        };

        // The NSWindow mask is the shape WindowServer uses. The Metal mask is
        // still required because gpui's compositor surface is not an ordinary
        // AppKit descendant. A theme-frame CALayer mask cannot replace either.
        if !shape_native_window(&native, corner) || !mask_view(view, corner) {
            return PlatformSupport::Unsupported;
        }

        // THE SHADOW IS THE OTHER HALF OF THE CORNER, and forgetting it looks
        // exactly like not having rounded the window at all: AppKit traces the
        // shadow from the window's alpha, so a window whose content is round but
        // whose shadow is still square wears a square grey frame around a round
        // app. `invalidateShadow` alone is not enough here — it recomputes from
        // whatever is drawn NOW, and at open gpui has not drawn a frame yet.
        // Dropping the shadow and asking for it again makes AppKit recompute it
        // against the real content on the next display pass instead.
        let _: () = msg_send![&*native, displayIfNeeded];
        let _: () = msg_send![&*native, setHasShadow: Bool::NO];
        let _: () = msg_send![&*native, setHasShadow: Bool::YES];
        let _: () = msg_send![&*native, invalidateShadow];
    }

    PlatformSupport::Performed
}

/// Changes the actual AppKit/WindowServer corner mask, not a view background.
///
/// There is no public AppKit setter for this. `_setCornerRadius:` is private but
/// is the method that rebuilds `NSWindow`'s `_cornerMask`; using only public
/// CALayer APIs leaves the outer window on AppKit's default contour.
///
/// # Safety
///
/// `window` must be a live `NSWindow` and this must be the main thread.
unsafe fn shape_native_window(window: &AnyObject, corner: WindowCorner) -> bool {
    unsafe {
        let setter: Bool = msg_send![window, respondsToSelector: sel!(_setCornerRadius:)];
        let getter: Bool = msg_send![window, respondsToSelector: sel!(_cornerRadius)];
        if !setter.as_bool() || !getter.as_bool() {
            return false;
        }

        // CGFloat is `double` on every architecture macOS still builds for.
        let requested = corner.radius as f64;
        let _: () = msg_send![window, _setCornerRadius: requested];
        let granted: f64 = msg_send![window, _cornerRadius];

        (granted - requested).abs() <= f64::EPSILON
    }
}

/// Rounds one view's layer, and reports whether it had one to round.
///
/// # Safety
///
/// `view` must be a live `NSView` and this must be the main thread.
unsafe fn mask_view(view: &AnyObject, corner: WindowCorner) -> bool {
    unsafe {
        // A view AppKit backs, rather than one hosting a layer it was handed,
        // re-syncs `masksToBounds` from `clipsToBounds` on every layout pass —
        // so setting only the layer would hold until the first resize. macOS 14
        // and later only; older releases keep whatever the layer was set to.
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

        // CGFloat is `double` on every architecture macOS still builds for.
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
