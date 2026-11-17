use std::sync::Arc;
use std::time::Duration;

use crate::texture::color::{ColorDescription, ColorRange, ColorSpace};
use crate::video::av_player::media_engine_frames::Nv12Picture;
use crate::video::decoder::HostI420Planes;

pub const RESYNC_AFTER: Duration = Duration::from_millis(250);

pub const LONGEST_WAIT: Duration = Duration::from_secs(1);

/// when one decoded picture should be shown
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PictureDue {
    Now,
    After(Duration),
}

/// maps presentation times (microseconds of the stream) to the monotonic host clock
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ClipClock {
    anchor: Option<(Duration, i64)>,
    paused_at: Option<Duration>,
}

impl ClipClock {
    /// when the picture at pts_us is due, host time being now
    /// the first picture after restart is due at once and anchors the pacing
    pub fn due(&mut self, pts_us: i64, now: Duration) -> PictureDue {
        let Some((host, pts)) = self.anchor else {
            self.anchor = Some((now, pts_us));
            return PictureDue::Now;
        };
        let ahead_us = pts_us.saturating_sub(pts);
        let Ok(ahead_us) = u64::try_from(ahead_us) else {
            self.anchor = Some((now, pts_us));
            return PictureDue::Now;
        };
        let target = host.saturating_add(Duration::from_micros(ahead_us));
        if target <= now {
            if now - target > RESYNC_AFTER {
                self.anchor = Some((now, pts_us));
            }
            return PictureDue::Now;
        }
        let wait = target - now;
        if wait > LONGEST_WAIT {
            self.anchor = Some((now, pts_us));
            return PictureDue::Now;
        }
        PictureDue::After(wait)
    }

    /// stops the clock at now
    /// the picture due later waits until resume
    pub fn pause(&mut self, now: Duration) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }

    /// starts the clock again at now, shifted by the time it stood
    pub fn resume(&mut self, now: Duration) {
        let Some(paused_at) = self.paused_at.take() else {
            return;
        };
        if let Some((host, pts)) = self.anchor {
            self.anchor = Some((host.saturating_add(now.saturating_sub(paused_at)), pts));
        }
    }

    /// new pass or new source, the next picture anchors the pacing again
    pub fn restart(&mut self) {
        self.anchor = None;
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }
}

/// what the end of one pass of the clip does
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PassEnd {
    pub posts_end: bool,
    pub restarts: bool,
}

/// end of a pass
/// without a looper the player holds the last picture whatever the end action is
/// advance leaves an empty queue and pause and none stop at media end, layer keeps the last picture
pub const fn pass_end(looping: bool) -> PassEnd {
    PassEnd { posts_end: true, restarts: looping }
}

const AVCOL_SPC_BT709: i32 = 1;
const AVCOL_SPC_BT470BG: i32 = 5;
const AVCOL_SPC_SMPTE170M: i32 = 6;
const AVCOL_SPC_BT2020_NCL: i32 = 9;
const AVCOL_SPC_BT2020_CL: i32 = 10;
const AVCOL_RANGE_MPEG: i32 = 1;
const AVCOL_RANGE_JPEG: i32 = 2;

/// colour description of a decoded picture from its colorspace and color_range
/// full_range_format is a yuvj pixel format which is full range whatever the tag says
/// untagged values go through ColorDescription resolve
pub fn picture_color(colorspace: i32, color_range: i32, full_range_format: bool, height: u32) -> ColorDescription {
    let space = match colorspace {
        AVCOL_SPC_BT709 => Some(ColorSpace::Bt709),
        AVCOL_SPC_BT470BG | AVCOL_SPC_SMPTE170M => Some(ColorSpace::Bt601),
        AVCOL_SPC_BT2020_NCL | AVCOL_SPC_BT2020_CL => Some(ColorSpace::Bt2020),
        _ => None,
    };
    let range = if full_range_format {
        Some(ColorRange::Full)
    } else {
        match color_range {
            AVCOL_RANGE_JPEG => Some(ColorRange::Full),
            AVCOL_RANGE_MPEG => Some(ColorRange::Limited),
            _ => None,
        }
    };
    ColorDescription::resolve(space, range, height)
}

/// packed nv12 planes of an 8 bit 4:2:0 planar picture, luma rows copied and cb cr rows interleaved
/// none when a plane is shorter than its rows and strides need
pub fn nv12_from_i420(planes: &HostI420Planes<'_>, generation: u64) -> Option<Nv12Picture> {
    let width = usize::try_from(planes.width).ok().filter(|width| *width > 0)?;
    let height = usize::try_from(planes.height).ok().filter(|height| *height > 0)?;
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    let stride_y = usize::try_from(planes.stride_y).ok()?;
    let stride_u = usize::try_from(planes.stride_u).ok()?;
    let stride_v = usize::try_from(planes.stride_v).ok()?;
    if !plane_fits(planes.y.len(), stride_y, width, height)
        || !plane_fits(planes.u.len(), stride_u, chroma_width, chroma_height)
        || !plane_fits(planes.v.len(), stride_v, chroma_width, chroma_height)
    {
        return None;
    }
    let mut luma = Vec::with_capacity(width * height);
    for row in 0..height {
        luma.extend_from_slice(&planes.y[row * stride_y..row * stride_y + width]);
    }
    let mut chroma = Vec::with_capacity(chroma_width * 2 * chroma_height);
    for row in 0..chroma_height {
        let u = &planes.u[row * stride_u..row * stride_u + chroma_width];
        let v = &planes.v[row * stride_v..row * stride_v + chroma_width];
        for (cb, cr) in u.iter().zip(v) {
            chroma.push(*cb);
            chroma.push(*cr);
        }
    }
    Some(Nv12Picture {
        width: planes.width,
        height: planes.height,
        luma: Arc::from(luma),
        luma_stride: planes.width,
        chroma: Arc::from(chroma),
        chroma_stride: u32::try_from(chroma_width * 2).ok()?,
        generation,
    })
}

/// packed nv12 planes of a semi planar picture whose rows are stride_y and stride_uv bytes apart
/// none when a plane is shorter than its rows and strides need
pub fn nv12_from_semi_planar(
    width: u32,
    height: u32,
    y: &[u8],
    stride_y: u32,
    uv: &[u8],
    stride_uv: u32,
    generation: u64,
) -> Option<Nv12Picture> {
    let columns = usize::try_from(width).ok().filter(|columns| *columns > 0)?;
    let rows = usize::try_from(height).ok().filter(|rows| *rows > 0)?;
    let chroma_row = columns.div_ceil(2) * 2;
    let chroma_rows = rows.div_ceil(2);
    let stride_y = usize::try_from(stride_y).ok()?;
    let stride_uv = usize::try_from(stride_uv).ok()?;
    if !plane_fits(y.len(), stride_y, columns, rows) || !plane_fits(uv.len(), stride_uv, chroma_row, chroma_rows) {
        return None;
    }
    let mut luma = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        luma.extend_from_slice(&y[row * stride_y..row * stride_y + columns]);
    }
    let mut chroma = Vec::with_capacity(chroma_row * chroma_rows);
    for row in 0..chroma_rows {
        chroma.extend_from_slice(&uv[row * stride_uv..row * stride_uv + chroma_row]);
    }
    Some(Nv12Picture {
        width,
        height,
        luma: Arc::from(luma),
        luma_stride: width,
        chroma: Arc::from(chroma),
        chroma_stride: u32::try_from(chroma_row).ok()?,
        generation,
    })
}

/// true if len bytes hold rows rows of row bytes stride apart
fn plane_fits(len: usize, stride: usize, row: usize, rows: usize) -> bool {
    if stride < row || rows == 0 {
        return false;
    }
    stride.checked_mul(rows - 1).and_then(|start| start.checked_add(row)).is_some_and(|needed| needed <= len)
}
