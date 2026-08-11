//! Which materials are real on this machine.
//!
//! **No component ever asks for glass.** A component asks for a [`Material`] and
//! this seam decides what that material actually is here. Nothing here picks a
//! colour: the painted form of a material is a rise-theme token.
//!
//! [`WindowMaterial`] (tier 0, whole-window) and [`Material`] (per-region) are
//! different mechanisms and must not be conflated.

use rise_core::Generation;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// What a surface asks the platform for.
///
/// Content is deliberately not a material: a surface that is never translucent
/// has no fallback to get wrong.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Material {
    /// The left rail: the strip that carries what the phone puts in a tab bar.
    Chrome,
    /// The list column beside the rail — chats, folders, search results.
    Panel,
    /// Sheets and popovers. In-window on every platform, never a native popup.
    Overlay,
}

/// The AppKit class behind [`MaterialBacking::LiquidGlass`].
pub const GLASS_VIEW_CLASS: &str = "NSGlassEffectView";

/// The AppKit class behind [`MaterialBacking::Vibrancy`].
pub const VIBRANCY_VIEW_CLASS: &str = "NSVisualEffectView";

impl Material {
    pub const ALL: [Self; 3] = [Self::Chrome, Self::Panel, Self::Overlay];

    /// The `NSVisualEffectMaterial` case a tier-1 vibrancy region is configured
    /// with. These cases are semantic: AppKit picks blur, tint and inactive
    /// behaviour from them, so choosing one by how it looks is wrong.
    pub const fn vibrancy_material(self) -> &'static str {
        match self {
            Self::Chrome => "NSVisualEffectMaterial.headerView",
            Self::Panel => "NSVisualEffectMaterial.sidebar",
            Self::Overlay => "NSVisualEffectMaterial.popover",
        }
    }

    /// The best backing this surface will accept, whatever the machine offers.
    ///
    /// PAINTED, everywhere, since the shell became an opaque plate. A native
    /// region is an AppKit view *below* the Metal layer, and it samples what is
    /// behind the WINDOW — so with an opaque app in front of it, every native
    /// region stops being a material and becomes a hole punched through the app
    /// to the desktop. The tier-1 machinery below is intact and reachable by
    /// raising this again; nothing about it is deleted.
    pub const fn ceiling(self) -> MaterialBacking {
        match self {
            Self::Chrome | Self::Panel | Self::Overlay => MaterialBacking::Painted,
        }
    }
}

/// What a [`Material`] actually is on a given machine.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MaterialBacking {
    /// [`GLASS_VIEW_CLASS`], macOS 26 and later.
    LiquidGlass,
    /// [`VIBRANCY_VIEW_CLASS`], which every macOS this product supports has.
    Vibrancy,
    /// Theme tokens: a translucent fill, a hairline border, a soft inner highlight.
    Painted,
}

impl MaterialBacking {
    pub const ALL: [Self; 3] = [Self::LiquidGlass, Self::Vibrancy, Self::Painted];

    /// Ascending: painted 0, vibrancy 1, glass 2.
    pub const fn tier(self) -> u8 {
        match self {
            Self::Painted => 0,
            Self::Vibrancy => 1,
            Self::LiquidGlass => 2,
        }
    }

    /// The lower of two tiers. Used to apply [`Material::ceiling`] to what the
    /// machine offered, and it can only ever lower.
    pub const fn capped_at(self, ceiling: Self) -> Self {
        if self.tier() <= ceiling.tier() {
            self
        } else {
            ceiling
        }
    }

    /// Whether this backing is an AppKit view rather than something GPUI draws.
    ///
    /// A native backing sits *below* the Metal layer: GPUI must leave that
    /// rectangle transparent, and anything it draws there lands on top.
    pub const fn is_native(self) -> bool {
        self.view_class().is_some()
    }

    pub const fn is_painted(self) -> bool {
        matches!(self, Self::Painted)
    }

    /// The class the platform layer must instantiate, or `None` when there is
    /// no view to instantiate at all.
    pub const fn view_class(self) -> Option<&'static str> {
        match self {
            Self::LiquidGlass => Some(GLASS_VIEW_CLASS),
            Self::Vibrancy => Some(VIBRANCY_VIEW_CLASS),
            Self::Painted => None,
        }
    }
}

/// A macOS release, as `NSProcessInfo` reports it.
///
/// The derived ordering runs over the fields in declaration order, so that order
/// is load-bearing; [`MacOsVersion::is_at_least`] is the `const fn` equivalent.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MacOsVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl MacOsVersion {
    /// The first release with [`GLASS_VIEW_CLASS`].
    ///
    /// Apple renumbered to the year scheme here, so 16 through 25 never shipped;
    /// the boundary is "at least 26" so a version inside that gap gets vibrancy.
    pub const LIQUID_GLASS: Self = Self::new(26, 0, 0);

    /// The oldest release this product runs on, matching `LSMinimumSystemVersion`.
    /// The floor is the product's, not `NSVisualEffectView`'s.
    pub const VIBRANCY: Self = Self::new(13, 0, 0);

    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub const fn is_at_least(self, other: Self) -> bool {
        if self.major != other.major {
            return self.major > other.major;
        }
        if self.minor != other.minor {
            return self.minor > other.minor;
        }
        self.patch >= other.patch
    }

    /// The best tier this release can host. Anything below
    /// [`MacOsVersion::VIBRANCY`] falls *down* to paint rather than guessing up.
    pub const fn backing(self) -> MaterialBacking {
        if self.is_at_least(Self::LIQUID_GLASS) {
            MaterialBacking::LiquidGlass
        } else if self.is_at_least(Self::VIBRANCY) {
            MaterialBacking::Vibrancy
        } else {
            MaterialBacking::Painted
        }
    }
}

/// The best tier this machine can host, before any per-material ceiling.
///
/// `version` is `None` off macOS and when it could not be read; both answer paint.
pub const fn offered_backing(host: HostOs, version: Option<MacOsVersion>) -> MaterialBacking {
    match host {
        HostOs::MacOs => match version {
            Some(version) => version.backing(),
            None => MaterialBacking::Painted,
        },
        // No equivalent exists: neither can put a system material behind a region.
        HostOs::Windows | HostOs::Linux => MaterialBacking::Painted,
    }
}

/// What `material` actually is on this host and this OS version.
///
/// [`Material::ceiling`] can only lower the tier the machine offers, never raise it.
pub const fn resolve(
    material: Material,
    host: HostOs,
    version: Option<MacOsVersion>,
) -> MaterialBacking {
    offered_backing(host, version).capped_at(material.ceiling())
}

/// The whole-window material: tier 0, a different mechanism from [`Material`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WindowMaterial {
    Opaque,
    /// Plain alpha: nothing at all behind the Metal layer.
    ///
    /// What the shell asks for on macOS, and not for a material — for the
    /// CORNER. AppKit rounds a titled window by leaving its corner pixels
    /// clear, and a window that has declared itself opaque has no clear pixels
    /// to leave. See [`preferred_window_material`].
    Transparent,
    /// A transparent Metal layer over a real system material.
    ///
    /// gpui turns subpixel text rendering off for any non-opaque background —
    /// free on macOS, real text quality on Windows and Linux.
    Blurred,
}

impl WindowMaterial {
    pub const ALL: [Self; 3] = [Self::Opaque, Self::Transparent, Self::Blurred];

    pub const fn into_gpui(self) -> gpui::WindowBackgroundAppearance {
        match self {
            Self::Opaque => gpui::WindowBackgroundAppearance::Opaque,
            Self::Transparent => gpui::WindowBackgroundAppearance::Transparent,
            Self::Blurred => gpui::WindowBackgroundAppearance::Blurred,
        }
    }
}

/// Whether asking gpui for a blurred window background produces one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WindowVibrancy {
    /// `gpui_macos` inserts an `NSVisualEffectView` below the Metal layer.
    Native,
    /// Only the running session can answer, and the failure is not a missing
    /// blur: without the compositor's blur protocol the window is see-through.
    SessionDependent,
    /// Only `SetWindowCompositionAttribute`, which Microsoft has never documented.
    Undocumented,
}

pub const fn window_vibrancy(host: HostOs) -> WindowVibrancy {
    match host {
        HostOs::MacOs => WindowVibrancy::Native,
        HostOs::Windows => WindowVibrancy::Undocumented,
        HostOs::Linux => WindowVibrancy::SessionDependent,
    }
}

/// What the window is actually set to, given what was asked for.
///
/// Blur is granted only where it is [`WindowVibrancy::Native`];
/// [`apply_window_material`] reports the substitution rather than hiding it.
///
/// Plain transparency is a weaker ask than blur — no system material, only an
/// alpha channel — and every target this ships to composites one, so it is
/// granted as asked.
pub const fn granted_window_material(host: HostOs, requested: WindowMaterial) -> WindowMaterial {
    match requested {
        WindowMaterial::Opaque => WindowMaterial::Opaque,
        WindowMaterial::Transparent => WindowMaterial::Transparent,
        WindowMaterial::Blurred => match window_vibrancy(host) {
            WindowVibrancy::Native => WindowMaterial::Blurred,
            WindowVibrancy::SessionDependent | WindowVibrancy::Undocumented => {
                WindowMaterial::Opaque
            }
        },
    }
}

/// The shell's own choice, and the two answers are asking for different things.
///
/// **macOS: TRANSPARENT, and not for a material — for the CORNER.** A rounded
/// corner is a piece of the window CUT AWAY, and there is nowhere to cut it out
/// of a window that has declared itself opaque; Apple's own rule for `isOpaque`
/// is that a window with rounded corners sets it `NO`. gpui maps
/// [`WindowMaterial::Opaque`] straight onto `-[NSWindow setOpaque:YES]`, so
/// asking for it squares off the whole app no matter who does the cutting.
///
/// Who does the cutting is [`crate::window_chrome::round_window_corner`], not
/// AppKit: AppKit's own rounding never reaches the Metal layer gpui renders the
/// app into. This half is only what makes that mask visible.
///
/// The app still fills its window edge to edge and paints every pixel of it, so
/// the alpha channel this buys reaches nothing but those four corners. Nothing
/// shows through anywhere else, which is why this is `Transparent` and not
/// [`WindowMaterial::Blurred`].
///
/// **Windows and Linux: OPAQUE.** Neither cuts the window out of the surface's
/// alpha — DWM rounds server-side, and under
/// [`crate::window_chrome::DecorationMode::ClientSide`] the Linux corner is
/// ours to paint — so on neither is there anything to buy with it.
///
/// The usual price of a non-opaque window is subpixel text antialiasing, which
/// gpui turns off for one. On macOS that price is zero: the Metal backend
/// reports no subpixel support at all, so mac text is greyscale-antialiased
/// either way. That is the same asymmetry [`WindowMaterial::Blurred`] notes.
pub const fn preferred_window_material(host: HostOs) -> WindowMaterial {
    let requested = match host {
        HostOs::MacOs => WindowMaterial::Transparent,
        HostOs::Windows | HostOs::Linux => WindowMaterial::Opaque,
    };

    granted_window_material(host, requested)
}

/// A region of the window that a native material is hosted behind.
///
/// One surface must keep one number for as long as it exists: the identity is
/// what lets the platform move an existing view instead of rebuilding it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RegionId(pub u64);

/// A rectangle in the window's logical pixels, top-left origin.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct RegionRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl RegionRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_gpui(bounds: gpui::Bounds<gpui::Pixels>) -> Self {
        Self {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        }
    }

    /// False for NaN as well as for zero — a NaN frame is undefined to AppKit.
    pub fn has_area(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}

/// One native material region, as the platform layer receives it.
///
/// A region is background and never a mid-stack layer: the view sits below the
/// Metal layer, so anything GPUI draws in its rectangle lands on top of it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GlassRegion {
    pub id: RegionId,
    pub material: Material,
    pub bounds: RegionRect,
    /// Rounding the *platform* applies, in logical pixels. GPUI cannot clip a
    /// native view, so this must match the painted form's radius.
    pub corner_radius: f32,
}

impl GlassRegion {
    pub const fn new(id: RegionId, material: Material, bounds: RegionRect) -> Self {
        Self {
            id,
            material,
            bounds,
            corner_radius: 0.0,
        }
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GlassError {
    #[error("layout pass {offered} is not newer than the committed pass {accepted}")]
    StaleLayout { accepted: u64, offered: u64 },
    #[error("glass region {0:?} has no area")]
    EmptyRegion(RegionId),
    #[error("glass region {0:?} was registered twice in one layout pass")]
    DuplicateRegion(RegionId),
    #[error("the platform refused the glass region: {0}")]
    Refused(String),
}

/// Admits a batch of regions only when it comes from a newer layout pass.
///
/// A region's rectangle must be static for the duration of a frame and updated
/// from the **layout** pass; a render pass has no new [`Generation`] to offer.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct LayoutGate {
    admitted: Option<Generation>,
}

impl LayoutGate {
    pub const fn new() -> Self {
        Self { admitted: None }
    }

    pub fn admit(&mut self, generation: Generation) -> Result<(), GlassError> {
        if let Some(accepted) = self.admitted
            && generation <= accepted
        {
            return Err(GlassError::StaleLayout {
                accepted: accepted.get(),
                offered: generation.get(),
            });
        }
        self.admitted = Some(generation);
        Ok(())
    }

    pub fn admitted(&self) -> Option<Generation> {
        self.admitted
    }
}

/// Every native material region for one layout pass, stamped with that pass.
///
/// Handed over whole rather than as a diff: a surface that stopped being laid
/// out sends no removal of its own.
pub struct GlassLayout {
    generation: Generation,
    regions: Vec<GlassRegion>,
}

impl GlassLayout {
    pub fn new(generation: Generation) -> Self {
        Self {
            generation,
            regions: Vec::new(),
        }
    }

    /// A zero-area region and a duplicate id are caller bugs, so both are errors.
    pub fn push(&mut self, region: GlassRegion) -> Result<(), GlassError> {
        if !region.bounds.has_area() {
            return Err(GlassError::EmptyRegion(region.id));
        }
        if self.regions.iter().any(|existing| existing.id == region.id) {
            return Err(GlassError::DuplicateRegion(region.id));
        }
        self.regions.push(region);
        Ok(())
    }

    pub fn generation(&self) -> Generation {
        self.generation
    }

    pub fn regions(&self) -> &[GlassRegion] {
        &self.regions
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}

/// The window's native material regions, for as long as this value lives.
///
/// Implementations hold AppKit views, so build, use and drop on the main thread.
/// [`MaterialBacking::Painted`] is what Linux and Windows get, and is a branch
/// every caller still has to handle.
pub trait GlassSurface {
    /// What this surface actually is. Ask once: it cannot change while running.
    fn backing(&self) -> MaterialBacking;

    /// Hands over every region for one layout pass.
    ///
    /// [`PlatformSupport::Unsupported`] on a painted host is the ordinary answer
    /// and not a fault; `Err` means the layout-pass contract was broken.
    fn commit(&mut self, layout: &GlassLayout) -> Result<PlatformSupport, GlassError>;

    /// Removes every region. Needed on the way out of a screen that had them: a
    /// view left behind is invisible but still allocated.
    fn clear(&mut self) -> PlatformSupport;
}

/// Records what it was asked to host instead of hosting it. Built with
/// [`MaterialBacking::Painted`] it is the shape Linux and Windows present.
pub struct InMemoryGlassSurface {
    backing: MaterialBacking,
    gate: LayoutGate,
    regions: Vec<GlassRegion>,
    commits: usize,
}

impl InMemoryGlassSurface {
    pub fn new(backing: MaterialBacking) -> Self {
        Self {
            backing,
            gate: LayoutGate::new(),
            regions: Vec::new(),
            commits: 0,
        }
    }

    pub fn regions(&self) -> &[GlassRegion] {
        &self.regions
    }

    pub fn commits(&self) -> usize {
        self.commits
    }

    pub fn last_layout(&self) -> Option<Generation> {
        self.gate.admitted()
    }
}

impl GlassSurface for InMemoryGlassSurface {
    fn backing(&self) -> MaterialBacking {
        self.backing
    }

    fn commit(&mut self, layout: &GlassLayout) -> Result<PlatformSupport, GlassError> {
        // The gate runs before the host check, so the contract is catchable everywhere.
        self.gate.admit(layout.generation())?;

        if self.backing.is_painted() {
            return Ok(PlatformSupport::Unsupported);
        }

        self.regions.clear();
        self.regions.extend_from_slice(layout.regions());
        self.commits += 1;
        Ok(PlatformSupport::Performed)
    }

    fn clear(&mut self) -> PlatformSupport {
        if self.backing.is_painted() {
            return PlatformSupport::Unsupported;
        }
        self.regions.clear();
        PlatformSupport::Performed
    }
}

/// The backing for `material` on the machine this process is running on.
pub fn current_backing(material: Material) -> MaterialBacking {
    resolve(material, HostOs::current(), macos_version())
}

/// The surface every host without a region material presents.
pub fn painted_glass_surface() -> Box<dyn GlassSurface> {
    Box::new(InMemoryGlassSurface::new(MaterialBacking::Painted))
}

/// The window's native material regions, for the machine this process is
/// running on.
///
/// Off macOS this is [`painted_glass_surface`] and `commit` reports
/// [`PlatformSupport::Unsupported`]. The macOS implementation owns AppKit views,
/// so build, use and drop it on the main thread.
pub fn current_glass_surface() -> Box<dyn GlassSurface> {
    #[cfg(target_os = "macos")]
    {
        Box::new(crate::macos::glass::MacGlassSurface::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        painted_glass_surface()
    }
}

/// The macOS release this process is running on, or `None` when it could not be
/// established — which includes every non-macOS host.
///
/// Read once and cached. macOS reports `10.16` to a process linked against a
/// pre-11.0 SDK, so a build that loses its modern SDK falls to paint, not up.
pub fn macos_version() -> Option<MacOsVersion> {
    static CACHED: std::sync::OnceLock<Option<MacOsVersion>> = std::sync::OnceLock::new();
    *CACHED.get_or_init(read_macos_version)
}

#[cfg(target_os = "macos")]
fn read_macos_version() -> Option<MacOsVersion> {
    // A major version of zero is what a failed or shimmed read looks like.
    let version = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();

    let major = u32::try_from(version.majorVersion).ok()?;
    if major == 0 {
        return None;
    }

    Some(MacOsVersion::new(
        major,
        u32::try_from(version.minorVersion).unwrap_or(0),
        u32::try_from(version.patchVersion).unwrap_or(0),
    ))
}

#[cfg(not(target_os = "macos"))]
fn read_macos_version() -> Option<MacOsVersion> {
    None
}

/// Sets the tier-0 window material and reports whether the OS will really
/// composite one.
///
/// Unsupported means the request was downgraded to opaque before it reached gpui.
pub fn apply_window_material(window: &gpui::Window, requested: WindowMaterial) -> PlatformSupport {
    let granted = granted_window_material(HostOs::current(), requested);
    window.set_background_appearance(granted.into_gpui());

    if granted == requested {
        PlatformSupport::Performed
    } else {
        PlatformSupport::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_core::GenerationCounter;

    fn generations(count: usize) -> Vec<Generation> {
        let counter = GenerationCounter::new();
        (0..count).map(|_| counter.bump()).collect()
    }

    #[test]
    fn no_material_is_glass_on_linux_or_windows() {
        let versions = [
            None,
            Some(MacOsVersion::VIBRANCY),
            Some(MacOsVersion::LIQUID_GLASS),
            Some(MacOsVersion::new(99, 0, 0)),
        ];

        for host in [HostOs::Windows, HostOs::Linux] {
            for material in Material::ALL {
                for version in versions {
                    assert_eq!(
                        resolve(material, host, version),
                        MaterialBacking::Painted,
                        "{material:?} on {host:?} with {version:?} must never be native"
                    );
                }
            }
        }
    }

    /// These read [`offered_backing`] rather than [`resolve`]: the version
    /// policy is what they are about, and `resolve` now caps every material at
    /// painted, which would make them pass without testing anything.
    #[test]
    fn macos_26_is_the_first_release_that_gets_liquid_glass() {
        assert_eq!(
            offered_backing(HostOs::MacOs, Some(MacOsVersion::new(25, 9, 9))),
            MaterialBacking::Vibrancy,
            "the machine must not offer a class that does not exist yet"
        );
        assert_eq!(
            offered_backing(HostOs::MacOs, Some(MacOsVersion::new(26, 0, 0))),
            MaterialBacking::LiquidGlass
        );
    }

    #[test]
    fn a_macos_version_that_could_not_be_read_falls_down_to_painted() {
        assert_eq!(
            offered_backing(HostOs::MacOs, None),
            MaterialBacking::Painted,
            "guessing upward from an unknown version leaves a blank rail"
        );
    }

    #[test]
    fn a_macos_older_than_the_product_floor_falls_down_rather_than_up() {
        // 10.16 is what macOS reports to a process linked against a pre-11.0 SDK.
        for version in [
            MacOsVersion::new(10, 16, 0),
            MacOsVersion::new(12, 7, 6),
            MacOsVersion::new(1, 0, 0),
        ] {
            assert_eq!(
                offered_backing(HostOs::MacOs, Some(version)),
                MaterialBacking::Painted,
                "{version:?} is not a host we have decided anything about"
            );
        }
    }

    #[test]
    fn the_versions_apple_skipped_when_it_renumbered_still_get_vibrancy() {
        for major in 16..=25 {
            assert_eq!(
                MacOsVersion::new(major, 0, 0).backing(),
                MaterialBacking::Vibrancy,
                "macOS {major} was never shipped, but reading one must not fall off the bottom"
            );
        }
    }

    #[test]
    fn a_material_can_only_lower_the_tier_the_machine_offers() {
        let versions = [
            None,
            Some(MacOsVersion::new(10, 16, 0)),
            Some(MacOsVersion::VIBRANCY),
            Some(MacOsVersion::LIQUID_GLASS),
        ];

        for host in HostOs::ALL {
            for version in versions {
                let offered = offered_backing(host, version);
                for material in Material::ALL {
                    assert!(
                        resolve(material, host, version).tier() <= offered.tier(),
                        "{material:?} raised {offered:?} on {host:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn no_surface_takes_a_native_backing_while_the_app_is_an_opaque_plate() {
        for material in Material::ALL {
            assert_eq!(
                material.ceiling(),
                MaterialBacking::Painted,
                "{material:?} would be an AppKit view below the Metal layer, and \
                 the plate in front of it turns that into a hole to the desktop"
            );
        }
    }

    #[test]
    fn the_machine_is_still_asked_even_though_nothing_accepts_the_answer() {
        assert_eq!(
            offered_backing(HostOs::MacOs, Some(MacOsVersion::LIQUID_GLASS)),
            MaterialBacking::LiquidGlass,
            "the tier-1 policy stays live so raising a ceiling is the whole change"
        );
    }

    #[test]
    fn version_ordering_is_major_then_minor_then_patch() {
        let ascending = [
            MacOsVersion::new(10, 16, 0),
            MacOsVersion::new(13, 0, 0),
            MacOsVersion::new(13, 0, 1),
            MacOsVersion::new(13, 4, 0),
            MacOsVersion::new(26, 0, 0),
        ];

        for (index, lower) in ascending.iter().enumerate() {
            for higher in &ascending[index + 1..] {
                assert!(higher > lower, "{higher:?} must sort above {lower:?}");
                assert!(higher.is_at_least(*lower));
                assert!(!lower.is_at_least(*higher));
            }
            assert!(lower.is_at_least(*lower), "at least is inclusive");
        }
    }

    #[test]
    fn a_native_backing_is_exactly_the_one_that_needs_an_appkit_view() {
        for backing in MaterialBacking::ALL {
            assert_eq!(backing.is_native(), backing.view_class().is_some());
            assert_eq!(backing.is_native(), !backing.is_painted());
        }

        assert_eq!(
            MaterialBacking::LiquidGlass.view_class(),
            Some(GLASS_VIEW_CLASS)
        );
        assert_eq!(MaterialBacking::Painted.view_class(), None);
    }

    #[test]
    fn each_surface_names_its_own_vibrancy_material() {
        for material in Material::ALL {
            assert!(
                material
                    .vibrancy_material()
                    .starts_with("NSVisualEffectMaterial."),
                "{material:?} must name a real AppKit case"
            );

            let clashes = Material::ALL
                .iter()
                .filter(|other| other.vibrancy_material() == material.vibrancy_material())
                .count();
            assert_eq!(
                clashes, 1,
                "{material:?} shares its vibrancy material with another surface"
            );
        }
    }

    #[test]
    fn only_macos_composites_a_real_window_material() {
        assert_eq!(window_vibrancy(HostOs::MacOs), WindowVibrancy::Native);
        assert_eq!(
            window_vibrancy(HostOs::Linux),
            WindowVibrancy::SessionDependent
        );
        assert_eq!(
            window_vibrancy(HostOs::Windows),
            WindowVibrancy::Undocumented
        );
    }

    #[test]
    fn asking_for_blur_where_it_is_not_real_leaves_the_window_opaque() {
        assert_eq!(
            granted_window_material(HostOs::MacOs, WindowMaterial::Blurred),
            WindowMaterial::Blurred
        );

        for host in [HostOs::Windows, HostOs::Linux] {
            assert_eq!(
                granted_window_material(host, WindowMaterial::Blurred),
                WindowMaterial::Opaque,
                "a non-opaque window with nothing behind it is see-through, not frosted"
            );
        }
    }

    /// The mac answer is the window's CORNER, not a material: AppKit rounds a
    /// titled window by leaving its corner pixels clear, and an opaque window
    /// has none to leave.
    #[test]
    fn the_shell_asks_for_the_window_material_its_corner_needs() {
        assert_eq!(
            preferred_window_material(HostOs::MacOs),
            WindowMaterial::Transparent,
            "an opaque window is a square one: AppKit has no clear pixels to \
             round it with"
        );

        for host in [HostOs::Windows, HostOs::Linux] {
            assert_eq!(
                preferred_window_material(host),
                WindowMaterial::Opaque,
                "{host:?} does not round the window out of the surface's alpha, \
                 and the app paints every pixel of its window"
            );
        }
    }

    /// Transparency here is for the corners and nothing else, so it must not
    /// come back as a material that puts something behind the whole window.
    #[test]
    fn the_preferred_window_material_is_never_blurred() {
        for host in HostOs::ALL {
            assert_ne!(preferred_window_material(host), WindowMaterial::Blurred);
        }
    }

    #[test]
    fn an_opaque_window_is_granted_on_every_host() {
        for host in HostOs::ALL {
            assert_eq!(
                granted_window_material(host, WindowMaterial::Opaque),
                WindowMaterial::Opaque
            );
        }
    }

    #[test]
    fn the_window_material_maps_onto_the_gpui_appearance() {
        assert_eq!(
            WindowMaterial::Opaque.into_gpui(),
            gpui::WindowBackgroundAppearance::Opaque
        );
        assert_eq!(
            WindowMaterial::Transparent.into_gpui(),
            gpui::WindowBackgroundAppearance::Transparent
        );
        assert_eq!(
            WindowMaterial::Blurred.into_gpui(),
            gpui::WindowBackgroundAppearance::Blurred
        );
    }

    #[test]
    fn downgrading_the_window_material_is_a_fixed_point() {
        for host in HostOs::ALL {
            for requested in WindowMaterial::ALL {
                let granted = granted_window_material(host, requested);
                assert_eq!(
                    granted_window_material(host, granted),
                    granted,
                    "re-applying what the window already is must not move it again"
                );
            }
        }
    }

    #[test]
    fn the_window_material_is_answered_without_the_macos_version() {
        let floor = offered_backing(HostOs::MacOs, Some(MacOsVersion::VIBRANCY));
        let latest = offered_backing(HostOs::MacOs, Some(MacOsVersion::LIQUID_GLASS));
        assert_ne!(
            floor, latest,
            "the per-region tier moves with the OS version"
        );

        assert_eq!(
            granted_window_material(HostOs::MacOs, WindowMaterial::Blurred),
            WindowMaterial::Blurred,
            "tier 0 is a view gpui already ships and is real on every macOS we run on"
        );
    }

    #[test]
    fn a_second_batch_from_the_same_layout_pass_is_refused() {
        let passes = generations(1);
        let mut gate = LayoutGate::new();

        gate.admit(passes[0]).unwrap();
        assert_eq!(
            gate.admit(passes[0]),
            Err(GlassError::StaleLayout {
                accepted: passes[0].get(),
                offered: passes[0].get(),
            }),
            "a render pass has no new generation and must not move a region"
        );
    }

    #[test]
    fn regions_move_only_when_a_newer_layout_pass_says_so() {
        let passes = generations(3);
        let mut gate = LayoutGate::new();

        for pass in &passes {
            gate.admit(*pass).unwrap();
        }
        assert_eq!(gate.admitted(), Some(passes[2]));

        assert!(
            gate.admit(passes[0]).is_err(),
            "a batch from an older layout must not overwrite a newer one"
        );
        assert_eq!(gate.admitted(), Some(passes[2]));
    }

    #[test]
    fn a_stale_batch_is_refused_on_a_host_that_has_no_glass_at_all() {
        let passes = generations(1);
        let mut surface = InMemoryGlassSurface::new(MaterialBacking::Painted);
        let layout = GlassLayout::new(passes[0]);

        surface.commit(&layout).unwrap();
        assert!(
            surface.commit(&layout).is_err(),
            "the contract must be catchable on the platform the developer is on"
        );
    }

    #[test]
    fn a_region_with_no_area_never_reaches_the_platform() {
        let passes = generations(1);
        let mut layout = GlassLayout::new(passes[0]);

        for bounds in [
            RegionRect::new(0.0, 0.0, 0.0, 800.0),
            RegionRect::new(0.0, 0.0, 56.0, 0.0),
            RegionRect::new(0.0, 0.0, f32::NAN, 800.0),
            RegionRect::new(0.0, 0.0, -56.0, 800.0),
        ] {
            assert_eq!(
                layout.push(GlassRegion::new(RegionId(1), Material::Chrome, bounds)),
                Err(GlassError::EmptyRegion(RegionId(1))),
                "{bounds:?} is a view with an undefined frame"
            );
        }

        assert!(layout.is_empty());
    }

    #[test]
    fn two_regions_cannot_share_one_identity() {
        let passes = generations(1);
        let mut layout = GlassLayout::new(passes[0]);
        let rail = RegionRect::new(0.0, 0.0, 56.0, 800.0);
        let list = RegionRect::new(56.0, 0.0, 320.0, 800.0);

        layout
            .push(GlassRegion::new(RegionId(1), Material::Chrome, rail))
            .unwrap();
        assert_eq!(
            layout.push(GlassRegion::new(RegionId(1), Material::Panel, list)),
            Err(GlassError::DuplicateRegion(RegionId(1))),
            "one id must map to one view or the pass moves it twice"
        );

        layout
            .push(GlassRegion::new(RegionId(2), Material::Panel, list))
            .unwrap();
        assert_eq!(layout.regions().len(), 2);
    }

    #[test]
    fn a_painted_surface_says_unsupported_instead_of_pretending_it_drew_glass() {
        let passes = generations(1);
        let mut surface = InMemoryGlassSurface::new(MaterialBacking::Painted);
        let mut layout = GlassLayout::new(passes[0]);
        layout
            .push(GlassRegion::new(
                RegionId(1),
                Material::Chrome,
                RegionRect::new(0.0, 0.0, 56.0, 800.0),
            ))
            .unwrap();

        assert_eq!(
            surface.commit(&layout).unwrap(),
            PlatformSupport::Unsupported
        );
        assert!(
            surface.regions().is_empty(),
            "a caller must be able to tell that it has to paint the material itself"
        );
        assert_eq!(surface.clear(), PlatformSupport::Unsupported);
    }

    #[test]
    fn a_commit_replaces_the_previous_regions_rather_than_adding_to_them() {
        let passes = generations(2);
        let mut surface = InMemoryGlassSurface::new(MaterialBacking::LiquidGlass);

        let mut first = GlassLayout::new(passes[0]);
        first
            .push(
                GlassRegion::new(
                    RegionId(1),
                    Material::Chrome,
                    RegionRect::new(0.0, 0.0, 56.0, 800.0),
                )
                .with_corner_radius(12.0),
            )
            .unwrap();
        first
            .push(GlassRegion::new(
                RegionId(2),
                Material::Panel,
                RegionRect::new(56.0, 0.0, 320.0, 800.0),
            ))
            .unwrap();
        assert_eq!(surface.commit(&first).unwrap(), PlatformSupport::Performed);
        assert_eq!(surface.regions().len(), 2);
        assert_eq!(surface.regions()[0].corner_radius, 12.0);

        let mut second = GlassLayout::new(passes[1]);
        second
            .push(GlassRegion::new(
                RegionId(1),
                Material::Chrome,
                RegionRect::new(0.0, 0.0, 56.0, 600.0),
            ))
            .unwrap();
        surface.commit(&second).unwrap();

        assert_eq!(
            surface.regions().len(),
            1,
            "a surface that stopped being laid out sends no removal of its own"
        );
        assert_eq!(surface.regions()[0].bounds.height, 600.0);
        assert_eq!(surface.commits(), 2);
        assert_eq!(surface.last_layout(), Some(passes[1]));
    }

    #[test]
    fn clearing_removes_every_region_a_native_surface_was_holding() {
        let passes = generations(1);
        let mut surface = InMemoryGlassSurface::new(MaterialBacking::Vibrancy);
        let mut layout = GlassLayout::new(passes[0]);
        layout
            .push(GlassRegion::new(
                RegionId(1),
                Material::Overlay,
                RegionRect::new(100.0, 100.0, 300.0, 200.0),
            ))
            .unwrap();

        surface.commit(&layout).unwrap();
        assert_eq!(surface.clear(), PlatformSupport::Performed);
        assert!(surface.regions().is_empty());
    }

    #[test]
    fn a_layout_rectangle_survives_the_conversion_from_gpui() {
        let bounds = gpui::Bounds {
            origin: gpui::point(gpui::px(56.0), gpui::px(28.0)),
            size: gpui::size(gpui::px(320.0), gpui::px(772.0)),
        };

        assert_eq!(
            RegionRect::from_gpui(bounds),
            RegionRect::new(56.0, 28.0, 320.0, 772.0)
        );
        assert!(RegionRect::from_gpui(bounds).has_area());
    }

    #[test]
    fn this_machine_reports_a_macos_version_only_on_macos() {
        assert_eq!(macos_version().is_some(), cfg!(target_os = "macos"));
    }

    #[test]
    fn the_surface_a_host_without_region_materials_gets_is_still_usable() {
        let passes = generations(2);
        let mut surface = painted_glass_surface();
        assert_eq!(surface.backing(), MaterialBacking::Painted);

        let mut layout = GlassLayout::new(passes[0]);
        layout
            .push(GlassRegion::new(
                RegionId(1),
                Material::Chrome,
                RegionRect::new(0.0, 0.0, 56.0, 800.0),
            ))
            .unwrap();

        assert_eq!(
            surface.commit(&layout).unwrap(),
            PlatformSupport::Unsupported,
            "the caller paints the material itself and carries on"
        );
        assert!(surface.commit(&layout).is_err(), "the gate still applies");

        let newer = GlassLayout::new(passes[1]);
        assert_eq!(
            surface.commit(&newer).unwrap(),
            PlatformSupport::Unsupported
        );
        assert_eq!(surface.clear(), PlatformSupport::Unsupported);
    }

    #[test]
    fn the_surface_this_machine_gets_agrees_with_the_backing_it_resolves() {
        let surface = current_glass_surface();
        let offered = offered_backing(HostOs::current(), macos_version());

        assert!(
            surface.backing().tier() <= offered.tier(),
            "a surface cannot host more than the machine offers"
        );

        if !cfg!(target_os = "macos") {
            assert_eq!(surface.backing(), MaterialBacking::Painted);
        }
    }

    #[test]
    fn nothing_on_this_machine_claims_glass_without_a_version_that_has_it() {
        for material in Material::ALL {
            if current_backing(material) == MaterialBacking::LiquidGlass {
                let version =
                    macos_version().expect("glass was claimed with no version to back it");
                assert!(
                    version.is_at_least(MacOsVersion::LIQUID_GLASS),
                    "{version:?} predates {:?}",
                    MacOsVersion::LIQUID_GLASS
                );
            }
        }
    }
}
