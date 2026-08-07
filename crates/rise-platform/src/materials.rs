//! Which materials are real on this machine.
//!
//! The rule this file exists to enforce is `docs/PLATFORM_GLASS.md`: **no
//! component ever asks for glass.** A component asks for a [`Material`] and this
//! seam decides what that material actually is here. A call site that could name
//! glass directly would be a hole on Linux, and nobody would notice until a
//! screenshot arrived; routing through a material keeps the fallback a property
//! of the design system instead of an afterthought at every call site.
//!
//! Nothing here picks a colour. The painted form of a material is a rise-theme
//! token; this file only decides whether painting is what happens.
//!
//! Two tiers live here and must not be conflated:
//!
//! - The **window** material, [`WindowMaterial`], is tier 0 and is already free.
//!   `gpui_macos` ships an `NSVisualEffectView` subclass and honours
//!   `WindowBackgroundAppearance::Blurred`, so it needs no native code of ours
//!   at all. It is modelled apart from [`Material`] precisely so that the Swift
//!   work in Phase 5 does not re-solve a problem that is already solved.
//! - The **per-region** materials, [`Material`], are tiers 1 and 2. Tier 1 is an
//!   AppKit view hosted below the Metal layer, which this crate does own — it is
//!   `crate::macos::glass`, the Rust half of the `RiseGlass` bridge; tier 2 is
//!   paint. The policy that chooses between them is here and is exercised for
//!   all three platforms from a Mac.

use rise_core::Generation;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// What a surface asks the platform for.
///
/// The shell's mapping, from `docs/PLATFORM_GLASS.md`: the left rail is
/// [`Material::Chrome`], the list column is [`Material::Panel`], and sheets and
/// popovers are [`Material::Overlay`]. Content is opaque and is deliberately not
/// a material — a surface that is never translucent has no fallback to get
/// wrong, and giving it one would invite glass behind dense scrolling text,
/// which is the one place Apple's own guidance says not to put it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Material {
    /// The left rail: the strip that carries what the phone puts in a tab bar.
    Chrome,
    /// The list column beside the rail — chats, folders, search results.
    Panel,
    /// Sheets and popovers. In-window on every platform, because gpui has no
    /// native popups on X11 or Windows and the design must never rely on
    /// leaving the window rectangle.
    Overlay,
}

/// The AppKit class behind [`MaterialBacking::LiquidGlass`].
///
/// Only the class is named. Its configuration API is Swift-first and changes
/// shape between OS versions, which is exactly why `docs/PLATFORM_GLASS.md`
/// puts that part in a Swift target rather than in hand-rolled message sends;
/// naming its style constants from Rust would be guessing at an API this crate
/// cannot see.
pub const GLASS_VIEW_CLASS: &str = "NSGlassEffectView";

/// The AppKit class behind [`MaterialBacking::Vibrancy`].
pub const VIBRANCY_VIEW_CLASS: &str = "NSVisualEffectView";

impl Material {
    pub const ALL: [Self; 3] = [Self::Chrome, Self::Panel, Self::Overlay];

    /// The `NSVisualEffectMaterial` case a tier-1 vibrancy region is configured
    /// with. Vibrancy materials are semantic rather than decorative: AppKit
    /// picks the blur radius, the tint and the behaviour behind an inactive
    /// window from this case, so choosing one by how it looks in a screenshot
    /// gets the inactive and the increased-contrast appearances wrong.
    ///
    /// [`Material::Overlay`] is `.popover` and not `.sheet` because our sheets
    /// never leave the window; `.sheet` is the case AppKit means for a real
    /// document-modal sheet, which is not what this is.
    pub const fn vibrancy_material(self) -> &'static str {
        match self {
            Self::Chrome => "NSVisualEffectMaterial.headerView",
            Self::Panel => "NSVisualEffectMaterial.sidebar",
            Self::Overlay => "NSVisualEffectMaterial.popover",
        }
    }

    /// The best backing this surface will accept, whatever the machine offers.
    ///
    /// All three are currently glass, because `docs/PLATFORM_GLASS.md` names the
    /// rail, the sidebar, the composer bar, sheets and popovers as glass
    /// regions. The ceiling exists so that the judgement can change in one
    /// place: if the list column ever fails the legibility check with dense text
    /// scrolling over it at one of the two appearances, lowering it here demotes
    /// that surface everywhere and no call site changes.
    pub const fn ceiling(self) -> MaterialBacking {
        match self {
            Self::Chrome | Self::Panel | Self::Overlay => MaterialBacking::LiquidGlass,
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
    /// Theme tokens: a translucent fill, a hairline border, a soft inner
    /// highlight. It reads as a deliberate flat design rather than a broken
    /// attempt at glass, which is the whole point of having a named tier for it.
    Painted,
}

impl MaterialBacking {
    pub const ALL: [Self; 3] = [Self::LiquidGlass, Self::Vibrancy, Self::Painted];

    /// Ascending: painted 0, vibrancy 1, glass 2.
    ///
    /// Written out rather than derived from the declaration order, so that
    /// reordering the variants cannot silently invert which way the fallback
    /// falls.
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
    /// The distinction is not cosmetic. A native backing is a real view sitting
    /// *below* the Metal layer, so GPUI has to leave that rectangle transparent,
    /// the rectangle has to be registered with the platform, and anything GPUI
    /// draws there lands on top of it. A painted backing is an ordinary element
    /// in the scene with none of that bookkeeping.
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
/// The derived ordering is lexicographic over the fields in declaration order,
/// so that order is load-bearing. [`MacOsVersion::is_at_least`] exists beside it
/// because the comparison operators are not usable from a `const fn`, and a test
/// asserts the two never disagree.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MacOsVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl MacOsVersion {
    /// The first release with [`GLASS_VIEW_CLASS`].
    ///
    /// Apple renumbered to the year scheme at this release, so 16 through 25
    /// were never shipped. The boundary is therefore written as "at least 26"
    /// rather than "after 15": a machine that reports something inside that gap
    /// — a beta, a virtual machine, a compatibility shim — lands on vibrancy
    /// instead of falling off the bottom into paint.
    pub const LIQUID_GLASS: Self = Self::new(26, 0, 0);

    /// The oldest release this product runs on, matching `LSMinimumSystemVersion`
    /// in the bundle. `NSVisualEffectView` is far older than this; the floor is
    /// the product's, not the API's, and anything below it is a host we make no
    /// claims about rather than one we know is missing vibrancy.
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

    /// The best tier this release can host.
    ///
    /// Unknown falls *down*. A version below [`MacOsVersion::VIBRANCY`] is not a
    /// release we have decided anything about, and guessing upward is the
    /// expensive mistake: assuming glass and getting nothing leaves a blank
    /// rail, while assuming paint and getting a flat panel is merely plainer.
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
/// `version` is `None` both off macOS and when the running version could not be
/// established, and both answer paint — see [`MacOsVersion::backing`] for why
/// the unknown case falls down rather than up.
pub const fn offered_backing(host: HostOs, version: Option<MacOsVersion>) -> MaterialBacking {
    match host {
        HostOs::MacOs => match version {
            Some(version) => version.backing(),
            None => MaterialBacking::Painted,
        },
        // Not "not implemented yet": there is no equivalent. Neither platform
        // can put a system material behind a *region* of a window, and GPUI
        // renders the whole window into one layer with no backdrop filter, so a
        // shader here could imitate a blur but never sample what is behind it.
        HostOs::Windows | HostOs::Linux => MaterialBacking::Painted,
    }
}

/// What `material` actually is on this host and this OS version.
///
/// The one function every call site is downstream of. It takes the material
/// rather than answering a single question per machine so that a surface can
/// decline a tier the machine offers ([`Material::ceiling`]) — the cap can only
/// ever lower, never raise.
pub const fn resolve(
    material: Material,
    host: HostOs,
    version: Option<MacOsVersion>,
) -> MaterialBacking {
    offered_backing(host, version).capped_at(material.ceiling())
}

/// The whole-window material: tier 0, and the only tier that costs us nothing.
///
/// Kept apart from [`Material`] because it is a different mechanism with a
/// different owner. gpui already implements it on every backend; a region
/// material does not exist anywhere yet. Conflating the two would hide the fact
/// that the app background is solved and the rail is not.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WindowMaterial {
    Opaque,
    /// A transparent Metal layer over a real system material.
    ///
    /// Not free everywhere it works: gpui turns subpixel text rendering off for
    /// any non-opaque window background. That costs nothing on macOS, whose
    /// backend never offered subpixel rendering at all, and costs real text
    /// quality on the Windows and Linux backends, which do offer it.
    Blurred,
}

impl WindowMaterial {
    pub const ALL: [Self; 2] = [Self::Opaque, Self::Blurred];

    pub const fn into_gpui(self) -> gpui::WindowBackgroundAppearance {
        match self {
            Self::Opaque => gpui::WindowBackgroundAppearance::Opaque,
            Self::Blurred => gpui::WindowBackgroundAppearance::Blurred,
        }
    }
}

/// Whether asking gpui for a blurred window background produces one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WindowVibrancy {
    /// `gpui_macos` inserts an `NSVisualEffectView` subclass below the Metal
    /// layer and the OS composites it. No native code of ours is involved, which
    /// is why this tier is available now and tier 1 is not.
    Native,
    /// Only the running session can answer, and the failure is not a missing
    /// blur. gpui clears the surface's opaque region for any non-opaque
    /// appearance, and adds a blur object only where the compositor exports the
    /// blur protocol — X11 exports none at all. Where it does not, the window
    /// ends up see-through with the desktop showing straight through the app
    /// background, which is worse than never asking.
    SessionDependent,
    /// Reached through `SetWindowCompositionAttribute`, which Microsoft has
    /// never documented, and it is not obviously the call we would want anyway:
    /// Windows 11 has Mica, which gpui exposes as its own appearance through a
    /// documented DWM API. Choosing between the two is Windows work nobody has
    /// done, and guessing is how an app ends up leaning on a blur that a future
    /// Windows update takes away.
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
/// Blur is granted only where it is [`WindowVibrancy::Native`]. Substituting
/// opaque elsewhere is deliberate and is not a silent no-op — [`apply_window_material`]
/// reports the substitution — because forwarding the request would hand the user
/// a transparent window on a Linux session with no blur protocol, and would trade
/// subpixel text for an undocumented call on Windows.
pub const fn granted_window_material(host: HostOs, requested: WindowMaterial) -> WindowMaterial {
    match requested {
        WindowMaterial::Opaque => WindowMaterial::Opaque,
        WindowMaterial::Blurred => match window_vibrancy(host) {
            WindowVibrancy::Native => WindowMaterial::Blurred,
            WindowVibrancy::SessionDependent | WindowVibrancy::Undocumented => {
                WindowMaterial::Opaque
            }
        },
    }
}

/// The shell's own choice for this platform: a real system material behind the
/// app wherever one exists.
pub const fn preferred_window_material(host: HostOs) -> WindowMaterial {
    granted_window_material(host, WindowMaterial::Blurred)
}

/// A region of the window that a native material is hosted behind.
///
/// The identity is stable across layout passes so the platform side can move a
/// view it already has instead of tearing every view down and building it again
/// every pass. Allocation of the numbers belongs to the shell; this crate only
/// requires that one surface keeps one number for as long as it exists.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RegionId(pub u64);

/// A rectangle in the window's logical pixels, top-left origin.
///
/// Deliberately not `gpui::Bounds`: the platform layer hands these to AppKit,
/// and nothing above this crate should have to hold a renderer type to describe
/// where its own sidebar is.
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

    /// A comparison rather than a subtraction, so a NaN from a layout that
    /// divided by a zero-width parent answers false and never reaches AppKit,
    /// where the same value is a view with an undefined frame.
    pub fn has_area(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}

/// One native material region, as the platform layer receives it.
///
/// A region is background and never a mid-stack layer. The view sits below the
/// Metal layer, so it cannot be put over GPUI content — anything drawn in its
/// rectangle appears on top of it — and a design that stacks something under a
/// glass surface has no way to be built. `docs/PLATFORM_GLASS.md` says to design
/// so that never comes up; this type cannot enforce it, only name it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GlassRegion {
    pub id: RegionId,
    pub material: Material,
    pub bounds: RegionRect,
    /// Rounding the *platform* applies, in logical pixels.
    ///
    /// A painted material is rounded by the renderer like any other element. A
    /// native one cannot be: the view is below the Metal layer, so GPUI drawing
    /// a rounded mask over it would draw on top of it rather than clip it. The
    /// radius therefore has to travel with the region, and it has to match the
    /// painted form's radius or the two tiers look like different designs.
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
/// This is the constraint in `docs/PLATFORM_GLASS.md` made mechanical rather
/// than left in a comment: a glass region's rectangle must be static for the
/// duration of a frame and must be updated from the **layout** pass, never from
/// a render pass. A render pass has no new [`Generation`] to offer, so it cannot
/// get through this gate, and a region animated frame by frame — which is how
/// the rectangle desynchronises from the content drawn around it — stops being
/// expressible rather than merely discouraged.
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
/// Built during layout and handed over whole rather than as a diff. Whole,
/// because the platform side has to know which views should no longer exist, and
/// a surface that simply stopped being laid out sends no removal of its own.
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

    /// Both refusals are caller bugs rather than platform conditions, which is
    /// why they are errors and not [`PlatformSupport::Unsupported`]: a
    /// zero-area region is a view the compositor draws nothing into, and two
    /// regions sharing an id would fight over one AppKit view, moving it twice
    /// per pass.
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
/// Deliberately neither `Send` nor `Sync`, like `Tray` and for the same reason:
/// the implementation this seam exists for holds AppKit views, and AppKit off
/// the main thread is undefined behaviour rather than a lint.
///
/// The implementation that hosts real [`GLASS_VIEW_CLASS`] views is
/// `crate::macos::glass::MacGlassSurface`, over the `RiseGlass` Swift target
/// described in `docs/PLATFORM_GLASS.md`; it is Swift rather than `objc2`
/// because the view's configuration API is Swift-first and shifts between OS
/// versions. [`current_glass_surface`] returns it on macOS and
/// [`painted_glass_surface`] everywhere else, so [`MaterialBacking::Painted`] is
/// what Linux and Windows get and is a branch every caller still has to handle.
pub trait GlassSurface {
    /// What this surface actually is. Ask once and keep the answer: it cannot
    /// change while the process runs, and a caller that re-asks per frame is
    /// paying for a decision that was made at startup.
    fn backing(&self) -> MaterialBacking;

    /// Hands over every region for one layout pass.
    ///
    /// Reports [`PlatformSupport::Unsupported`] on a painted host, which is the
    /// ordinary answer on Linux and Windows and not a fault; the caller paints
    /// the material itself and carries on. `Err` means the layout-pass contract
    /// was broken, which is a bug on any platform.
    fn commit(&mut self, layout: &GlassLayout) -> Result<PlatformSupport, GlassError>;

    /// Removes every region. Needed on the way out of a screen that had them:
    /// a view left behind under a Metal layer that no longer leaves a hole for
    /// it is invisible, and stays allocated.
    fn clear(&mut self) -> PlatformSupport;
}

/// Records what it was asked to host instead of hosting it.
///
/// For tests above this crate, and for the case every caller has to handle
/// anyway: built with [`MaterialBacking::Painted`] it is the shape Linux and
/// Windows present, and code that has only ever run against a native backing is
/// code that has never taken that branch.
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
        // The gate runs before the host check on purpose. A violation of the
        // layout-pass contract is a bug everywhere, and gating it behind a
        // native backing would mean it could only ever be caught on the one
        // platform that has glass.
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
///
/// Named rather than inlined into [`current_glass_surface`] because it is the
/// branch that never runs on the machine this is developed on, and a value with
/// a name can be exercised here anyway.
pub fn painted_glass_surface() -> Box<dyn GlassSurface> {
    Box::new(InMemoryGlassSurface::new(MaterialBacking::Painted))
}

/// The window's native material regions, for the machine this process is
/// running on.
///
/// The caller never learns which OS that is. Off macOS, and on a macOS whose
/// version or SDK cannot host a region material, this is
/// [`painted_glass_surface`]: `commit` reports [`PlatformSupport::Unsupported`]
/// and the caller paints the material from theme tokens, which is the ordinary
/// answer rather than a fault.
///
/// Cheap to build and expensive to hold: the macOS implementation owns AppKit
/// views, so the returned value is neither `Send` nor `Sync` and must be built,
/// used and dropped on the main thread.
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
/// Read once and cached, because the value cannot change under a running
/// process and the resolution above it is on the layout path.
///
/// The trap worth knowing about is that this number is not always the truth.
/// macOS reports `10.16` to a process linked against an SDK older than 11.0, so
/// a build that loses its modern SDK starts reporting a version below
/// [`MacOsVersion::VIBRANCY`]. That answer costs a flat panel and nothing else,
/// which is the direction this seam is deliberately wrong in.
pub fn macos_version() -> Option<MacOsVersion> {
    static CACHED: std::sync::OnceLock<Option<MacOsVersion>> = std::sync::OnceLock::new();
    *CACHED.get_or_init(read_macos_version)
}

// ---------------------------------------------------------------------------
// Bindings. No decisions live below this line.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn read_macos_version() -> Option<MacOsVersion> {
    // `NSProcessInfo` is Foundation, so it is present on every macOS this can
    // run on, and `objc2-foundation` 0.3 exposes both calls as safe `fn`s. The
    // one thing that can still come back wrong is the value, so a major version
    // of zero — which is what a failed or shimmed read looks like — is treated
    // as "not established" rather than as "older than everything".
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
/// Unsupported means the request was downgraded to opaque before it reached
/// gpui; see [`granted_window_material`] for why forwarding it would be worse
/// than refusing it.
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

    #[test]
    fn macos_26_is_the_first_release_that_gets_liquid_glass() {
        for material in Material::ALL {
            assert_eq!(
                resolve(material, HostOs::MacOs, Some(MacOsVersion::new(25, 9, 9))),
                MaterialBacking::Vibrancy,
                "{material:?} must not reach for a class that does not exist yet"
            );
            assert_eq!(
                resolve(material, HostOs::MacOs, Some(MacOsVersion::new(26, 0, 0))),
                MaterialBacking::LiquidGlass
            );
        }
    }

    #[test]
    fn a_macos_version_that_could_not_be_read_falls_down_to_painted() {
        for material in Material::ALL {
            assert_eq!(
                resolve(material, HostOs::MacOs, None),
                MaterialBacking::Painted,
                "guessing upward from an unknown version leaves a blank rail"
            );
        }
    }

    #[test]
    fn a_macos_older_than_the_product_floor_falls_down_rather_than_up() {
        // 10.16 is what macOS 11 and later report to a process linked against a
        // pre-11.0 SDK, so it is a version we can actually be handed.
        for version in [
            MacOsVersion::new(10, 16, 0),
            MacOsVersion::new(12, 7, 6),
            MacOsVersion::new(1, 0, 0),
        ] {
            for material in Material::ALL {
                assert_eq!(
                    resolve(material, HostOs::MacOs, Some(version)),
                    MaterialBacking::Painted,
                    "{version:?} is not a host we have decided anything about"
                );
            }
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
    fn every_surface_the_shell_uses_takes_glass_where_the_machine_has_it() {
        for material in Material::ALL {
            assert_eq!(
                material.ceiling(),
                MaterialBacking::LiquidGlass,
                "PLATFORM_GLASS.md names the rail, the sidebar and the overlays as glass regions"
            );
        }
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
            preferred_window_material(HostOs::MacOs),
            WindowMaterial::Blurred
        );

        for host in [HostOs::Windows, HostOs::Linux] {
            assert_eq!(
                preferred_window_material(host),
                WindowMaterial::Opaque,
                "a non-opaque window with nothing behind it is see-through, not frosted"
            );
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
            preferred_window_material(HostOs::MacOs),
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
        // This is exactly the value `current_glass_surface` returns off macOS,
        // which is the branch that never runs on the machine this is written
        // on. Naming it is what makes it testable here.
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
