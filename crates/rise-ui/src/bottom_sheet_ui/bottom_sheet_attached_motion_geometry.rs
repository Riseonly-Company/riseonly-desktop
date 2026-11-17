use gpui::{Bounds, Pixels, point, px, size};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BottomSheetAttachedMotionGeometry {
    pub sheet_frame: Bounds<Pixels>,
    pub translation_y: Pixels,
}

fn is_finite(value: Pixels) -> bool {
    f32::from(value).is_finite()
}

impl BottomSheetAttachedMotionGeometry {
    pub const MINIMUM_BOTTOM_COVERAGE: Pixels = px(1.0);

    pub fn resolve(
        container_bounds: Bounds<Pixels>,
        sheet_height: Pixels,
        requested_translation_y: Pixels,
        committed_dismissal: bool,
    ) -> Self {
        let resolved_height = if is_finite(sheet_height) {
            Self::MINIMUM_BOTTOM_COVERAGE.max(sheet_height)
        } else {
            Self::MINIMUM_BOTTOM_COVERAGE
        };
        let requested = if is_finite(requested_translation_y) {
            px(0.0).max(requested_translation_y)
        } else {
            px(0.0)
        };
        let maximum_attached_translation =
            px(0.0).max(resolved_height - Self::MINIMUM_BOTTOM_COVERAGE);
        let resolved_translation = if committed_dismissal {
            requested
        } else {
            requested.min(maximum_attached_translation)
        };

        Self {
            sheet_frame: Bounds {
                origin: point(
                    container_bounds.left(),
                    container_bounds.bottom() - resolved_height + resolved_translation,
                ),
                size: size(container_bounds.size.width, resolved_height),
            },
            translation_y: resolved_translation,
        }
    }

    pub fn resolve_detent_interaction(
        container_bounds: Bounds<Pixels>,
        resting_height: Pixels,
        minimum_height: Pixels,
        maximum_height: Pixels,
        requested_translation_y: Pixels,
        allows_dismissal_preview: bool,
    ) -> Self {
        let resolved_maximum = if is_finite(maximum_height) {
            Self::MINIMUM_BOTTOM_COVERAGE.max(maximum_height)
        } else {
            Self::MINIMUM_BOTTOM_COVERAGE
        };
        let resolved_minimum = if is_finite(minimum_height) {
            resolved_maximum.min(Self::MINIMUM_BOTTOM_COVERAGE.max(minimum_height))
        } else {
            Self::MINIMUM_BOTTOM_COVERAGE
        };
        let resolved_resting = if is_finite(resting_height) {
            resolved_maximum.min(resolved_minimum.max(resting_height))
        } else {
            resolved_minimum
        };
        let translation = if is_finite(requested_translation_y) {
            requested_translation_y
        } else {
            px(0.0)
        };

        if translation < px(0.0) {
            return Self::attached(
                container_bounds,
                resolved_maximum.min(resolved_resting - translation),
            );
        }

        let contraction_distance = px(0.0).max(resolved_resting - resolved_minimum);
        if allows_dismissal_preview && translation > contraction_distance {
            return Self::resolve(
                container_bounds,
                resolved_minimum,
                translation - contraction_distance,
                false,
            );
        }

        Self::attached(
            container_bounds,
            resolved_minimum.max(resolved_resting - translation),
        )
    }

    pub fn covers_container_bottom(&self, container_bounds: Bounds<Pixels>) -> bool {
        self.sheet_frame.top() <= container_bounds.bottom() - Self::MINIMUM_BOTTOM_COVERAGE
            && self.sheet_frame.bottom() >= container_bounds.bottom()
    }

    fn attached(container_bounds: Bounds<Pixels>, height: Pixels) -> Self {
        Self {
            sheet_frame: Bounds {
                origin: point(container_bounds.left(), container_bounds.bottom() - height),
                size: size(container_bounds.size.width, height),
            },
            translation_y: px(0.0),
        }
    }
}
