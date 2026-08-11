use gpui::Pixels;

use crate::tokens::density::Density;

/// How a long list behaves at its edges.
///
/// Pagination triggers on DISTANCE from the end, never on a row index: a row
/// count is a viewport on a phone and a third of one in a tall window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ListMetrics {
    /// How far past the visible range the list measures and mounts rows, so a
    /// fling does not reveal unmeasured space.
    pub overdraw: Pixels,
    /// The floor and the ceiling on the pagination trigger distance.
    pub pagination_min_distance: Pixels,
    pub pagination_max_distance: Pixels,
}

impl ListMetrics {
    /// The speed-independent term of the trigger distance, in viewports.
    pub const LEAD_VIEWPORTS: f32 = 1.0;

    /// How far ahead the velocity term looks, in seconds.
    pub const LEAD_SECONDS: f32 = 0.6;

    pub fn new(density: Density) -> Self {
        let l = |value: f32| density.scale(value);

        Self {
            overdraw: l(320.0),
            pagination_min_distance: l(400.0),
            pagination_max_distance: l(2400.0),
        }
    }
}

impl Default for ListMetrics {
    fn default() -> Self {
        Self::new(Density::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bounds_are_ordered_and_the_floor_clears_a_viewport() {
        let list = ListMetrics::default();
        assert!(list.pagination_min_distance < list.pagination_max_distance);
        assert!(
            list.pagination_min_distance > list.overdraw,
            "a request that starts inside the overdraw band has already lost the race"
        );
    }

    #[test]
    fn density_scales_the_distances() {
        let dense = ListMetrics::new(Density::new(1.25));
        assert!(dense.overdraw > ListMetrics::default().overdraw);
    }
}
