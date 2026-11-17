use gpui::{Bounds, Pixels, Point, point, px};

use crate::core::shared_element_transition::kernel::shared_element_types::{
    SharedElementReturnScrollMath, SharedElementReturnScrollMathBand, SharedElementReturnScrollMathClamp,
    SharedElementReturnScrollMathPosition,
};

impl SharedElementReturnScrollMath {
    pub fn cell_width(container_width: f32, columns: usize, spacing: f32) -> f32 {
        let count = columns.max(1);
        let total_spacing = spacing * (count - 1) as f32;
        ((container_width - total_spacing) / count as f32).max(0.0)
    }

    /// item rect in inner content coords of a fixed column grid laid out top leading, no leading inset
    pub fn grid_item_rect(
        index: usize,
        columns: usize,
        cell_width: f32,
        cell_height: f32,
        spacing: f32,
        origin_y: f32,
    ) -> Bounds<Pixels> {
        Self::grid_item_rect_with_origin_x(index, columns, cell_width, cell_height, spacing, origin_y, 0.0)
    }

    /// item rect in inner content coords of a fixed column grid laid out top leading
    /// origin_x is the leading inset of the grid
    pub fn grid_item_rect_with_origin_x(
        index: usize,
        columns: usize,
        cell_width: f32,
        cell_height: f32,
        spacing: f32,
        origin_y: f32,
        origin_x: f32,
    ) -> Bounds<Pixels> {
        let count = columns.max(1);
        let row = index / count;
        let column = index % count;
        Bounds::new(
            point(
                px(origin_x + column as f32 * (cell_width + spacing)),
                px(origin_y + row as f32 * (cell_height + spacing)),
            ),
            gpui::size(px(cell_width), px(cell_height)),
        )
    }

    /// band center in viewport local coordinates
    pub fn band_center_y(band: SharedElementReturnScrollMathBand, viewport_height: f32) -> f32 {
        (band.top + (viewport_height - band.bottom_inset)) / 2.0
    }

    /// master offset that centers the item on the band center, clamped to 0 up to C + R
    /// with Top the header is fully expanded and with Bottom the grid end stops the cell below center
    pub fn position_item_mid_y(
        item_mid_y: f32,
        band_center_y: f32,
        pin_offset: f32,
        collapse_range: f32,
        inner_range: f32,
        epsilon: f32,
    ) -> SharedElementReturnScrollMathPosition {
        let collapse = collapse_range.max(0.0);
        let inner = inner_range.max(0.0);
        let upper_bound = collapse + inner;
        let desired = collapse + pin_offset + item_mid_y - band_center_y;

        let (master, clamp) = if desired < 0.0 {
            (0.0, SharedElementReturnScrollMathClamp::Top)
        } else if desired > upper_bound {
            (upper_bound, SharedElementReturnScrollMathClamp::Bottom)
        } else {
            (desired, SharedElementReturnScrollMathClamp::None)
        };

        Self::resolved(master, collapse, epsilon, clamp)
    }

    /// keeps the current master offset when the item already sits wholly
    /// in the band so returning to a visible cell doesnt move the grid
    /// otherwise centers it like position_item_mid_y
    #[allow(clippy::too_many_arguments)]
    pub fn position_item_rect(
        item_rect: Bounds<Pixels>,
        band: SharedElementReturnScrollMathBand,
        viewport_height: f32,
        current_master_offset: Option<f32>,
        pin_offset: f32,
        collapse_range: f32,
        inner_range: f32,
        epsilon: f32,
    ) -> SharedElementReturnScrollMathPosition {
        let collapse = collapse_range.max(0.0);
        let upper_bound = collapse + inner_range.max(0.0);
        if let Some(current_master_offset) = current_master_offset.filter(|offset| offset.is_finite()) {
            let master = current_master_offset.max(0.0).min(upper_bound);
            let top = pin_offset + collapse + f32::from(item_rect.top()) - master;
            let tolerance = 0.5_f32;
            if top >= band.top - tolerance
                && top + f32::from(item_rect.size.height) <= viewport_height - band.bottom_inset + tolerance
            {
                return Self::resolved(master, collapse, epsilon, SharedElementReturnScrollMathClamp::None);
            }
        }
        Self::position_item_mid_y(
            f32::from(item_rect.center().y),
            Self::band_center_y(band, viewport_height),
            pin_offset,
            collapse_range,
            inner_range,
            epsilon,
        )
    }

    /// screen frame of item_rect once the master offset is master_offset
    pub fn predicted_screen_frame(
        item_rect: Bounds<Pixels>,
        viewport_origin: Point<Pixels>,
        pin_offset: f32,
        collapse_range: f32,
        master_offset: f32,
    ) -> Bounds<Pixels> {
        Bounds::new(
            point(
                viewport_origin.x + item_rect.origin.x,
                viewport_origin.y + px(pin_offset + collapse_range.max(0.0) - master_offset) + item_rect.origin.y,
            ),
            item_rect.size,
        )
    }

    fn resolved(
        master: f32,
        collapse: f32,
        epsilon: f32,
        clamp: SharedElementReturnScrollMathClamp,
    ) -> SharedElementReturnScrollMathPosition {
        SharedElementReturnScrollMathPosition {
            master_offset: master,
            collapse_progress: if collapse > epsilon { (master / collapse).min(1.0) } else { 1.0 },
            inner_offset: (master - collapse).max(0.0),
            clamp,
        }
    }
}
