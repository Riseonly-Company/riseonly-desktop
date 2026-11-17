use std::collections::HashMap;

use gpui::{Bounds, Edges, Pixels, Point, point, px, size};
use rise_canon::RiseGuidePlacement;

use crate::swiftui::stacks::Alignment;

/// numbers the placement solver needs, passed as an argument so the solver stays pure
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideOverlayLayoutMetrics {
    pub screen_margin: Pixels,
    pub cutout_clearance: Pixels,
    pub group_spacing: Pixels,
    pub minimum_vertical_band: Pixels,
    pub minimum_horizontal_band: Pixels,
    pub minimum_surface_width: Pixels,
}

/// which side of the cutout the narrator and surface stand on
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuideOverlaySide {
    Above,
    Below,
    Leading,
    Trailing,
    Centred,
}

impl GuideOverlaySide {
    pub fn group_alignment(self) -> Alignment {
        match self {
            Self::Above => Alignment::Bottom,
            Self::Below => Alignment::Top,
            Self::Leading => Alignment::Trailing,
            Self::Trailing => Alignment::Leading,
            Self::Centred => Alignment::Center,
        }
    }

    pub fn is_horizontal(self) -> bool {
        match self {
            Self::Leading | Self::Trailing => true,
            Self::Above | Self::Below | Self::Centred => false,
        }
    }
}

/// band the narrator and surface are laid out in plus the width the surface may use, window coordinates
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideOverlayFrames {
    pub band: Bounds<Pixels>,
    pub side: GuideOverlaySide,
    pub surface_width: Pixels,
    pub is_cramped: bool,
}

impl GuideOverlayFrames {
    pub fn new(band: Bounds<Pixels>, side: GuideOverlaySide, surface_width: Pixels) -> Self {
        Self {
            band,
            side,
            surface_width,
            is_cramped: false,
        }
    }

    pub fn group_alignment(&self) -> Alignment {
        if !self.is_cramped {
            return self.side.group_alignment();
        }
        if self.side == GuideOverlaySide::Below {
            Alignment::Bottom
        } else {
            Alignment::Top
        }
    }
}

/// everything the layout solve reads
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideOverlaySolveInput {
    pub container: Bounds<Pixels>,
    pub safe_area: Edges<Pixels>,
    pub keyboard_height: Pixels,
    pub cutout: Option<Bounds<Pixels>>,
    pub origin: Option<Bounds<Pixels>>,
    pub keep_clear: Option<Bounds<Pixels>>,
    pub placement: RiseGuidePlacement,
    pub prefers_centred: bool,
    pub narrator_side: Pixels,
    pub surface_width_limit: Pixels,
    pub is_right_to_left: bool,
    pub metrics: GuideOverlayLayoutMetrics,
}

/// where the narrator and surface of a step stand
/// pure geometry, no window or store reads
/// result is a band because group height depends on how much text the step has
pub enum GuideOverlayLayout {}

type Bands = HashMap<GuideOverlaySide, Bounds<Pixels>>;

const UNTARGETED_CENTRE_COLUMN: f32 = 500.0;

fn mid_x(rect: &Bounds<Pixels>) -> Pixels {
    rect.origin.x + rect.size.width * 0.5
}

fn mid_y(rect: &Bounds<Pixels>) -> Pixels {
    rect.origin.y + rect.size.height * 0.5
}

fn rect(x: Pixels, y: Pixels, width: Pixels, height: Pixels) -> Bounds<Pixels> {
    Bounds::new(point(x, y), size(width, height))
}

impl GuideOverlayLayout {
    /// rectangle a guide may draw chrome in
    pub fn content_region(
        container: Bounds<Pixels>,
        safe_area: Edges<Pixels>,
        keyboard_height: Pixels,
        metrics: GuideOverlayLayoutMetrics,
    ) -> Bounds<Pixels> {
        let margin = metrics.screen_margin;
        let bottom = safe_area.bottom.max(keyboard_height);
        let width = container.size.width - safe_area.left - safe_area.right - margin * 2.0;
        let height = container.size.height - safe_area.top - bottom - margin * 2.0;
        rect(
            container.left() + safe_area.left + margin,
            container.top() + safe_area.top + margin,
            width.max(px(0.0)),
            height.max(px(0.0)),
        )
    }

    pub fn solve(input: GuideOverlaySolveInput) -> GuideOverlayFrames {
        let metrics = input.metrics;
        let region = Self::content_region(input.container, input.safe_area, input.keyboard_height, metrics);

        let cutout = match input.cutout {
            Some(cutout)
                if !input.prefers_centred && region.size.width > px(0.0) && region.size.height > px(0.0) =>
            {
                cutout
            }
            None if !input.prefers_centred && region.size.width > px(0.0) && region.size.height > px(0.0) => {
                if let Some(frames) = Self::beside_centre_column(&input, region) {
                    return frames;
                }
                return GuideOverlayFrames::new(
                    region,
                    GuideOverlaySide::Centred,
                    input
                        .surface_width_limit
                        .min(metrics.minimum_surface_width)
                        .max(input.surface_width_limit.min(region.size.width)),
                );
            }
            _ => {
                return GuideOverlayFrames::new(
                    region,
                    GuideOverlaySide::Centred,
                    input
                        .surface_width_limit
                        .min(metrics.minimum_surface_width)
                        .max(input.surface_width_limit.min(region.size.width)),
                );
            }
        };

        let blocked = input
            .keep_clear
            .map_or(cutout, |keep_clear| cutout.union(&keep_clear))
            .dilate(metrics.cutout_clearance);
        let bands = Self::bands(
            region,
            blocked,
            point(mid_x(&cutout), mid_y(&cutout)),
            input.is_right_to_left,
        );
        let side = Self::choose_side(input.placement, cutout, input.origin, &bands, metrics);
        let is_cramped = !side.is_horizontal() && !Self::fits(side, &bands, metrics);
        let band = if is_cramped {
            region
        } else {
            bands.get(&side).copied().unwrap_or(region)
        };

        GuideOverlayFrames {
            band,
            side,
            surface_width: Self::surface_width(band, input.narrator_side, input.surface_width_limit, metrics),
            is_cramped,
        }
    }

    fn beside_centre_column(input: &GuideOverlaySolveInput, region: Bounds<Pixels>) -> Option<GuideOverlayFrames> {
        let metrics = input.metrics;
        let column_width = px(UNTARGETED_CENTRE_COLUMN).min(region.size.width);
        let column = rect(
            region.left() + (region.size.width - column_width) * 0.5,
            region.top(),
            column_width,
            region.size.height,
        )
        .dilate(metrics.cutout_clearance);
        let bands = Self::bands(region, column, point(mid_x(&region), mid_y(&region)), input.is_right_to_left);
        let side = GuideOverlaySide::Trailing;
        if !Self::fits(side, &bands, metrics) {
            return None;
        }
        let band = bands.get(&side).copied()?;
        Some(GuideOverlayFrames::new(
            band,
            side,
            Self::surface_width(band, input.narrator_side, input.surface_width_limit, metrics),
        ))
    }

    fn bands(
        region: Bounds<Pixels>,
        blocked: Bounds<Pixels>,
        anchor_centre: Point<Pixels>,
        is_right_to_left: bool,
    ) -> Bands {
        let zero = px(0.0);
        let above = rect(
            region.left(),
            region.top(),
            region.size.width,
            (blocked.top().min(region.bottom()) - region.top()).max(zero),
        );
        let below_top = blocked.bottom().max(region.top());
        let below = rect(
            region.left(),
            below_top,
            region.size.width,
            (region.bottom() - below_top).max(zero),
        );

        let half_height = (anchor_centre.y - region.top())
            .min(region.bottom() - anchor_centre.y)
            .max(zero);
        let side_band_y = anchor_centre.y - half_height;
        let side_band_height = half_height * 2.0;

        let left = rect(
            region.left(),
            side_band_y,
            (blocked.left().min(region.right()) - region.left()).max(zero),
            side_band_height,
        );
        let right_left = blocked.right().max(region.left());
        let right = rect(
            right_left,
            side_band_y,
            (region.right() - right_left).max(zero),
            side_band_height,
        );

        HashMap::from([
            (GuideOverlaySide::Above, above),
            (GuideOverlaySide::Below, below),
            (GuideOverlaySide::Leading, if is_right_to_left { right } else { left }),
            (GuideOverlaySide::Trailing, if is_right_to_left { left } else { right }),
        ])
    }

    fn choose_side(
        placement: RiseGuidePlacement,
        cutout: Bounds<Pixels>,
        origin: Option<Bounds<Pixels>>,
        bands: &Bands,
        metrics: GuideOverlayLayoutMetrics,
    ) -> GuideOverlaySide {
        let order = Self::preference_order(placement, bands);
        if let Some(fitting) = order.iter().copied().find(|side| Self::fits(*side, bands, metrics)) {
            return fitting;
        }
        if let Some(origin) = origin {
            return if mid_y(&origin) <= mid_y(&cutout) {
                GuideOverlaySide::Above
            } else {
                GuideOverlaySide::Below
            };
        }
        let vertical = Self::roomiest(&[GuideOverlaySide::Below, GuideOverlaySide::Above], bands);
        if let Some(vertical) = vertical
            && Self::length(vertical, bands) > px(0.0)
        {
            return vertical;
        }
        Self::roomiest(&order, bands).unwrap_or(GuideOverlaySide::Below)
    }

    fn preference_order(placement: RiseGuidePlacement, bands: &Bands) -> Vec<GuideOverlaySide> {
        use GuideOverlaySide::{Above, Below, Leading, Trailing};
        let vertical_first = if Self::length(Below, bands) >= Self::length(Above, bands) {
            [Below, Above]
        } else {
            [Above, Below]
        };
        let horizontal_first = if Self::length(Trailing, bands) >= Self::length(Leading, bands) {
            [Trailing, Leading]
        } else {
            [Leading, Trailing]
        };

        let (first, then) = match placement {
            RiseGuidePlacement::Auto => (vertical_first, horizontal_first),
            RiseGuidePlacement::Above => ([Above, Below], horizontal_first),
            RiseGuidePlacement::Below => ([Below, Above], horizontal_first),
            RiseGuidePlacement::Leading => ([Leading, Trailing], vertical_first),
            RiseGuidePlacement::Trailing => ([Trailing, Leading], vertical_first),
        };
        first.into_iter().chain(then).collect()
    }

    /// picks the roomiest side, on a tie keeps the earlier one
    fn roomiest(sides: &[GuideOverlaySide], bands: &Bands) -> Option<GuideOverlaySide> {
        sides.iter().copied().reduce(|best, side| {
            if Self::length(best, bands) < Self::length(side, bands) {
                side
            } else {
                best
            }
        })
    }

    fn length(side: GuideOverlaySide, bands: &Bands) -> Pixels {
        let Some(band) = bands.get(&side) else {
            return px(0.0);
        };
        if side.is_horizontal() {
            band.size.width
        } else {
            band.size.height
        }
    }

    fn fits(side: GuideOverlaySide, bands: &Bands, metrics: GuideOverlayLayoutMetrics) -> bool {
        let Some(band) = bands.get(&side) else {
            return false;
        };
        if band.size.width <= px(0.0) || band.size.height <= px(0.0) {
            return false;
        }
        if side.is_horizontal() {
            band.size.width >= metrics.minimum_horizontal_band
        } else {
            band.size.height >= metrics.minimum_vertical_band
        }
    }

    fn surface_width(
        band: Bounds<Pixels>,
        narrator_side: Pixels,
        limit: Pixels,
        metrics: GuideOverlayLayoutMetrics,
    ) -> Pixels {
        let available = band.size.width - narrator_side - metrics.group_spacing;
        let floor = limit.min(metrics.minimum_surface_width);
        floor.max(limit.min(available))
    }
}
