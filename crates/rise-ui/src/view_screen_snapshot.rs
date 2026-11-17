use gpui::{
    AnyElement, AnyView, Bounds, IntoElement, ParentElement, Pixels, Size, Styled, Window, div,
    point, px,
};

use crate::view_anchor_probe::ViewAnchorProbe;

pub enum ViewScreenSnapshotPolicy {}

impl ViewScreenSnapshotPolicy {
    pub const WAITS_FOR_SCREEN_UPDATES: bool = false;

    pub const MAXIMUM_FULL_SOURCE_SCALE: f32 = 2.0;
    pub const MAXIMUM_FULL_SOURCE_PIXEL_COUNT: f32 = 4_000_000.0;
    pub const MAXIMUM_FULL_SOURCE_PIXEL_DIMENSION: f32 = 16_000.0;
    pub const TARGET_OVERFLOW_PREVIEW_RASTER_HEIGHT: f32 = 2_048.0;
    pub const OVERFLOW_PREVIEW_POINT_HEIGHT: f32 = 1_200.0;
    pub const MINIMUM_FULL_SOURCE_SCALE: f32 = 0.75;

    pub fn full_source_raster_scale(size: Size<Pixels>, display_scale: f32) -> Option<f32> {
        let width = f32::from(size.width);
        let height = f32::from(size.height);
        if !(width > 1.0 && height > 1.0 && width.is_finite() && height.is_finite())
            || !display_scale.is_finite()
            || display_scale <= 0.0
        {
            return None;
        }

        let point_area = width * height;
        if point_area <= 0.0 {
            return None;
        }
        let pixel_budget_scale = (Self::MAXIMUM_FULL_SOURCE_PIXEL_COUNT / point_area).sqrt();
        let dimension_scale = Self::MAXIMUM_FULL_SOURCE_PIXEL_DIMENSION / width.max(height);
        let overflow_scale = if height > Self::OVERFLOW_PREVIEW_POINT_HEIGHT {
            Self::MINIMUM_FULL_SOURCE_SCALE.max(Self::TARGET_OVERFLOW_PREVIEW_RASTER_HEIGHT / height)
        } else {
            Self::MAXIMUM_FULL_SOURCE_SCALE
        };
        let scale = display_scale
            .min(Self::MAXIMUM_FULL_SOURCE_SCALE)
            .min(pixel_budget_scale)
            .min(dimension_scale)
            .min(overflow_scale);
        (scale >= Self::MINIMUM_FULL_SOURCE_SCALE).then_some(scale)
    }
}

/// mounted subtree that can be drawn again, its view and the probe on its root
#[derive(Clone)]
pub struct ViewScreenSource {
    pub view: AnyView,
    pub probe: ViewAnchorProbe,
}

impl ViewScreenSource {
    pub fn new(view: impl Into<AnyView>, probe: ViewAnchorProbe) -> Self {
        Self { view: view.into(), probe }
    }

    /// where the source was painted last, in window coords
    pub fn window_frame(&self) -> Option<Bounds<Pixels>> {
        self.probe.window_frame()
    }
}

/// chat row containers opt in as the exact unclipped render root
/// so the shared list item host isnt drawn with every mounted sibling
pub trait HoldContextMenuSnapshotRenderRoot {
    fn hold_context_menu_live_source_view(&self) -> Option<ViewScreenSource>;
}

#[derive(Clone)]
pub struct HoldContextMenuLiveSourceCandidate {
    pub source_view: ViewScreenSource,
    pub crop_rect_in_source: Bounds<Pixels>,
}

/// one device pixel at display_scale, slack for every containment check
fn pixel_tolerance(display_scale: f32) -> f32 {
    1.0 / 1f32.max(if display_scale.is_finite() { display_scale } else { 1.0 })
}

/// true if rect is inside bounds grown by tolerance
fn contains_with_tolerance(bounds: Bounds<Pixels>, rect: Bounds<Pixels>, tolerance: f32) -> bool {
    rect.is_contained_within(&bounds.dilate(px(tolerance)))
}

pub enum HoldContextMenuLiveSourceLiftPolicy {}

impl HoldContextMenuLiveSourceLiftPolicy {
    /// resolves only an explicitly marked and already mounted row subtree
    /// gpui has no superviews so the caller passes the render roots around the trigger, innermost first
    pub fn candidate(
        render_roots: &[&dyn HoldContextMenuSnapshotRenderRoot],
        source_rect_in_coordinate_space: Bounds<Pixels>,
        display_scale: f32,
    ) -> Option<HoldContextMenuLiveSourceCandidate> {
        let source_rect = source_rect_in_coordinate_space;
        if f32::from(source_rect.size.width) <= 1.0 || f32::from(source_rect.size.height) <= 1.0 {
            return None;
        }
        let tolerance = pixel_tolerance(display_scale);
        render_roots.iter().find_map(|marker| {
            let live_source = marker.hold_context_menu_live_source_view()?;
            let frame = live_source.window_frame()?;
            if !contains_with_tolerance(frame, source_rect, tolerance) {
                return None;
            }
            let crop = Bounds::new(source_rect.origin - frame.origin, source_rect.size);
            Some(HoldContextMenuLiveSourceCandidate { source_view: live_source, crop_rect_in_source: crop })
        })
    }
}

/// captured picture, source drawn again and clipped to crop in source coords
#[derive(Clone)]
pub struct ViewScreenImage {
    pub source: AnyView,
    pub source_size: Size<Pixels>,
    pub crop: Bounds<Pixels>,
    pub raster_scale: f32,
}

impl ViewScreenImage {
    pub fn size(&self) -> Size<Pixels> {
        self.crop.size
    }

    /// picture as an element of the crop size
    /// source is drawn at its own size so text wraps like in the list
    pub fn element(&self) -> AnyElement {
        div()
            .relative()
            .flex_none()
            .w(self.crop.size.width)
            .h(self.crop.size.height)
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left(-self.crop.origin.x)
                    .top(-self.crop.origin.y)
                    .w(self.source_size.width)
                    .h(self.source_size.height)
                    .child(self.source.clone()),
            )
            .into_any_element()
    }
}

#[derive(Clone)]
pub struct ViewScreenCapture {
    pub image: ViewScreenImage,
    pub screen_rect: Bounds<Pixels>,
}

pub enum ViewScreenSnapshot {}

impl ViewScreenSnapshot {
    /// part of source_view inside the region and the window, as on screen now
    /// none when either is under a point wide or the source isnt drawn
    pub fn capture(
        region_in_screen: Bounds<Pixels>,
        window: &Window,
        source_view: Option<&ViewScreenSource>,
    ) -> Option<ViewScreenCapture> {
        let rect_in_window = region_in_screen;
        if f32::from(rect_in_window.size.width) <= 1.0 || f32::from(rect_in_window.size.height) <= 1.0 {
            return None;
        }
        let window_bounds = Bounds::new(point(px(0.0), px(0.0)), window.viewport_size());
        let clipped_window = rect_in_window.intersect(&window_bounds);
        if !Self::is_capturable(&clipped_window) {
            return None;
        }

        let source = source_view?;
        let source_frame = source.window_frame()?;
        let clipped = clipped_window.intersect(&source_frame);
        if !Self::is_capturable(&clipped) {
            return None;
        }
        let image = Self::snapshot_region(source, source_frame, clipped, window.scale_factor());
        Some(ViewScreenCapture { image, screen_rect: clipped })
    }

    pub fn capture_plate(plate: &ViewScreenSource, window: &Window) -> Option<ViewScreenImage> {
        let screen = plate.window_frame()?;
        Self::capture(screen, window, Some(plate)).map(|capture| capture.image)
    }

    pub fn capture_full_source(
        region_in_screen: Bounds<Pixels>,
        window: &Window,
        source_view: &ViewScreenSource,
        render_source_view_directly: bool,
    ) -> Option<ViewScreenCapture> {
        let _ = render_source_view_directly;
        if f32::from(region_in_screen.size.width) <= 1.0 || f32::from(region_in_screen.size.height) <= 1.0 {
            return None;
        }
        let source_frame = source_view.window_frame()?;
        let display_scale = window.scale_factor();
        if !contains_with_tolerance(source_frame, region_in_screen, pixel_tolerance(display_scale)) {
            return None;
        }
        let raster_scale = ViewScreenSnapshotPolicy::full_source_raster_scale(region_in_screen.size, display_scale)?;
        let image = Self::snapshot_region(source_view, source_frame, region_in_screen, raster_scale);
        Some(ViewScreenCapture { image, screen_rect: region_in_screen })
    }

    fn is_capturable(rect: &Bounds<Pixels>) -> bool {
        f32::from(rect.size.width) > 1.0 && f32::from(rect.size.height) > 1.0
    }

    fn snapshot_region(
        source: &ViewScreenSource,
        source_frame: Bounds<Pixels>,
        rect_in_window: Bounds<Pixels>,
        raster_scale: f32,
    ) -> ViewScreenImage {
        ViewScreenImage {
            source: source.view.clone(),
            source_size: source_frame.size,
            crop: Bounds::new(rect_in_window.origin - source_frame.origin, rect_in_window.size),
            raster_scale,
        }
    }
}
