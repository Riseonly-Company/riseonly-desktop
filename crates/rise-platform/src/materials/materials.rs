use rise_core::Generation;
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// what a surface asks the platform for
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Material {
    Chrome,
    Panel,
    Overlay,
}

pub const GLASS_VIEW_CLASS: &str = "NSGlassEffectView";

pub const VIBRANCY_VIEW_CLASS: &str = "NSVisualEffectView";

impl Material {
    pub const ALL: [Self; 3] = [Self::Chrome, Self::Panel, Self::Overlay];

    /// vibrancy material case this uses, the cases are semantic
    pub const fn vibrancy_material(self) -> &'static str {
        match self {
            Self::Chrome => "NSVisualEffectMaterial.headerView",
            Self::Panel => "NSVisualEffectMaterial.sidebar",
            Self::Overlay => "NSVisualEffectMaterial.popover",
        }
    }

    /// best backing this surface accepts
    /// painted for now, a native region would punch a hole to the desktop
    pub const fn ceiling(self) -> MaterialBacking {
        match self {
            Self::Chrome | Self::Panel | Self::Overlay => MaterialBacking::Painted,
        }
    }
}

/// what a material actually is on this machine
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MaterialBacking {
    LiquidGlass,
    Vibrancy,
    Painted,
}

impl MaterialBacking {
    pub const ALL: [Self; 3] = [Self::LiquidGlass, Self::Vibrancy, Self::Painted];

    /// ascending tier, painted 0 vibrancy 1 glass 2
    pub const fn tier(self) -> u8 {
        match self {
            Self::Painted => 0,
            Self::Vibrancy => 1,
            Self::LiquidGlass => 2,
        }
    }

    /// lower of two tiers, it can only lower
    pub const fn capped_at(self, ceiling: Self) -> Self {
        if self.tier() <= ceiling.tier() {
            self
        } else {
            ceiling
        }
    }

    /// true when this is an appkit view under the metal layer, not a gpui drawing
    pub const fn is_native(self) -> bool {
        self.view_class().is_some()
    }

    pub const fn is_painted(self) -> bool {
        matches!(self, Self::Painted)
    }

    /// class the platform layer must make, none when there is none
    pub const fn view_class(self) -> Option<&'static str> {
        match self {
            Self::LiquidGlass => Some(GLASS_VIEW_CLASS),
            Self::Vibrancy => Some(VIBRANCY_VIEW_CLASS),
            Self::Painted => None,
        }
    }
}

/// a macos release version, ordering follows field order
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MacOsVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl MacOsVersion {
    pub const LIQUID_GLASS: Self = Self::new(26, 0, 0);

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

    /// best tier this release can host, falls to paint below the vibrancy version
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

/// best tier this machine can host before any per material ceiling, none means paint
pub const fn offered_backing(host: HostOs, version: Option<MacOsVersion>) -> MaterialBacking {
    match host {
        HostOs::MacOs => match version {
            Some(version) => version.backing(),
            None => MaterialBacking::Painted,
        },
        HostOs::Windows | HostOs::Linux => MaterialBacking::Painted,
    }
}

/// what the material really is on this host and version, the ceiling can only lower
pub const fn resolve(
    material: Material,
    host: HostOs,
    version: Option<MacOsVersion>,
) -> MaterialBacking {
    offered_backing(host, version).capped_at(material.ceiling())
}

/// whole window material, tier 0 and a different mechanism than Material
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WindowMaterial {
    Opaque,
    Transparent,
    Blurred,
    Mica,
}

impl WindowMaterial {
    pub const ALL: [Self; 4] = [Self::Opaque, Self::Transparent, Self::Blurred, Self::Mica];

    pub const fn into_gpui(self) -> gpui::WindowBackgroundAppearance {
        match self {
            Self::Opaque => gpui::WindowBackgroundAppearance::Opaque,
            Self::Transparent => gpui::WindowBackgroundAppearance::Transparent,
            Self::Blurred => gpui::WindowBackgroundAppearance::Blurred,
            Self::Mica => gpui::WindowBackgroundAppearance::MicaBackdrop,
        }
    }
}

/// windows build number this process runs on
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct WindowsBuild(pub u32);

impl WindowsBuild {
    pub const SYSTEM_BACKDROP: Self = Self(22_621);

    pub const fn is_at_least(self, other: Self) -> bool {
        self.0 >= other.0
    }
}

/// true when this host and build draw the mica backdrop
pub const fn mica_available(host: HostOs, build: Option<WindowsBuild>) -> bool {
    match (host, build) {
        (HostOs::Windows, Some(build)) => build.is_at_least(WindowsBuild::SYSTEM_BACKDROP),
        _ => false,
    }
}

/// whether a blurred window background request actually gives a blur
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum WindowVibrancy {
    Native,
    SessionDependent,
    Undocumented,
}

pub const fn window_vibrancy(host: HostOs) -> WindowVibrancy {
    match host {
        HostOs::MacOs => WindowVibrancy::Native,
        HostOs::Windows => WindowVibrancy::Undocumented,
        HostOs::Linux => WindowVibrancy::SessionDependent,
    }
}

/// what the window is set to on this build
/// blur only if native and mica only if available, anything else is opaque
pub const fn granted_window_material_on(
    host: HostOs,
    build: Option<WindowsBuild>,
    requested: WindowMaterial,
) -> WindowMaterial {
    match requested {
        WindowMaterial::Opaque => WindowMaterial::Opaque,
        WindowMaterial::Transparent => WindowMaterial::Transparent,
        WindowMaterial::Blurred => match window_vibrancy(host) {
            WindowVibrancy::Native => WindowMaterial::Blurred,
            WindowVibrancy::SessionDependent | WindowVibrancy::Undocumented => {
                WindowMaterial::Opaque
            }
        },
        WindowMaterial::Mica => {
            if mica_available(host, build) {
                WindowMaterial::Mica
            } else {
                WindowMaterial::Opaque
            }
        }
    }
}

/// what the window is set to on this machine windows build, none off windows
pub fn granted_window_material(host: HostOs, requested: WindowMaterial) -> WindowMaterial {
    granted_window_material_on(host, windows_build(), requested)
}

/// the shell choice on this build
/// mac asks transparent because an opaque window cant have rounded corners
pub const fn preferred_window_material_on(
    host: HostOs,
    build: Option<WindowsBuild>,
) -> WindowMaterial {
    let requested = match host {
        HostOs::MacOs => WindowMaterial::Transparent,
        HostOs::Windows => WindowMaterial::Mica,
        HostOs::Linux => WindowMaterial::Opaque,
    };

    granted_window_material_on(host, build, requested)
}

/// the shell choice on this machine
pub fn preferred_window_material(host: HostOs) -> WindowMaterial {
    preferred_window_material_on(host, windows_build())
}

/// window region a native material sits behind, one surface keeps one id for life
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RegionId(pub u64);

/// rect in logical window pixels, top left origin
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

    /// false for nan and for zero, a nan frame is undefined for appkit
    pub fn has_area(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}

/// one native material region, always background since the view is below the metal layer
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GlassRegion {
    pub id: RegionId,
    pub material: Material,
    pub bounds: RegionRect,
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

/// lets a batch in only from a newer layout pass, render passes have no new generation
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

/// all native material regions of one layout pass, handed whole not as a diff
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

    /// adds a region, zero area or duplicate id is a caller bug so both error
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

/// the window native material regions, holds appkit views so main thread only
pub trait GlassSurface {
    /// what this surface really is, ask once since it cant change while running
    fn backing(&self) -> MaterialBacking;

    /// hands over every region for one layout pass, err means the layout pass contract broke
    fn commit(&mut self, layout: &GlassLayout) -> Result<PlatformSupport, GlassError>;

    /// removes every region, a view left behind is invisible but still allocated
    fn clear(&mut self) -> PlatformSupport;
}

/// records what it was asked to host instead of hosting it
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

pub fn current_backing(material: Material) -> MaterialBacking {
    resolve(material, HostOs::current(), macos_version())
}

/// the surface every host without a region material gets
pub fn painted_glass_surface() -> Box<dyn GlassSurface> {
    Box::new(InMemoryGlassSurface::new(MaterialBacking::Painted))
}

pub fn current_glass_surface() -> Box<dyn GlassSurface> {
    super::os::current_glass_surface()
}

/// macos release this process runs on
/// macos says 10.16 to a binary built with an older sdk
pub fn macos_version() -> Option<MacOsVersion> {
    static CACHED: std::sync::OnceLock<Option<MacOsVersion>> = std::sync::OnceLock::new();
    *CACHED.get_or_init(super::os::read_macos_version)
}

/// windows build this process runs on, none on mac and linux or if the read failed
pub fn windows_build() -> Option<WindowsBuild> {
    static CACHED: std::sync::OnceLock<Option<WindowsBuild>> = std::sync::OnceLock::new();
    *CACHED.get_or_init(super::os::read_windows_build)
}

/// sets the tier 0 window material, unsupported means it was already downgraded to opaque
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

    #[test]
    fn the_shell_asks_for_the_window_material_its_corner_needs() {
        assert_eq!(
            preferred_window_material(HostOs::MacOs),
            WindowMaterial::Transparent,
            "an opaque window is a square one: AppKit has no clear pixels to \
             round it with"
        );

        for build in [None, Some(WindowsBuild(19_045)), Some(WindowsBuild(22_000))] {
            assert_eq!(
                preferred_window_material_on(HostOs::Linux, build),
                WindowMaterial::Opaque,
                "Linux does not round the window out of the surface's alpha, \
                 and the app paints every pixel of its window"
            );
            assert_eq!(
                preferred_window_material_on(HostOs::Windows, build),
                WindowMaterial::Opaque,
                "below build 22621 gpui skips the backdrop and the window would be see-through"
            );
        }
    }

    #[test]
    fn windows_11_gets_mica_from_the_system_backdrop_build() {
        for build in [WindowsBuild::SYSTEM_BACKDROP, WindowsBuild(26_100)] {
            assert_eq!(
                preferred_window_material_on(HostOs::Windows, Some(build)),
                WindowMaterial::Mica
            );
        }
        assert_eq!(
            preferred_window_material_on(HostOs::Windows, Some(WindowsBuild(22_620))),
            WindowMaterial::Opaque
        );
    }

    #[test]
    fn mica_is_never_granted_off_windows() {
        for host in [HostOs::MacOs, HostOs::Linux] {
            assert!(!mica_available(host, Some(WindowsBuild(26_100))));
            assert_eq!(
                granted_window_material_on(host, Some(WindowsBuild(26_100)), WindowMaterial::Mica),
                WindowMaterial::Opaque
            );
        }
        assert!(!mica_available(HostOs::Windows, None));
    }

    #[test]
    fn the_preferred_window_material_is_never_blurred() {
        for host in HostOs::ALL {
            for build in [None, Some(WindowsBuild(19_045)), Some(WindowsBuild::SYSTEM_BACKDROP)] {
                assert_ne!(
                    preferred_window_material_on(host, build),
                    WindowMaterial::Blurred
                );
            }
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
        assert_eq!(
            WindowMaterial::Mica.into_gpui(),
            gpui::WindowBackgroundAppearance::MicaBackdrop
        );
    }

    #[test]
    fn downgrading_the_window_material_is_a_fixed_point() {
        for host in HostOs::ALL {
            for build in [None, Some(WindowsBuild(19_045)), Some(WindowsBuild::SYSTEM_BACKDROP)] {
                for requested in WindowMaterial::ALL {
                    let granted = granted_window_material_on(host, build, requested);
                    assert_eq!(
                        granted_window_material_on(host, build, granted),
                        granted,
                        "re-applying what the window already is must not move it again"
                    );
                }
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
        assert_eq!(macos_version().is_some(), HostOs::current() == HostOs::MacOs);
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

        if HostOs::current() != HostOs::MacOs {
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
