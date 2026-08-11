use gpui::{Pixels, Size, px};
use rise_theme::ModalMetrics;

/// How wide a modal asks to be. Naming the content, rather than pixels, keeps
/// every modal in the product to one of three widths.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum ModalWidth {
    /// A confirmation: one sentence and two buttons.
    Small,
    #[default]
    /// A form, a short list, an explanation with an action.
    Medium,
    /// A picker: a search field over a list.
    Large,
    /// The escape hatch, for a modal that must match a column it is about.
    Fixed(Pixels),
}

impl ModalWidth {
    fn asked(self, metrics: ModalMetrics) -> Pixels {
        match self {
            Self::Small => metrics.width_small,
            Self::Medium => metrics.width_medium,
            Self::Large => metrics.width_large,
            Self::Fixed(width) => width,
        }
    }
}

/// What a modal actually gets, once the window has had its say.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ModalFrame {
    pub width: Pixels,
    /// A CEILING, not a height. A short modal is as tall as its content; only a
    /// tall one is clipped to this and scrolls inside itself.
    pub max_height: Pixels,
    /// The ask did not survive the window, on either bound.
    pub clamped: bool,
}

/// The smallest a modal may be squeezed to before it stops being readable.
const MINIMUM_WIDTH: f32 = 240.0;
const MINIMUM_HEIGHT: f32 = 120.0;

/// Where a modal fits, given the window it is in. Total: any input — a viewport
/// of zero, NaN or infinity, a `Fixed` wider than the display, a negative margin
/// — yields a positive frame and never panics.
pub fn resolve_frame(
    width: ModalWidth,
    metrics: ModalMetrics,
    viewport: Size<Pixels>,
    overlay_margin: Pixels,
) -> ModalFrame {
    let asked = finite(width.asked(metrics), metrics.width_medium);
    let margin = finite(overlay_margin, px(0.0)).max(px(0.0));

    let available_width = finite(viewport.width, px(0.0)) - margin * 2.0;
    let width = if available_width <= px(MINIMUM_WIDTH) {
        // Genuinely tiny, or not laid out yet: both want the ask, not a sliver.
        asked.min(px(MINIMUM_WIDTH).max(available_width.max(px(0.0))))
    } else {
        asked.min(available_width)
    };
    let width = width.max(px(MINIMUM_WIDTH).min(asked));

    let viewport_height = finite(viewport.height, px(0.0));
    let ceiling = if viewport_height <= px(0.0) {
        metrics.max_height
    } else {
        metrics.height_ceiling(viewport_height)
    };
    let max_height = ceiling.max(px(MINIMUM_HEIGHT));

    ModalFrame {
        width,
        max_height,
        clamped: width < asked || max_height < metrics.max_height,
    }
}

/// NaN propagates through `f32::min`/`max`, so replace it before a clamp sees it.
fn finite(value: Pixels, fallback: Pixels) -> Pixels {
    if f32::from(value).is_finite() {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    fn metrics() -> ModalMetrics {
        ModalMetrics::default()
    }

    fn frame(width: ModalWidth, viewport: Size<Pixels>) -> ModalFrame {
        resolve_frame(width, metrics(), viewport, px(12.0))
    }

    fn desktop() -> Size<Pixels> {
        size(px(1280.0), px(800.0))
    }

    #[test]
    fn a_roomy_window_gives_the_modal_exactly_what_it_asked_for() {
        let resolved = frame(ModalWidth::Medium, desktop());
        assert_eq!(resolved.width, metrics().width_medium);
        assert!(!resolved.clamped);
    }

    #[test]
    fn the_three_ramp_steps_ascend() {
        let small = frame(ModalWidth::Small, desktop()).width;
        let medium = frame(ModalWidth::Medium, desktop()).width;
        let large = frame(ModalWidth::Large, desktop()).width;
        assert!(small < medium && medium < large);
    }

    #[test]
    fn a_fixed_width_is_honoured_when_it_fits() {
        assert_eq!(
            frame(ModalWidth::Fixed(px(400.0)), desktop()).width,
            px(400.0)
        );
    }

    #[test]
    fn a_narrow_window_squeezes_the_modal_and_says_so() {
        let resolved = frame(ModalWidth::Large, size(px(400.0), px(800.0)));
        assert!(resolved.width < metrics().width_large);
        assert!(resolved.width <= px(400.0));
        assert!(resolved.clamped);
    }

    #[test]
    fn the_ceiling_always_leaves_the_window_something() {
        for height in [px(300.0), px(600.0), px(900.0), px(1600.0)] {
            let resolved = frame(ModalWidth::Medium, size(px(1280.0), height));
            assert!(
                resolved.max_height < height || height < px(MINIMUM_HEIGHT),
                "{height:?} left no scrim: {resolved:?}"
            );
        }
    }

    /// gpui reports a zero viewport for at least one frame while a window opens.
    #[test]
    fn a_degenerate_viewport_still_produces_a_drawable_frame() {
        for viewport in [
            size(px(0.0), px(0.0)),
            size(px(-100.0), px(-100.0)),
            size(px(f32::NAN), px(f32::NAN)),
            size(px(f32::INFINITY), px(f32::INFINITY)),
        ] {
            let resolved = frame(ModalWidth::Medium, viewport);
            assert!(
                f32::from(resolved.width) > 0.0,
                "{viewport:?} -> {resolved:?}"
            );
            assert!(
                f32::from(resolved.max_height) > 0.0,
                "{viewport:?} -> {resolved:?}"
            );
        }
    }

    #[test]
    fn a_hostile_margin_cannot_invert_the_width() {
        for margin in [px(-50.0), px(f32::NAN), px(10_000.0)] {
            let resolved = resolve_frame(ModalWidth::Medium, metrics(), desktop(), margin);
            assert!(
                f32::from(resolved.width) > 0.0,
                "{margin:?} -> {resolved:?}"
            );
        }
    }

    #[test]
    fn a_fixed_width_larger_than_the_display_is_brought_back_inside() {
        let resolved = frame(ModalWidth::Fixed(px(4000.0)), desktop());
        assert!(resolved.width <= px(1280.0));
        assert!(resolved.clamped);
    }

    #[test]
    fn the_default_is_the_middle_of_the_ramp() {
        assert_eq!(ModalWidth::default(), ModalWidth::Medium);
    }
}
