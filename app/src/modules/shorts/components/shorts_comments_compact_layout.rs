use gpui::{Bounds, Pixels, Point, Size};

/// where the live reel parks under the bar when comments are up
/// same player so playback never restarts
/// assumes the cell is flush with window bottom
pub enum ShortsCommentsCompactLayout {}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShortsCommentsCompactLayoutPlacement {
    pub scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub unscaled_corner_radius: f32,
    pub video_bottom_y: f32,
    pub video_right_x: f32,
    pub parks_reel: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShortsCommentsCompactLayoutGeometry {
    pub cell_size: Size<Pixels>,
    pub cell_origin: Point<Pixels>,
    pub window_height: f32,
    pub safe_top: f32,
    pub resting_fraction: f32,
}

impl ShortsCommentsCompactLayoutGeometry {
    pub fn motion_placement(
        &self,
        sheet_fraction: f32,
        sheet_frame: Option<Bounds<Pixels>>,
        progress: f32,
    ) -> ShortsCommentsCompactLayoutPlacement {
        if sheet_frame.is_some_and(|sheet| ShortsCommentsCompactLayout::is_centred_dialog(Bounds::new(self.cell_origin, self.cell_size), sheet)) {
            return ShortsCommentsCompactLayoutPlacement::unmoved(self.cell_size);
        }
        let target = ShortsCommentsCompactLayout::placement(
            self.cell_size,
            self.window_height,
            self.safe_top,
            self.resting_fraction.max(sheet_fraction),
        );
        let amount = progress.clamp(0.0, 1.0);
        let scale = 1.0 + (target.scale - 1.0) * amount;
        let offset_x = target.offset_x * amount;
        let offset_y = target.offset_y * amount;
        let cell_width = f32::from(self.cell_size.width).max(1.0);
        let cell_height = f32::from(self.cell_size.height).max(1.0);
        ShortsCommentsCompactLayoutPlacement {
            scale,
            offset_x,
            offset_y,
            unscaled_corner_radius: ShortsCommentsCompactLayout::CORNER_RADIUS * amount / scale,
            video_bottom_y: cell_height * (1.0 + scale) / 2.0 + offset_y,
            video_right_x: cell_width * (1.0 + scale) / 2.0 + offset_x,
            parks_reel: true,
        }
    }
}

impl ShortsCommentsCompactLayoutPlacement {
    pub fn unmoved(cell_size: Size<Pixels>) -> Self {
        Self {
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            unscaled_corner_radius: 0.0,
            video_bottom_y: f32::from(cell_size.height).max(1.0),
            video_right_x: f32::from(cell_size.width).max(1.0),
            parks_reel: false,
        }
    }
}

impl ShortsCommentsCompactLayout {
    pub const BOTTOM_GAP: f32 = 12.0;
    pub const CORNER_RADIUS: f32 = 14.0;
    pub const DIALOG_SIDE_SLACK: f32 = 1.0;

    pub fn placement(
        cell_size: Size<Pixels>,
        window_height: f32,
        safe_top: f32,
        sheet_fraction: f32,
    ) -> ShortsCommentsCompactLayoutPlacement {
        let cell_width = f32::from(cell_size.width).max(1.0);
        let cell_height = f32::from(cell_size.height).max(1.0);
        let cell_top_in_window = (window_height - cell_height).max(0.0);
        let sheet_top_local = cell_height - window_height * sheet_fraction;
        let top_local = (safe_top - cell_top_in_window).max(0.0);
        let available_height = (sheet_top_local - top_local - Self::BOTTOM_GAP).max(1.0);
        let scale = (available_height / cell_height).min(1.0);
        let scaled_height = cell_height * scale;
        let offset_y = top_local - (cell_height - scaled_height) / 2.0;
        ShortsCommentsCompactLayoutPlacement {
            scale,
            offset_x: 0.0,
            offset_y,
            unscaled_corner_radius: Self::CORNER_RADIUS / scale,
            video_bottom_y: top_local + scaled_height,
            video_right_x: cell_width * (1.0 + scale) / 2.0,
            parks_reel: true,
        }
    }

    pub fn is_centred_dialog(cell: Bounds<Pixels>, sheet: Bounds<Pixels>) -> bool {
        let leading = f32::from(sheet.origin.x - cell.origin.x);
        let trailing = f32::from((cell.origin.x + cell.size.width) - (sheet.origin.x + sheet.size.width));
        leading > Self::DIALOG_SIDE_SLACK || trailing > Self::DIALOG_SIDE_SLACK
    }
}
