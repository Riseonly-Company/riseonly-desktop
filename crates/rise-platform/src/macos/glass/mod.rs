//! The Rust half of the `RiseGlass` bridge: tier 1 of `docs/PLATFORM_GLASS.md`.
//!
//! `RiseGlass.swift` hosts one AppKit view per region below the transparent
//! Metal layer. Everything this file decides — which backing a region gets,
//! whether a batch is admitted at all, what a status code means — is a pure
//! function tested on this machine for all three platforms. What crosses the ABI
//! is only the result.
//!
//! One thing the version policy in [`crate::materials`] cannot answer alone: the
//! view could fail to instantiate for a reason no version number predicts.
//! [`probe_backing`] asks the Swift side what it can really host, and the answer
//! caps the prediction — it can only ever lower it.
//!
//! It is NOT there to rescue a build made against an older SDK, and the earlier
//! claim that it was has been measured and is false. `RiseGlass.swift` compiles
//! clean against the 13.3, 14 and 15 SDKs, none of which declare
//! `NSGlassEffectView`, and the 15-SDK build run on macOS 26.1 reports
//! LiquidGlass — because `NSClassFromString` plus `#available` resolve against
//! the RUNNING OS, not the one the app was built on. An old SDK is survivable
//! precisely because nothing here ever names the class as a type.

use std::ffi::c_void;
#[cfg(test)]
use std::ffi::{CStr, c_char};
use std::ptr::NonNull;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::materials::{
    GlassError, GlassLayout, GlassSurface, LayoutGate, MacOsVersion, Material, MaterialBacking,
    RegionRect, macos_version, offered_backing, resolve,
};

// ---------------------------------------------------------------------------
// The ABI. Mirrored in RiseGlass.swift; changing one without the other is a
// silent miscompile, which is why every value below is asserted in a test that
// calls the linked Swift.
// ---------------------------------------------------------------------------

const OK: i32 = 0;
const NOT_MAIN_THREAD: i32 = -1;
const NO_HANDLE: i32 = -2;
const NO_HOST_VIEW: i32 = -3;
const NO_VIEW_CLASS: i32 = -4;
const UNKNOWN_MATERIAL: i32 = -5;

const BACKING_PAINTED: i32 = 0;
const BACKING_VIBRANCY: i32 = 1;
const BACKING_LIQUID_GLASS: i32 = 2;

const MATERIAL_CHROME: i32 = 0;
const MATERIAL_PANEL: i32 = 1;
const MATERIAL_OVERLAY: i32 = 2;

unsafe extern "C" {
    fn rise_glass_probe_backing() -> i32;
    // Only the drift test calls this; see vibrancy_material_name.
    #[cfg(test)]
    fn rise_glass_vibrancy_material_name(material: i32) -> *const c_char;
    fn rise_glass_attach() -> *mut c_void;
    fn rise_glass_begin(handle: *mut c_void) -> i32;
    fn rise_glass_region(
        handle: *mut c_void,
        id: u64,
        backing: i32,
        material: i32,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        radius: f64,
    ) -> i32;
    fn rise_glass_commit(handle: *mut c_void) -> i32;
    fn rise_glass_clear(handle: *mut c_void) -> i32;
    fn rise_glass_detach(handle: *mut c_void) -> i32;
}

pub const fn backing_code(backing: MaterialBacking) -> i32 {
    match backing {
        MaterialBacking::Painted => BACKING_PAINTED,
        MaterialBacking::Vibrancy => BACKING_VIBRANCY,
        MaterialBacking::LiquidGlass => BACKING_LIQUID_GLASS,
    }
}

/// An unrecognised code is [`MaterialBacking::Painted`], for the same reason an
/// unrecognised OS version is: falling *down* costs a flat panel, and guessing
/// upward costs a blank rail.
pub const fn backing_from_code(code: i32) -> MaterialBacking {
    match code {
        BACKING_LIQUID_GLASS => MaterialBacking::LiquidGlass,
        BACKING_VIBRANCY => MaterialBacking::Vibrancy,
        _ => MaterialBacking::Painted,
    }
}

pub const fn material_code(material: Material) -> i32 {
    match material {
        Material::Chrome => MATERIAL_CHROME,
        Material::Panel => MATERIAL_PANEL,
        Material::Overlay => MATERIAL_OVERLAY,
    }
}

const fn refusal(code: i32) -> &'static str {
    match code {
        NOT_MAIN_THREAD => "AppKit was reached from a thread that is not the main one",
        NO_HANDLE => "the glass surface was used after it was detached",
        NO_HOST_VIEW => "the window has no view to host a glass region below",
        NO_VIEW_CLASS => "this machine has no NSGlassEffectView to instantiate",
        UNKNOWN_MATERIAL => "the material has no NSVisualEffectMaterial case",
        _ => "the platform layer refused the region",
    }
}

fn check(code: i32) -> Result<(), GlassError> {
    if code == OK {
        Ok(())
    } else {
        Err(GlassError::Refused(format!("{} ({code})", refusal(code))))
    }
}

// ---------------------------------------------------------------------------
// Policy. Pure, and exercised for every host from this machine.
// ---------------------------------------------------------------------------

/// The best backing the whole surface can host: the version policy, lowered by
/// what the AppKit side reports it can actually instantiate.
///
/// It can only ever lower. A probe that claimed glass on a machine whose version
/// says otherwise would be believed by nothing.
pub const fn surface_backing(
    host: HostOs,
    version: Option<MacOsVersion>,
    probed: MaterialBacking,
) -> MaterialBacking {
    offered_backing(host, version).capped_at(probed)
}

/// The same, for one surface's [`Material::ceiling`].
pub const fn region_backing(
    material: Material,
    host: HostOs,
    version: Option<MacOsVersion>,
    probed: MaterialBacking,
) -> MaterialBacking {
    resolve(material, host, version).capped_at(probed)
}

/// A rectangle AppKit can be given a frame from.
///
/// Every one of the four numbers is checked for finiteness, and none of them is
/// covered by [`RegionRect::has_area`]. That comparison rejects a NaN extent
/// because NaN fails every comparison — but `f32::INFINITY > 0.0` is true, so an
/// infinite width sails through it and reaches AppKit as a view frame of
/// undefined size. A NaN origin is the same class of hazard from the other side:
/// the view has an undefined position rather than an invisible one, and AppKit
/// does not complain about either.
pub fn is_hostable(bounds: &RegionRect) -> bool {
    bounds.has_area()
        && bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
}

// ---------------------------------------------------------------------------
// Bindings. No decisions live below this line.
// ---------------------------------------------------------------------------

/// What the linked `RiseGlass` can instantiate on this machine, right now.
///
/// Safe from any thread: it is a class lookup behind an availability guard and
/// touches no AppKit state.
pub fn probe_backing() -> MaterialBacking {
    backing_from_code(unsafe { rise_glass_probe_backing() })
}

/// The `NSVisualEffectMaterial` case `RiseGlass.swift` will configure a vibrancy
/// region with, so the two halves of the bridge can be asserted equal instead of
/// assumed equal.
///
/// Test-only, and that is the whole point: production never asks the Swift what
/// it is going to do — the Rust names the case in `Material::vibrancy_material`
/// and the Swift names it again on its own side. This is the probe that pins the
/// two together, and nothing but the test has any business calling it.
///
/// Safe from any thread. The returned string is leaked once by the Swift side
/// and lives for the process.
#[cfg(test)]
pub fn vibrancy_material_name(material: Material) -> Option<&'static str> {
    let raw = unsafe { rise_glass_vibrancy_material_name(material_code(material)) };
    if raw.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(raw) }.to_str().ok()
}

/// Real `NSGlassEffectView` and `NSVisualEffectView` regions below the Metal
/// layer.
///
/// Neither `Send` nor `Sync`, and not by accident: it owns AppKit views, and
/// AppKit off the main thread is undefined behaviour. The raw handle makes that
/// a property of the type rather than a rule in a comment.
pub struct MacGlassSurface {
    host: HostOs,
    version: Option<MacOsVersion>,
    probed: MaterialBacking,
    backing: MaterialBacking,
    gate: LayoutGate,
    handle: Option<NonNull<c_void>>,
}

impl MacGlassSurface {
    pub fn new() -> Self {
        Self::with_policy(HostOs::current(), macos_version(), probe_backing())
    }

    fn with_policy(host: HostOs, version: Option<MacOsVersion>, probed: MaterialBacking) -> Self {
        Self {
            host,
            version,
            probed,
            backing: surface_backing(host, version, probed),
            gate: LayoutGate::new(),
            handle: None,
        }
    }

    /// Attached on first use rather than at construction: the shell builds its
    /// surface while it is deciding what to lay out, which is before the window
    /// exists.
    fn handle(&mut self) -> Option<NonNull<c_void>> {
        if self.handle.is_none() {
            self.handle = NonNull::new(unsafe { rise_glass_attach() });
        }
        self.handle
    }
}

impl Default for MacGlassSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl GlassSurface for MacGlassSurface {
    fn backing(&self) -> MaterialBacking {
        self.backing
    }

    fn commit(&mut self, layout: &GlassLayout) -> Result<PlatformSupport, GlassError> {
        // Before the host check, exactly as `InMemoryGlassSurface` does it: a
        // batch that did not come from a newer layout pass is a bug on every
        // platform, and gating it behind a native backing would mean it could
        // only ever be caught on the one platform that has glass.
        self.gate.admit(layout.generation())?;

        if self.backing.is_painted() {
            return Ok(PlatformSupport::Unsupported);
        }

        // Whole batch, before the pass is opened, so that a refusal leaves the
        // previous frame's views exactly where they were rather than half of
        // this frame's.
        for region in layout.regions() {
            if !is_hostable(&region.bounds) {
                return Err(GlassError::Refused(format!(
                    "region {:?} has a frame AppKit cannot be given: {:?}",
                    region.id, region.bounds
                )));
            }
        }

        let Some(handle) = self.handle() else {
            return Err(GlassError::Refused(
                "no window is open to host a glass region in".to_owned(),
            ));
        };

        check(unsafe { rise_glass_begin(handle.as_ptr()) })?;

        for region in layout.regions() {
            let backing = region_backing(region.material, self.host, self.version, self.probed);
            if backing.is_painted() {
                continue;
            }

            check(unsafe {
                rise_glass_region(
                    handle.as_ptr(),
                    region.id.0,
                    backing_code(backing),
                    material_code(region.material),
                    f64::from(region.bounds.x),
                    f64::from(region.bounds.y),
                    f64::from(region.bounds.width),
                    f64::from(region.bounds.height),
                    f64::from(region.corner_radius),
                )
            })?;
        }

        check(unsafe { rise_glass_commit(handle.as_ptr()) })?;
        Ok(PlatformSupport::Performed)
    }

    fn clear(&mut self) -> PlatformSupport {
        if self.backing.is_painted() {
            return PlatformSupport::Unsupported;
        }

        // Never attached means nothing was ever hosted, so there is nothing left
        // behind under the Metal layer and the request is already satisfied.
        let Some(handle) = self.handle else {
            return PlatformSupport::Performed;
        };

        if unsafe { rise_glass_clear(handle.as_ptr()) } == OK {
            PlatformSupport::Performed
        } else {
            PlatformSupport::Unsupported
        }
    }
}

/// The container view outlives this value unless it is torn down explicitly:
/// it belongs to the window's view tree, not to the handle.
impl Drop for MacGlassSurface {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe { rise_glass_detach(handle.as_ptr()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::{GlassRegion, RegionId};
    use rise_core::GenerationCounter;

    fn on_main_thread() -> bool {
        objc2::MainThreadMarker::new().is_some()
    }

    fn rail(height: f32) -> RegionRect {
        RegionRect::new(0.0, 0.0, 56.0, height)
    }

    #[test]
    fn the_surface_reports_what_the_version_policy_predicts_capped_by_the_probe() {
        let versions = [
            None,
            Some(MacOsVersion::new(10, 16, 0)),
            Some(MacOsVersion::VIBRANCY),
            Some(MacOsVersion::LIQUID_GLASS),
            Some(MacOsVersion::new(99, 0, 0)),
        ];

        for host in HostOs::ALL {
            for version in versions {
                for probed in MaterialBacking::ALL {
                    let expected = offered_backing(host, version).capped_at(probed);
                    let surface = MacGlassSurface::with_policy(host, version, probed);

                    assert_eq!(
                        surface.backing(),
                        expected,
                        "{host:?} {version:?} with a {probed:?} probe"
                    );
                    assert!(
                        surface.backing().tier() <= offered_backing(host, version).tier(),
                        "the probe raised the tier the version policy allows"
                    );
                }
            }
        }
    }

    #[test]
    fn this_machine_reports_the_backing_resolve_predicts_for_it() {
        let surface = MacGlassSurface::new();
        let expected = surface_backing(HostOs::current(), macos_version(), probe_backing());

        assert_eq!(surface.backing(), expected);
        for material in Material::ALL {
            assert!(
                region_backing(
                    material,
                    HostOs::current(),
                    macos_version(),
                    probe_backing()
                )
                .tier()
                    <= surface.backing().tier(),
                "{material:?} may lower the surface tier, never raise it"
            );
        }
    }

    /// The version policy says LiquidGlass; the probe says the class did not
    /// come back. The probe wins, and the app degrades to vibrancy rather than
    /// asking for a view that is not there.
    #[test]
    fn the_probe_overrules_a_version_policy_that_predicted_glass() {
        let surface = MacGlassSurface::with_policy(
            HostOs::MacOs,
            Some(MacOsVersion::LIQUID_GLASS),
            MaterialBacking::Vibrancy,
        );

        assert_eq!(
            surface.backing(),
            MaterialBacking::Vibrancy,
            "a probe answering vibrancy must degrade the app, not blank the rail"
        );
    }

    #[test]
    fn the_backing_codes_round_trip_and_an_unknown_one_falls_down() {
        for backing in MaterialBacking::ALL {
            assert_eq!(backing_from_code(backing_code(backing)), backing);
        }

        for code in [-1, 3, 99, i32::MIN, i32::MAX] {
            assert_eq!(backing_from_code(code), MaterialBacking::Painted);
        }
    }

    #[test]
    fn every_material_has_its_own_code() {
        for material in Material::ALL {
            let clashes = Material::ALL
                .iter()
                .filter(|other| material_code(**other) == material_code(material))
                .count();
            assert_eq!(
                clashes, 1,
                "{material:?} shares a code with another surface"
            );
        }
    }

    #[test]
    fn every_refusal_names_something_other_than_the_number() {
        for code in [
            NOT_MAIN_THREAD,
            NO_HANDLE,
            NO_HOST_VIEW,
            NO_VIEW_CLASS,
            UNKNOWN_MATERIAL,
            -99,
        ] {
            let error = check(code).unwrap_err();
            let GlassError::Refused(message) = &error else {
                panic!("{code} became {error:?} rather than a refusal");
            };
            assert!(message.contains(&code.to_string()));
            assert!(message.len() > code.to_string().len() + 4);
        }

        assert!(check(OK).is_ok());
    }

    #[test]
    fn a_frame_appkit_cannot_use_is_not_hostable() {
        assert!(is_hostable(&rail(800.0)));

        for bounds in [
            RegionRect::new(f32::NAN, 0.0, 56.0, 800.0),
            RegionRect::new(0.0, f32::NAN, 56.0, 800.0),
            RegionRect::new(f32::INFINITY, 0.0, 56.0, 800.0),
            RegionRect::new(0.0, f32::NEG_INFINITY, 56.0, 800.0),
            RegionRect::new(0.0, 0.0, 0.0, 800.0),
            RegionRect::new(0.0, 0.0, 56.0, f32::NAN),
            RegionRect::new(0.0, 0.0, f32::NAN, 800.0),
            // has_area() passes these: infinity IS greater than zero. Only the
            // finiteness check stops them, which is why it is not redundant.
            RegionRect::new(0.0, 0.0, f32::INFINITY, 800.0),
            RegionRect::new(0.0, 0.0, 56.0, f32::INFINITY),
        ] {
            assert!(!is_hostable(&bounds), "{bounds:?} is an undefined frame");
        }
    }

    #[test]
    fn an_undefined_origin_is_refused_before_the_pass_is_opened() {
        let counter = GenerationCounter::new();
        let mut surface = MacGlassSurface::with_policy(
            HostOs::MacOs,
            Some(MacOsVersion::LIQUID_GLASS),
            probe_backing(),
        );
        assert!(!surface.backing().is_painted());

        // GlassLayout::push guards the size, not the origin, so this is the one
        // undefined frame that can reach the platform layer.
        let mut layout = GlassLayout::new(counter.bump());
        layout
            .push(GlassRegion::new(
                RegionId(1),
                Material::Chrome,
                RegionRect::new(f32::NAN, 0.0, 56.0, 800.0),
            ))
            .unwrap();

        let error = surface.commit(&layout).unwrap_err();
        let GlassError::Refused(message) = &error else {
            panic!("an undefined frame became {error:?} rather than a refusal");
        };
        assert!(message.contains("NaN"), "{message}");
    }

    #[test]
    fn a_stale_or_repeated_layout_pass_is_refused_before_anything_is_hosted() {
        let counter = GenerationCounter::new();
        let first = counter.bump();
        let second = counter.bump();

        // Painted policy, so the refusal is provably the gate and not a missing
        // window: the contract has to be catchable on the developer's machine.
        let mut surface =
            MacGlassSurface::with_policy(HostOs::MacOs, None, MaterialBacking::LiquidGlass);
        assert_eq!(surface.backing(), MaterialBacking::Painted);

        let newer = GlassLayout::new(second);
        assert_eq!(
            surface.commit(&newer).unwrap(),
            PlatformSupport::Unsupported
        );

        assert_eq!(
            surface.commit(&newer),
            Err(GlassError::StaleLayout {
                accepted: second.get(),
                offered: second.get(),
            }),
            "a render pass has no new generation and must not move a region"
        );

        let older = GlassLayout::new(first);
        assert!(surface.commit(&older).is_err());
    }

    #[test]
    fn a_painted_mac_hosts_nothing_and_says_so() {
        let counter = GenerationCounter::new();
        let mut surface = MacGlassSurface::with_policy(
            HostOs::MacOs,
            Some(MacOsVersion::new(12, 0, 0)),
            MaterialBacking::LiquidGlass,
        );

        let mut layout = GlassLayout::new(counter.bump());
        layout
            .push(GlassRegion::new(RegionId(1), Material::Chrome, rail(800.0)))
            .unwrap();

        assert_eq!(
            surface.commit(&layout).unwrap(),
            PlatformSupport::Unsupported,
            "the caller has to be able to tell it must paint the material itself"
        );
        assert_eq!(surface.clear(), PlatformSupport::Unsupported);
    }

    #[test]
    fn a_zero_area_or_repeated_region_never_reaches_the_platform() {
        let counter = GenerationCounter::new();
        let mut layout = GlassLayout::new(counter.bump());

        assert_eq!(
            layout.push(GlassRegion::new(
                RegionId(1),
                Material::Chrome,
                RegionRect::new(0.0, 0.0, 56.0, 0.0),
            )),
            Err(GlassError::EmptyRegion(RegionId(1)))
        );

        layout
            .push(GlassRegion::new(RegionId(1), Material::Chrome, rail(800.0)))
            .unwrap();
        assert_eq!(
            layout.push(GlassRegion::new(
                RegionId(1),
                Material::Panel,
                RegionRect::new(56.0, 0.0, 320.0, 800.0),
            )),
            Err(GlassError::DuplicateRegion(RegionId(1)))
        );

        assert_eq!(
            layout.regions().len(),
            1,
            "only the one well-formed region may ever be handed to AppKit"
        );
    }

    // -----------------------------------------------------------------------
    // Below here the linked Swift actually runs.
    // -----------------------------------------------------------------------

    #[test]
    fn the_linked_swift_configures_the_vibrancy_material_the_policy_names() {
        for material in Material::ALL {
            assert_eq!(
                vibrancy_material_name(material),
                Some(material.vibrancy_material()),
                "RiseGlass.swift and Material::vibrancy_material drifted apart"
            );
        }
    }

    #[test]
    fn the_linked_swift_never_claims_glass_on_a_release_that_has_none() {
        let probed = probe_backing();

        assert!(
            !probed.is_painted(),
            "NSVisualEffectView exists on every macOS this product runs on"
        );

        if let Some(version) = macos_version()
            && !version.is_at_least(MacOsVersion::LIQUID_GLASS)
        {
            assert_eq!(
                probed,
                MaterialBacking::Vibrancy,
                "{version:?} predates NSGlassEffectView"
            );
        }
    }

    #[test]
    fn a_native_surface_with_no_window_refuses_instead_of_pretending() {
        if on_main_thread() {
            // libtest usually hands a test a spawned thread, which is the arm
            // worth asserting: the ABI must refuse before it touches AppKit.
            // Run under --test-threads=1 the harness is the main thread, and
            // reaching NSApplication from a process that never called
            // NSApplicationMain is not something a test should provoke.
            eprintln!("skipped: this test ran on the main thread");
            return;
        }

        let counter = GenerationCounter::new();
        let mut surface = MacGlassSurface::with_policy(
            HostOs::MacOs,
            Some(MacOsVersion::LIQUID_GLASS),
            probe_backing(),
        );
        assert!(!surface.backing().is_painted());

        let mut layout = GlassLayout::new(counter.bump());
        layout
            .push(
                GlassRegion::new(RegionId(1), Material::Chrome, rail(800.0))
                    .with_corner_radius(12.0),
            )
            .unwrap();

        let error = surface.commit(&layout).unwrap_err();
        assert!(
            matches!(error, GlassError::Refused(_)),
            "an unattachable surface must report a refusal, not success: {error:?}"
        );

        assert_eq!(
            surface.clear(),
            PlatformSupport::Performed,
            "nothing was ever hosted, so nothing is left under the Metal layer"
        );
    }
}
