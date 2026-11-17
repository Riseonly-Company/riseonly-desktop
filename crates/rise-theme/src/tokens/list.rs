use gpui::Pixels;

use crate::tokens::density::Density;

/// long list layout, page policy isnt here
/// a row asks for the next page through the SaiPaginationPrefetch twin
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ListMetrics {
    pub overdraw: Pixels,
}

impl ListMetrics {
    pub fn new(density: Density) -> Self {
        Self {
            overdraw: density.scale(320.0),
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
    fn density_scales_the_overdraw() {
        let dense = ListMetrics::new(Density::new(1.25));
        assert!(dense.overdraw > ListMetrics::default().overdraw);
    }
}
