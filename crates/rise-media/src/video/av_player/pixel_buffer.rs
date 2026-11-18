use gpui::RenderImage;
use image::{Frame, RgbaImage};
use smallvec::SmallVec;

use crate::video::av_player::media_engine_frames::Nv12Picture;

/// os side of a copied picture, makes an upright bgra image
/// runs off the main thread, once
pub trait PixelBufferImage: Send {
    fn oriented_image(self: Box<Self>) -> Option<RenderImage>;
}

/// pixel buffer copied out of a player, owns the picture and never touches the player
pub struct AVPlayerPixelBuffer {
    image: Box<dyn PixelBufferImage>,
}

impl AVPlayerPixelBuffer {
    pub fn new(image: Box<dyn PixelBufferImage>) -> Self {
        Self { image }
    }

    pub fn oriented_image(self) -> Option<RenderImage> {
        self.image.oriented_image()
    }
}

/// copied nv12 picture from a software decoder with its color matrix and turn
pub struct Nv12PixelBuffer {
    picture: Nv12Picture,
    ycbcr_to_rgb: [[f32; 4]; 4],
    clockwise_quarter_turns: u8,
}

impl Nv12PixelBuffer {
    /// ycbcr_to_rgb is the column major matrix the surface shader applies to y cb cr 1
    /// values are in 0 to 1, quarter turns are taken modulo 4
    pub fn new(picture: Nv12Picture, ycbcr_to_rgb: [[f32; 4]; 4], clockwise_quarter_turns: u8) -> Self {
        Self { picture, ycbcr_to_rgb, clockwise_quarter_turns: clockwise_quarter_turns % 4 }
    }
}

impl PixelBufferImage for Nv12PixelBuffer {
    fn oriented_image(self: Box<Self>) -> Option<RenderImage> {
        let (width, height, bgra) = oriented_bgra_of_nv12(&self.picture, &self.ycbcr_to_rgb, self.clockwise_quarter_turns)?;
        render_image_of_bgra(width, height, bgra)
    }
}

/// clockwise quarter turns needed to show a picture upright for an ffmpeg display rotation
pub fn clockwise_quarter_turns_of_display_rotation(rotation_degrees: f64) -> u8 {
    if !rotation_degrees.is_finite() {
        return 0;
    }
    let turns = (-rotation_degrees / 90.0).round() as i64;
    u8::try_from(turns.rem_euclid(4)).unwrap_or(0)
}

/// converts the nv12 picture to bgra and turns it clockwise, alpha is opaque
/// none for an empty picture or planes too short for their strides
pub fn oriented_bgra_of_nv12(
    picture: &Nv12Picture,
    ycbcr_to_rgb: &[[f32; 4]; 4],
    clockwise_quarter_turns: u8,
) -> Option<(u32, u32, Vec<u8>)> {
    let columns = usize::try_from(picture.width).ok()?;
    let rows = usize::try_from(picture.height).ok()?;
    if columns == 0 || rows == 0 {
        return None;
    }
    let luma_stride = usize::try_from(picture.luma_stride).ok()?;
    let chroma_stride = usize::try_from(picture.chroma_stride).ok()?;
    let chroma_columns = columns.div_ceil(2);
    let chroma_rows = rows.div_ceil(2);
    if luma_stride < columns || chroma_stride < chroma_columns * 2 {
        return None;
    }
    let luma_needed = luma_stride.checked_mul(rows - 1)?.checked_add(columns)?;
    let chroma_needed = chroma_stride.checked_mul(chroma_rows - 1)?.checked_add(chroma_columns * 2)?;
    if picture.luma.len() < luma_needed || picture.chroma.len() < chroma_needed {
        return None;
    }
    let turns = clockwise_quarter_turns % 4;
    let (out_columns, out_rows) = if turns % 2 == 1 { (rows, columns) } else { (columns, rows) };
    let mut bgra = vec![0_u8; out_columns.checked_mul(out_rows)?.checked_mul(4)?];
    for row in 0..rows {
        for column in 0..columns {
            let luma = picture.luma[row * luma_stride + column];
            let at = (row / 2) * chroma_stride + (column / 2) * 2;
            let (cb, cr) = (picture.chroma[at], picture.chroma[at + 1]);
            let [red, green, blue] = rgb_of(ycbcr_to_rgb, luma, cb, cr);
            let (to_column, to_row) = match turns {
                1 => (rows - 1 - row, column),
                2 => (columns - 1 - column, rows - 1 - row),
                3 => (row, columns - 1 - column),
                _ => (column, row),
            };
            let to = (to_row * out_columns + to_column) * 4;
            bgra[to..to + 4].copy_from_slice(&[blue, green, red, u8::MAX]);
        }
    }
    Some((u32::try_from(out_columns).ok()?, u32::try_from(out_rows).ok()?, bgra))
}

/// nv12 picture turned clockwise by quarter turns, rows packed, none for planes too short for their strides
pub fn turned_nv12(picture: &Nv12Picture, clockwise_quarter_turns: u8) -> Option<Nv12Picture> {
    let columns = usize::try_from(picture.width).ok()?;
    let rows = usize::try_from(picture.height).ok()?;
    if columns == 0 || rows == 0 {
        return None;
    }
    let luma_stride = usize::try_from(picture.luma_stride).ok()?;
    let chroma_stride = usize::try_from(picture.chroma_stride).ok()?;
    let chroma_columns = columns.div_ceil(2);
    let chroma_rows = rows.div_ceil(2);
    if luma_stride < columns || chroma_stride < chroma_columns * 2 {
        return None;
    }
    let luma_needed = luma_stride.checked_mul(rows - 1)?.checked_add(columns)?;
    let chroma_needed = chroma_stride.checked_mul(chroma_rows - 1)?.checked_add(chroma_columns * 2)?;
    if picture.luma.len() < luma_needed || picture.chroma.len() < chroma_needed {
        return None;
    }
    let turns = clockwise_quarter_turns % 4;
    if turns == 0 {
        return Some(picture.clone());
    }
    let (out_columns, out_rows) = if turns % 2 == 1 { (rows, columns) } else { (columns, rows) };
    let source = |out_column: usize, out_row: usize| match turns {
        1 => (out_row, rows - 1 - out_column),
        2 => (columns - 1 - out_column, rows - 1 - out_row),
        _ => (columns - 1 - out_row, out_column),
    };
    let mut luma = Vec::with_capacity(out_columns.checked_mul(out_rows)?);
    for out_row in 0..out_rows {
        for out_column in 0..out_columns {
            let (column, row) = source(out_column, out_row);
            luma.push(picture.luma[row * luma_stride + column]);
        }
    }
    let (out_chroma_columns, out_chroma_rows) = (out_columns.div_ceil(2), out_rows.div_ceil(2));
    let mut chroma = Vec::with_capacity(out_chroma_columns.checked_mul(out_chroma_rows)?.checked_mul(2)?);
    for out_row in 0..out_chroma_rows {
        for out_column in 0..out_chroma_columns {
            let (column, row) = source(out_column * 2, out_row * 2);
            let at = (row / 2) * chroma_stride + (column / 2) * 2;
            chroma.extend_from_slice(&picture.chroma[at..at + 2]);
        }
    }
    Some(Nv12Picture {
        width: u32::try_from(out_columns).ok()?,
        height: u32::try_from(out_rows).ok()?,
        luma: luma.into(),
        luma_stride: u32::try_from(out_columns).ok()?,
        chroma: chroma.into(),
        chroma_stride: u32::try_from(out_chroma_columns * 2).ok()?,
        generation: picture.generation,
    })
}

fn rgb_of(columns: &[[f32; 4]; 4], luma: u8, cb: u8, cr: u8) -> [u8; 3] {
    let yuv = [f32::from(luma) / 255.0, f32::from(cb) / 255.0, f32::from(cr) / 255.0];
    let mut rgb = [0_u8; 3];
    for (channel, out) in rgb.iter_mut().enumerate() {
        let value = columns[0][channel] * yuv[0] + columns[1][channel] * yuv[1] + columns[2][channel] * yuv[2] + columns[3][channel];
        *out = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    rgb
}

/// one frame gpui image from packed bgra rows, the byte order gpui uploads
pub fn render_image_of_bgra(width: u32, height: u32, bgra: Vec<u8>) -> Option<RenderImage> {
    let buffer = RgbaImage::from_raw(width, height, bgra)?;
    let mut frames: SmallVec<[Frame; 1]> = SmallVec::new();
    frames.push(Frame::new(buffer));
    Some(RenderImage::new(frames))
}
