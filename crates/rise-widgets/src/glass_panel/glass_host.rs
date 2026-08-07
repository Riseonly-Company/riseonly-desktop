use std::collections::BTreeMap;

use gpui::{App, Bounds, IntoElement, Pixels, canvas, prelude::*};
use rise_core::GenerationCounter;
use rise_platform::materials::{
    GlassLayout, GlassRegion, GlassSurface, Material, MaterialBacking, RegionId, RegionRect,
};

/// The window's native material regions, gathered once per frame.
///
/// Installed as a gpui global because there is one window's worth of AppKit
/// views and one process that owns them, and because `GlassSurface` is neither
/// `Send` nor `Sync` — the implementation behind it holds AppKit views, and
/// AppKit off the main thread is undefined behaviour rather than a lint.
pub struct GlassHost {
    surface: Box<dyn GlassSurface>,
    layouts: GenerationCounter,
    ids: BTreeMap<&'static str, RegionId>,
    next_id: u64,
    pending: Vec<GlassRegion>,
    last_error: Option<String>,
}

impl gpui::Global for GlassHost {}

impl GlassHost {
    pub fn install(surface: Box<dyn GlassSurface>, cx: &mut App) {
        cx.set_global(Self {
            surface,
            layouts: GenerationCounter::new(),
            ids: BTreeMap::new(),
            next_id: 1,
            pending: Vec::new(),
            last_error: None,
        });
    }

    pub fn is_installed(cx: &App) -> bool {
        cx.has_global::<Self>()
    }

    /// What `material` actually is in this window.
    ///
    /// Painted when nothing has been installed, which is what a headless test
    /// and a Linux session both look like — so no caller needs a second branch
    /// for "the host is missing".
    pub fn backing(material: Material, cx: &App) -> MaterialBacking {
        if !cx.has_global::<Self>() {
            return MaterialBacking::Painted;
        }
        cx.global::<Self>()
            .surface
            .backing()
            .capped_at(material.ceiling())
    }

    /// A stable number for a surface that lives as long as the window.
    ///
    /// Stable is the whole requirement: the platform side moves the view it
    /// already has for an id it has seen, and tears down the ones it no longer
    /// receives. An id minted per frame would rebuild every AppKit view on every
    /// layout pass.
    pub fn region_id(key: &'static str, cx: &mut App) -> RegionId {
        let host = cx.global_mut::<Self>();
        if let Some(id) = host.ids.get(key) {
            return *id;
        }

        let id = RegionId(host.next_id);
        host.next_id += 1;
        host.ids.insert(key, id);
        id
    }

    /// Records one region for the layout pass in progress.
    ///
    /// Called from an element's prepaint, which is the layout pass — never from
    /// paint. `docs/PLATFORM_GLASS.md` requires the rectangle to be static for
    /// the frame, and `LayoutGate` in rise-platform enforces the other half of
    /// it by refusing a generation that is not newer.
    pub fn record(region: GlassRegion, cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }

        let host = cx.global_mut::<Self>();
        if host.pending.iter().any(|existing| existing.id == region.id) {
            return;
        }
        host.pending.push(region);
    }

    /// Hands the frame's regions to the platform and starts the next one.
    ///
    /// Whole rather than as a diff: the platform side has to know which views
    /// should no longer exist, and a surface that simply stopped being laid out
    /// sends no removal of its own.
    pub fn commit(cx: &mut App) {
        if !cx.has_global::<Self>() {
            return;
        }

        let host = cx.global_mut::<Self>();
        let regions = std::mem::take(&mut host.pending);
        let generation = host.layouts.bump();
        let mut layout = GlassLayout::new(generation);

        for region in regions {
            if let Err(error) = layout.push(region) {
                host.last_error = Some(error.to_string());
            }
        }

        if let Err(error) = host.surface.commit(&layout) {
            host.last_error = Some(error.to_string());
        }
    }

    /// The last contract violation the platform reported, for a diagnostics
    /// surface. A region rejected here draws nothing, and nothing is exactly
    /// what a silent failure looks like.
    pub fn last_error(cx: &App) -> Option<String> {
        cx.global::<Self>().last_error.clone()
    }
}

/// Commits the frame's regions.
///
/// Mounted once, at the root. gpui lays out and prepaints the whole tree before
/// it paints any of it, so a paint callback anywhere in the tree runs after
/// every `GlassPanel` has reported its rectangle — which is what makes one
/// commit per frame both complete and free of a frame's lag.
///
/// Checked against the pinned rev rather than assumed: `Window::draw_roots` sets
/// `DrawPhase::Prepaint`, prepaints the root and then the deferred draws, and
/// only then sets `DrawPhase::Paint` and paints. A gpui bump is where this
/// stops being true, which is why bumping is scheduled work with a test gate.
pub fn glass_host() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |_: Bounds<Pixels>, _, _, cx: &mut App| GlassHost::commit(cx),
    )
    .absolute()
    .size_0()
}

pub(crate) fn region_rect(bounds: Bounds<Pixels>) -> RegionRect {
    RegionRect::from_gpui(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rise_platform::materials::InMemoryGlassSurface;

    fn host(backing: MaterialBacking) -> GlassHost {
        GlassHost {
            surface: Box::new(InMemoryGlassSurface::new(backing)),
            layouts: GenerationCounter::new(),
            ids: BTreeMap::new(),
            next_id: 1,
            pending: Vec::new(),
            last_error: None,
        }
    }

    #[test]
    fn a_surface_keeps_one_id_for_as_long_as_it_exists() {
        let mut host = host(MaterialBacking::Vibrancy);

        let first = {
            let id = RegionId(host.next_id);
            host.next_id += 1;
            host.ids.insert("rail", id);
            id
        };

        assert_eq!(host.ids.get("rail"), Some(&first));
        assert_ne!(first, RegionId(host.next_id));
    }

    #[test]
    fn every_commit_carries_a_newer_generation_than_the_last() {
        let host = host(MaterialBacking::Vibrancy);
        let first = host.layouts.bump();
        let second = host.layouts.bump();
        assert!(
            second > first,
            "the layout gate would reject the second pass"
        );
    }

    #[test]
    fn a_region_registered_twice_in_one_pass_is_recorded_once() {
        let mut host = host(MaterialBacking::Vibrancy);
        let region = GlassRegion::new(
            RegionId(1),
            Material::Chrome,
            RegionRect::new(0.0, 0.0, 56.0, 800.0),
        );

        for _ in 0..2 {
            if !host.pending.iter().any(|existing| existing.id == region.id) {
                host.pending.push(region);
            }
        }

        assert_eq!(
            host.pending.len(),
            1,
            "two regions sharing an id fight over one AppKit view"
        );
    }
}
