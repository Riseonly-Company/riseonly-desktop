use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicU32, Ordering};

use gpui::{Bounds, ElementId, IntoElement, Pixels, Size, Window, px};
use rise_ui::swiftui::prelude::measure;

use crate::modules::post::stores::post_actions::post_types::{PostMediaSize, PostMediaSizeAspectRatio};

/// media height in feed and publications and post detail and search
/// frame keeps natural height between a floor and a ceiling tied to window height
/// nothing cropped, too big shows whole over a blurred copy
pub enum PostMediaBox {}

static VIEWPORT_WIDTH: AtomicU32 = AtomicU32::new(440.0_f32.to_bits());
static VIEWPORT_HEIGHT: AtomicU32 = AtomicU32::new(900.0_f32.to_bits());

impl PostMediaBox {
    pub const MAXIMUM_HEIGHT_FRACTION: f32 = 0.60;
    pub const MINIMUM_HEIGHT_FRACTION: f32 = 0.20;

    pub const PLACEHOLDER_ASPECT: f32 = 0.8;

    const ASPECT_EPSILON: f32 = 0.001;

    pub fn maximum_height() -> f32 {
        (f32::from(Self::viewport().height) * Self::MAXIMUM_HEIGHT_FRACTION).max(1.0)
    }

    pub fn minimum_height() -> f32 {
        (f32::from(Self::viewport().height) * Self::MINIMUM_HEIGHT_FRACTION).max(1.0)
    }

    pub fn aspect_range() -> RangeInclusive<f32> {
        let width = f32::from(Self::viewport().width).max(1.0);
        let tallest = width / Self::maximum_height();
        let flattest = width / Self::minimum_height();
        tallest..=tallest.max(flattest)
    }

    pub fn display_aspect(natural: Option<f32>) -> f32 {
        let value = natural.filter(|natural| natural.is_finite() && *natural > 0.0).unwrap_or(Self::PLACEHOLDER_ASPECT);
        let range = Self::aspect_range();
        range.end().min(range.start().max(value))
    }

    pub fn display_aspect_for_width(natural: Option<f32>, width: Option<f32>) -> f32 {
        let Some(width) = width.filter(|width| width.is_finite() && *width > 1.0) else {
            return Self::display_aspect(natural);
        };
        let value = natural.filter(|natural| natural.is_finite() && *natural > 0.0).unwrap_or(Self::PLACEHOLDER_ASPECT);
        let tallest = width / Self::maximum_height();
        let flattest = tallest.max(width / Self::minimum_height());
        flattest.min(tallest.max(value))
    }

    /// false means the box hit the ceiling or floor, so the
    /// frame is fitted inside and the margins are covered by blur
    pub fn fills(natural: Option<f32>, box_aspect: f32) -> bool {
        match natural {
            Some(natural) if natural.is_finite() && natural > 0.0 => (natural - box_aspect).abs() < Self::ASPECT_EPSILON,
            _ => true,
        }
    }

    /// the size reports zeros for not measured, zero must not become a box aspect
    pub fn aspect(size: Option<&PostMediaSize>) -> Option<f32> {
        let value = size?.aspect_ratio()?;
        (value.is_finite() && value > 0.0).then_some(value as f32)
    }

    pub fn viewport() -> Size<Pixels> {
        Size {
            width: px(f32::from_bits(VIEWPORT_WIDTH.load(Ordering::Relaxed))),
            height: px(f32::from_bits(VIEWPORT_HEIGHT.load(Ordering::Relaxed))),
        }
    }

    pub fn set_viewport(viewport: Size<Pixels>) -> bool {
        let width = f32::from(viewport.width);
        let height = f32::from(viewport.height);
        if !(width.is_finite() && height.is_finite() && width > 1.0 && height > 1.0) {
            return false;
        }
        let previous = Self::viewport();
        VIEWPORT_WIDTH.store(width.to_bits(), Ordering::Relaxed);
        VIEWPORT_HEIGHT.store(height.to_bits(), Ordering::Relaxed);
        (f32::from(previous.width) - width).abs() > 0.5 || (f32::from(previous.height) - height).abs() > 0.5
    }

    pub fn viewport_reader(id: impl Into<ElementId>) -> impl IntoElement {
        measure(id, |bounds: Bounds<Pixels>, window: &mut Window, _| {
            VIEWPORT_WIDTH.store(f32::from(bounds.size.width).to_bits(), Ordering::Relaxed);
            VIEWPORT_HEIGHT.store(f32::from(bounds.size.height).to_bits(), Ordering::Relaxed);
            window.refresh();
        })
    }
}
