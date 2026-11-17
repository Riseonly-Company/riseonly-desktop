use std::sync::Arc;
use std::time::Duration;

pub const PICTURE_POLL: Duration = Duration::from_millis(8);

/// one picture as nv12 planes in cpu memory, rows packed with no padding
/// chroma is half size, rounded up and cb and cr interleaved
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nv12Picture {
    pub width: u32,
    pub height: u32,
    pub luma: Arc<[u8]>,
    pub luma_stride: u32,
    pub chroma: Arc<[u8]>,
    pub chroma_stride: u32,
    pub generation: u64,
}

/// full range bt601 luma and chroma of one bgra pixel, fixed point and rounded
pub fn ycbcr_of(blue: u8, green: u8, red: u8) -> (u8, u8, u8) {
    let (r, g, b) = (i32::from(red), i32::from(green), i32::from(blue));
    let y = (19_595 * r + 38_470 * g + 7_471 * b + 32_768) >> 16;
    let cb = ((-11_059 * r - 21_709 * g + 32_768 * b + 32_768) >> 16) + 128;
    let cr = ((32_768 * r - 27_439 * g - 5_329 * b + 32_768) >> 16) + 128;
    (clamp_byte(y), clamp_byte(cb), clamp_byte(cr))
}

fn clamp_byte(value: i32) -> u8 {
    u8::try_from(value.clamp(0, 255)).unwrap_or(u8::MAX)
}

/// turns bgra rows into packed nv12, chroma is the mean of each 2x2 block
/// none if the size is empty or the rows dont fit the buffer
pub fn bgra_to_nv12(bgra: &[u8], row_pitch: usize, width: u32, height: u32, generation: u64) -> Option<Nv12Picture> {
    let columns = usize::try_from(width).ok()?;
    let rows = usize::try_from(height).ok()?;
    if columns == 0 || rows == 0 || row_pitch < columns.checked_mul(4)? {
        return None;
    }
    let needed = row_pitch.checked_mul(rows - 1)?.checked_add(columns * 4)?;
    if bgra.len() < needed {
        return None;
    }
    let chroma_columns = columns.div_ceil(2);
    let chroma_rows = rows.div_ceil(2);
    let mut luma = vec![0_u8; columns * rows];
    let mut chroma = vec![0_u8; chroma_columns * 2 * chroma_rows];
    for chroma_row in 0..chroma_rows {
        for chroma_column in 0..chroma_columns {
            let (mut cb_sum, mut cr_sum, mut count) = (0_u32, 0_u32, 0_u32);
            for row in (chroma_row * 2)..((chroma_row * 2 + 2).min(rows)) {
                for column in (chroma_column * 2)..((chroma_column * 2 + 2).min(columns)) {
                    let at = row * row_pitch + column * 4;
                    let (y, cb, cr) = ycbcr_of(bgra[at], bgra[at + 1], bgra[at + 2]);
                    luma[row * columns + column] = y;
                    cb_sum += u32::from(cb);
                    cr_sum += u32::from(cr);
                    count += 1;
                }
            }
            let at = chroma_row * chroma_columns * 2 + chroma_column * 2;
            chroma[at] = u8::try_from((cb_sum + count / 2) / count).unwrap_or(u8::MAX);
            chroma[at + 1] = u8::try_from((cr_sum + count / 2) / count).unwrap_or(u8::MAX);
        }
    }
    Some(Nv12Picture {
        width,
        height,
        luma: Arc::from(luma),
        luma_stride: width,
        chroma: Arc::from(chroma),
        chroma_stride: u32::try_from(chroma_columns * 2).ok()?,
        generation,
    })
}

/// what one answer of the video stream tick means for the stream
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamTick {
    Same,
    New,
    Wrapped,
}

/// follows the presentation times of the pictures the engine reports
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PictureClock {
    last: Option<i64>,
}

impl PictureClock {
    pub fn tick(&mut self, presentation_time: i64) -> StreamTick {
        let previous = self.last.replace(presentation_time);
        match previous {
            Some(last) if presentation_time == last => StreamTick::Same,
            Some(last) if presentation_time < last => StreamTick::Wrapped,
            _ => StreamTick::New,
        }
    }

    /// forgets the last time for a new source, so its first picture is never seen as a wrap
    pub fn reset(&mut self) {
        self.last = None;
    }
}
