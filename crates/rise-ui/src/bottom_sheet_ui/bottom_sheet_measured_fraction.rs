use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, IntoElement, ParentElement, RenderOnce, Window};

use crate::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use crate::swiftui::prelude::*;

type ApplyFraction = Box<dyn Fn(f32, &mut Window, &mut App)>;

pub trait BottomSheetMeasuredFractionModifiers: IntoElement + Sized {
    /// minimum is a floor so the sheet doesnt collapse to a strip before the content is measured
    /// apply gets the fraction and runs only when the height changes
    /// else measure and layout chase each other
    fn measured_sheet_fraction(
        self,
        id: impl Into<ElementId>,
        minimum: f32,
        apply: impl Fn(f32, &mut Window, &mut App) + 'static,
    ) -> MeasuredSheetFraction {
        MeasuredSheetFraction {
            id: id.into(),
            minimum,
            apply: Box::new(apply),
            child: self.into_any_element(),
        }
    }

    /// reports the element height once per change
    /// the content and footer of a sheet sized by BottomSheetMeasuredFraction attached
    fn on_sheet_part_height(
        self,
        id: impl Into<ElementId>,
        apply: impl Fn(f32, &mut Window, &mut App) + 'static,
    ) -> SheetPartHeight {
        SheetPartHeight {
            id: id.into(),
            apply: Rc::new(apply),
            child: self.into_any_element(),
        }
    }
}

impl<E: IntoElement> BottomSheetMeasuredFractionModifiers for E {}

#[derive(IntoElement)]
pub struct MeasuredSheetFraction {
    id: ElementId,
    minimum: f32,
    apply: ApplyFraction,
    child: AnyElement,
}

impl RenderOnce for MeasuredSheetFraction {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let minimum = self.minimum;
        let apply = self.apply;
        div()
            .relative()
            .flex_none()
            .w_full()
            .child(self.child)
            .child(measure(self.id, move |bounds, window, cx| {
                let fraction = BottomSheetMeasuredFraction::value(
                    f32::from(bounds.size.height),
                    minimum,
                    window,
                );
                apply(fraction, window, cx);
            }))
    }
}

#[derive(IntoElement)]
pub struct SheetPartHeight {
    id: ElementId,
    apply: Rc<dyn Fn(f32, &mut Window, &mut App)>,
    child: AnyElement,
}

impl RenderOnce for SheetPartHeight {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let last = window.use_keyed_state((self.id.clone(), "last-height"), cx, |_, _| None::<f32>);
        let apply = self.apply;
        div()
            .relative()
            .w_full()
            .child(self.child)
            .child(measure(self.id, move |bounds, window, cx| {
                let height = f32::from(bounds.size.height);
                if last.read(cx).is_some_and(|last| last == height) {
                    return;
                }
                last.update(cx, |last, _| *last = Some(height));
                apply(height, window, cx);
            }))
    }
}

/// namespace of the fraction rule
pub enum BottomSheetMeasuredFraction {}

impl BottomSheetMeasuredFraction {
    const CHROME_HEIGHT: f32 = 34.0;

    const MAXIMUM_FRACTION: f32 = 0.86;

    /// fraction of the sheet window, the desktop screen
    pub fn value(content_height: f32, minimum: f32, window: &Window) -> f32 {
        Self::value_in(content_height, minimum, Self::available(window))
    }

    /// sheet with footer attached under the content, as tall as handle plus content plus gap plus footer
    /// none until the content is measured so the first frame keeps the caller detent
    /// content past the ceiling scrolls
    pub fn attached(content_height: f32, footer_height: f32, minimum: f32, window: &Window) -> Option<f32> {
        if content_height <= 0.0 {
            return None;
        }
        Some(Self::fraction(
            Self::attached_chrome(footer_height) + content_height,
            minimum,
            Self::available(window),
        ))
    }

    pub fn attached_content_ceiling(footer_height: f32, window: &Window) -> f32 {
        (Self::available(window) * Self::MAXIMUM_FRACTION - Self::attached_chrome(footer_height)).max(1.0)
    }

    pub fn value_in(content_height: f32, minimum: f32, available_height: f32) -> f32 {
        Self::fraction(content_height.max(0.0) + Self::CHROME_HEIGHT, minimum, available_height)
    }

    fn attached_chrome(footer_height: f32) -> f32 {
        let handle = BottomSheetMetrics::HANDLE_TOP_PADDING
            + BottomSheetMetrics::HANDLE_HEIGHT
            + BottomSheetMetrics::HANDLE_BOTTOM_PADDING;
        handle + BottomSheetMetrics::ATTACHED_FOOTER_TOP_PADDING + footer_height.max(0.0)
    }

    fn fraction(sheet_height: f32, minimum: f32, available_height: f32) -> f32 {
        let available = available_height.max(1.0);
        Self::MAXIMUM_FRACTION.min(minimum.max(sheet_height / available))
    }

    fn available(window: &Window) -> f32 {
        f32::from(window.viewport_size().height).max(1.0)
    }
}

/// horizontal inset the shell already applied to the sheet content
/// edge gestures need it since their hit zone is local but the content origin is shifted
/// must be exactly what the shell applied
pub struct BottomSheetHorizontalSafeInsetKey;

impl EnvKey for BottomSheetHorizontalSafeInsetKey {
    type Value = f32;

    fn default_value() -> Self::Value {
        0.0
    }
}

pub fn bottom_sheet_horizontal_safe_inset(cx: &App) -> f32 {
    env::<BottomSheetHorizontalSafeInsetKey>(cx)
}
