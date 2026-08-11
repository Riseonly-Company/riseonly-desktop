use gpui::{Bounds, Pixels, Point, Size, point, px};

/// What an overlay is positioned against.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OverlayAnchor {
    /// A pointer position, as a right-click gives it.
    Point(Point<Pixels>),
    /// The rectangle of whatever opened the overlay.
    Rect(Bounds<Pixels>),
}

/// Which side of a [`OverlayAnchor::Rect`] the overlay would rather open on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum OverlaySide {
    #[default]
    Below,
    Above,
    Right,
    Left,
}

impl OverlaySide {
    pub fn opposite(self) -> Self {
        match self {
            Self::Below => Self::Above,
            Self::Above => Self::Below,
            Self::Right => Self::Left,
            Self::Left => Self::Right,
        }
    }

    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Below | Self::Above)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OverlayPlacement {
    pub origin: Point<Pixels>,
    pub side: OverlaySide,
    /// The ideal origin did not fit and was pushed inside the viewport; a tail or
    /// arrow cannot be aligned with the anchor while this is set.
    pub clamped: bool,
}

/// Where an overlay of `size` goes so that it never leaves `viewport`. Total:
/// any geometry — oversized, negative, infinite, NaN — yields an origin inside
/// the viewport inset by `margin`, never negative and never a panic.
pub fn place(
    anchor: OverlayAnchor,
    size: Size<Pixels>,
    viewport: Size<Pixels>,
    gap: Pixels,
    margin: Pixels,
    preferred: OverlaySide,
) -> OverlayPlacement {
    let viewport_width = positive(viewport.width);
    let viewport_height = positive(viewport.height);
    let width = positive(size.width);
    let height = positive(size.height);
    let margin = positive(margin);

    let frame = Frame {
        width,
        height,
        viewport_width,
        viewport_height,
        gap: positive(gap),
        margin_x: effective_margin(margin, width, viewport_width),
        margin_y: effective_margin(margin, height, viewport_height),
    };

    let (ideal, side) = match anchor {
        OverlayAnchor::Rect(rect) => frame.against_rect(rect, preferred),
        OverlayAnchor::Point(at) => frame.against_point(at, preferred),
    };

    let (x, clamped_x) = clamp_axis(ideal.0, frame.width, viewport_width, frame.margin_x);
    let (y, clamped_y) = clamp_axis(ideal.1, frame.height, viewport_height, frame.margin_y);

    OverlayPlacement {
        origin: point(px(x), px(y)),
        side,
        clamped: clamped_x || clamped_y,
    }
}

#[derive(Clone, Copy)]
struct Frame {
    width: f32,
    height: f32,
    viewport_width: f32,
    viewport_height: f32,
    gap: f32,
    margin_x: f32,
    margin_y: f32,
}

impl Frame {
    fn against_rect(
        &self,
        rect: Bounds<Pixels>,
        preferred: OverlaySide,
    ) -> ((f32, f32), OverlaySide) {
        let left = finite(f32::from(rect.origin.x));
        let top = finite(f32::from(rect.origin.y));
        let edges = Edges {
            left,
            top,
            right: left + positive(rect.size.width),
            bottom: top + positive(rect.size.height),
        };

        let opposite = preferred.opposite();
        let side = if self.fits(preferred, self.beside(preferred, edges)) {
            preferred
        } else if self.fits(opposite, self.beside(opposite, edges)) {
            opposite
        } else {
            preferred
        };

        (self.beside(side, edges), side)
    }

    fn against_point(
        &self,
        at: Point<Pixels>,
        preferred: OverlaySide,
    ) -> ((f32, f32), OverlaySide) {
        let x = finite(f32::from(at.x));
        let y = finite(f32::from(at.y));

        let flipped_x = x + self.width > self.viewport_width - self.margin_x;
        let flipped_y = y + self.height > self.viewport_height - self.margin_y;

        let origin = (
            if flipped_x { x - self.width } else { x },
            if flipped_y { y - self.height } else { y },
        );

        let side = match (preferred.is_vertical(), flipped_x, flipped_y) {
            (true, _, true) => OverlaySide::Above,
            (true, _, false) => OverlaySide::Below,
            (false, true, _) => OverlaySide::Left,
            (false, false, _) => OverlaySide::Right,
        };

        (origin, side)
    }

    fn beside(&self, side: OverlaySide, edges: Edges) -> (f32, f32) {
        match side {
            OverlaySide::Below => (edges.left, edges.bottom + self.gap),
            OverlaySide::Above => (edges.left, edges.top - self.gap - self.height),
            OverlaySide::Right => (edges.right + self.gap, edges.top),
            OverlaySide::Left => (edges.left - self.gap - self.width, edges.top),
        }
    }

    fn fits(&self, side: OverlaySide, origin: (f32, f32)) -> bool {
        match side {
            OverlaySide::Below => origin.1 + self.height <= self.viewport_height - self.margin_y,
            OverlaySide::Above => origin.1 >= self.margin_y,
            OverlaySide::Right => origin.0 + self.width <= self.viewport_width - self.margin_x,
            OverlaySide::Left => origin.0 >= self.margin_x,
        }
    }
}

#[derive(Clone, Copy)]
struct Edges {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

fn clamp_axis(ideal: f32, extent: f32, viewport: f32, margin: f32) -> (f32, bool) {
    let low = margin;
    let high = viewport - margin - extent;
    if high < low {
        return (low, true);
    }

    let ideal = finite(ideal);
    let clamped = ideal.clamp(low, high);
    (clamped, clamped != ideal)
}

/// A margin wider than the room left over satisfies the near edge by pushing the
/// overlay out through the far one, so it is reduced until both fit.
fn effective_margin(requested: f32, extent: f32, viewport: f32) -> f32 {
    let room = viewport - extent;
    if room < 0.0 {
        return requested.min(viewport / 2.0);
    }
    requested.min(room / 2.0)
}

fn positive(value: Pixels) -> f32 {
    finite(f32::from(value)).max(0.0)
}

/// `f32::clamp` panics on a NaN bound, so caller geometry passes through here.
fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{bounds, size};

    const SIDES: [OverlaySide; 4] = [
        OverlaySide::Below,
        OverlaySide::Above,
        OverlaySide::Right,
        OverlaySide::Left,
    ];

    fn window(width: f32, height: f32) -> Size<Pixels> {
        size(px(width), px(height))
    }

    fn menu(width: f32, height: f32) -> Size<Pixels> {
        size(px(width), px(height))
    }

    fn at(x: f32, y: f32) -> OverlayAnchor {
        OverlayAnchor::Point(point(px(x), px(y)))
    }

    fn control(x: f32, y: f32, width: f32, height: f32) -> OverlayAnchor {
        OverlayAnchor::Rect(bounds(point(px(x), px(y)), size(px(width), px(height))))
    }

    fn assert_inside(placement: OverlayPlacement, overlay: Size<Pixels>, viewport: Size<Pixels>) {
        let x = f32::from(placement.origin.x);
        let y = f32::from(placement.origin.y);

        assert!(x >= 0.0, "negative x {x}");
        assert!(y >= 0.0, "negative y {y}");

        let width = f32::from(overlay.width);
        let height = f32::from(overlay.height);
        let viewport_width = f32::from(viewport.width);
        let viewport_height = f32::from(viewport.height);

        if width <= viewport_width {
            assert!(
                x + width <= viewport_width + 1e-3,
                "overflows the right edge: {x} + {width} > {viewport_width}"
            );
        }
        if height <= viewport_height {
            assert!(
                y + height <= viewport_height + 1e-3,
                "overflows the bottom edge: {y} + {height} > {viewport_height}"
            );
        }
    }

    #[test]
    fn nothing_ever_leaves_the_viewport_whatever_the_geometry() {
        let viewports = [window(400.0, 700.0), window(2560.0, 1440.0)];
        let overlays = [menu(1.0, 1.0), menu(220.0, 320.0), menu(900.0, 2000.0)];
        let anchors = [
            at(0.0, 0.0),
            at(-80.0, -80.0),
            at(2559.0, 1439.0),
            at(9000.0, 9000.0),
            control(0.0, 0.0, 10.0, 10.0),
            control(-40.0, -40.0, 10.0, 10.0),
            control(380.0, 680.0, 200.0, 200.0),
            control(2540.0, 1420.0, 40.0, 40.0),
        ];

        for viewport in viewports {
            for overlay in overlays {
                for anchor in anchors {
                    for side in SIDES {
                        for margin in [0.0, 8.0, 400.0] {
                            let placement =
                                place(anchor, overlay, viewport, px(6.0), px(margin), side);
                            assert_inside(placement, overlay, viewport);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn an_overlay_larger_than_the_viewport_pins_to_the_margin_and_says_so() {
        let viewport = window(400.0, 400.0);
        let overlay = menu(900.0, 900.0);

        let placement = place(
            at(200.0, 200.0),
            overlay,
            viewport,
            px(6.0),
            px(12.0),
            OverlaySide::Below,
        );

        assert_eq!(placement.origin, point(px(12.0), px(12.0)));
        assert!(placement.clamped);
    }

    #[test]
    fn the_preferred_side_is_taken_when_it_fits() {
        let placement = place(
            control(100.0, 100.0, 40.0, 30.0),
            menu(200.0, 200.0),
            window(1280.0, 800.0),
            px(6.0),
            px(8.0),
            OverlaySide::Below,
        );

        assert_eq!(placement.side, OverlaySide::Below);
        assert_eq!(placement.origin, point(px(100.0), px(136.0)));
        assert!(!placement.clamped);
    }

    #[test]
    fn a_side_that_does_not_fit_flips_to_the_one_that_does() {
        let viewport = window(1280.0, 800.0);
        let overlay = menu(200.0, 300.0);

        let flipped_up = place(
            control(100.0, 700.0, 40.0, 30.0),
            overlay,
            viewport,
            px(6.0),
            px(8.0),
            OverlaySide::Below,
        );
        assert_eq!(flipped_up.side, OverlaySide::Above);
        assert_eq!(flipped_up.origin, point(px(100.0), px(394.0)));
        assert!(!flipped_up.clamped);

        let flipped_down = place(
            control(100.0, 40.0, 40.0, 30.0),
            overlay,
            viewport,
            px(6.0),
            px(8.0),
            OverlaySide::Above,
        );
        assert_eq!(flipped_down.side, OverlaySide::Below);
        assert_eq!(flipped_down.origin, point(px(100.0), px(76.0)));

        let flipped_left = place(
            control(1200.0, 100.0, 40.0, 30.0),
            overlay,
            viewport,
            px(6.0),
            px(8.0),
            OverlaySide::Right,
        );
        assert_eq!(flipped_left.side, OverlaySide::Left);
        assert_eq!(flipped_left.origin, point(px(994.0), px(100.0)));

        let flipped_right = place(
            control(20.0, 100.0, 40.0, 30.0),
            overlay,
            viewport,
            px(6.0),
            px(8.0),
            OverlaySide::Left,
        );
        assert_eq!(flipped_right.side, OverlaySide::Right);
        assert_eq!(flipped_right.origin, point(px(66.0), px(100.0)));
    }

    #[test]
    fn when_neither_side_fits_the_preferred_one_is_kept_and_clamped() {
        let viewport = window(400.0, 300.0);
        let overlay = menu(200.0, 260.0);

        let placement = place(
            control(100.0, 140.0, 40.0, 30.0),
            overlay,
            viewport,
            px(6.0),
            px(8.0),
            OverlaySide::Below,
        );

        assert_eq!(placement.side, OverlaySide::Below);
        assert!(placement.clamped);
        assert_inside(placement, overlay, viewport);
    }

    #[test]
    fn a_pointer_anchor_opens_with_its_top_left_on_the_pointer() {
        let placement = place(
            at(300.0, 200.0),
            menu(220.0, 180.0),
            window(1280.0, 800.0),
            px(6.0),
            px(8.0),
            OverlaySide::Below,
        );

        assert_eq!(placement.origin, point(px(300.0), px(200.0)));
        assert_eq!(placement.side, OverlaySide::Below);
        assert!(!placement.clamped);
    }

    #[test]
    fn a_pointer_anchor_flips_left_and_up_at_the_far_corner() {
        let overlay = menu(220.0, 180.0);
        let viewport = window(1280.0, 800.0);

        let flipped_left = place(
            at(1200.0, 200.0),
            overlay,
            viewport,
            px(6.0),
            px(8.0),
            OverlaySide::Below,
        );
        assert_eq!(flipped_left.origin, point(px(980.0), px(200.0)));
        assert!(!flipped_left.clamped);

        let flipped_up = place(
            at(300.0, 780.0),
            overlay,
            viewport,
            px(6.0),
            px(8.0),
            OverlaySide::Below,
        );
        assert_eq!(flipped_up.origin, point(px(300.0), px(600.0)));
        assert_eq!(flipped_up.side, OverlaySide::Above);

        let both = place(
            at(1270.0, 795.0),
            overlay,
            viewport,
            px(6.0),
            px(8.0),
            OverlaySide::Below,
        );
        assert_eq!(both.origin, point(px(1050.0), px(612.0)));
        assert_eq!(both.side, OverlaySide::Above);
        assert!(both.clamped, "the flip alone did not clear the margin");
    }

    #[test]
    fn a_pointer_anchor_reports_the_horizontal_side_when_asked_horizontally() {
        let overlay = menu(220.0, 180.0);
        let viewport = window(1280.0, 800.0);

        assert_eq!(
            place(
                at(300.0, 200.0),
                overlay,
                viewport,
                px(6.0),
                px(8.0),
                OverlaySide::Right
            )
            .side,
            OverlaySide::Right
        );
        assert_eq!(
            place(
                at(1200.0, 200.0),
                overlay,
                viewport,
                px(6.0),
                px(8.0),
                OverlaySide::Right
            )
            .side,
            OverlaySide::Left
        );
    }

    #[test]
    fn the_cross_axis_is_clamped_rather_than_flipped() {
        let overlay = menu(300.0, 120.0);
        let viewport = window(400.0, 700.0);

        let placement = place(
            control(360.0, 100.0, 30.0, 30.0),
            overlay,
            viewport,
            px(6.0),
            px(8.0),
            OverlaySide::Below,
        );

        assert_eq!(placement.side, OverlaySide::Below);
        assert_eq!(placement.origin, point(px(92.0), px(136.0)));
        assert!(placement.clamped);
    }

    #[test]
    fn the_gap_separates_the_overlay_from_its_anchor() {
        for gap in [0.0, 6.0, 24.0] {
            let placement = place(
                control(100.0, 100.0, 40.0, 30.0),
                menu(200.0, 200.0),
                window(1280.0, 800.0),
                px(gap),
                px(8.0),
                OverlaySide::Below,
            );
            assert_eq!(placement.origin.y, px(130.0 + gap));
        }
    }

    #[test]
    fn a_degenerate_viewport_answers_instead_of_panicking() {
        for viewport in [window(0.0, 0.0), window(-100.0, -100.0), window(1.0, 1.0)] {
            for side in SIDES {
                let placement = place(
                    at(50.0, 50.0),
                    menu(200.0, 200.0),
                    viewport,
                    px(6.0),
                    px(8.0),
                    side,
                );
                assert!(f32::from(placement.origin.x) >= 0.0);
                assert!(f32::from(placement.origin.y) >= 0.0);
                assert!(placement.clamped);
            }
        }
    }

    #[test]
    fn non_finite_geometry_is_answered_rather_than_propagated() {
        let placement = place(
            at(f32::NAN, f32::INFINITY),
            size(px(f32::NAN), px(200.0)),
            window(1280.0, 800.0),
            px(f32::NEG_INFINITY),
            px(8.0),
            OverlaySide::Below,
        );

        assert!(f32::from(placement.origin.x).is_finite());
        assert!(f32::from(placement.origin.y).is_finite());
        assert!(f32::from(placement.origin.x) >= 0.0);
        assert!(f32::from(placement.origin.y) >= 0.0);
    }

    #[test]
    fn a_side_and_its_opposite_are_a_pair() {
        for side in SIDES {
            assert_eq!(side.opposite().opposite(), side);
            assert_ne!(side.opposite(), side);
            assert_eq!(side.is_vertical(), side.opposite().is_vertical());
        }
    }
}
