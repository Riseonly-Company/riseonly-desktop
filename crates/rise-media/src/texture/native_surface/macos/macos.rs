use std::ffi::c_void;
use std::sync::Arc;

use core_foundation::base::{CFAllocatorRef, CFGetTypeID, CFRelease, CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_video::pixel_buffer::{
    CVPixelBuffer, CVPixelBufferKeys, CVPixelBufferRef, kCVPixelFormatType_32BGRA,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
};
use core_video::pixel_buffer_pool::CVPixelBufferPool;
use core_video::r#return::kCVReturnSuccess;
use gpui::{AnyElement, IntoElement, Styled};
use parking_lot::Mutex;
use rise_platform::HostOs;

use crate::texture::color::ConversionParams;
use crate::texture::native_surface::*;

pub(crate) const DRAWABLE_PIXEL_FORMAT: u32 = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange;

pub(crate) const BGRA_PIXEL_FORMAT: u32 = kCVPixelFormatType_32BGRA;

pub(crate) struct NativeSurfaceBinding;

/// cvpixelbuffer this branch holds one corefoundation reference to, drop releases it
pub(crate) struct HeldPixelBuffer(CVPixelBuffer);

unsafe impl Send for HeldPixelBuffer {}
unsafe impl Sync for HeldPixelBuffer {}

impl SurfaceBinding for NativeSurfaceBinding {
    unsafe fn retain(
        &self,
        _host: HostOs,
        address: *mut c_void,
    ) -> Result<RetainedSurface, SurfaceError> {
        if address.is_null() {
            return Err(SurfaceError::Null);
        }
        let type_id = unsafe { CFGetTypeID(address as CFTypeRef) };
        if type_id != <CVPixelBuffer as TCFType>::type_id() {
            return Err(SurfaceError::WrongKind {
                expected: "a CVPixelBuffer",
            });
        }
        let buffer = unsafe { CVPixelBuffer::wrap_under_get_rule(address as CVPixelBufferRef) };

        Ok(RetainedSurface::new(
            address as usize,
            HeldPixelBuffer(buffer),
        ))
    }

    fn present(&self, _host: HostOs, surface: &RetainedSurface) -> Result<AnyElement, SurfaceError> {
        drawable_buffer(surface).map(|buffer| gpui::surface(buffer).size_full().into_any_element())
    }

    fn prepare_host_frame(
        &self,
        _host: HostOs,
        frame: HostNv12Frame,
    ) -> Result<PreparedHostFrame, SurfaceError> {
        let buffer = host_frame_buffer(&frame)?;
        let address = buffer.as_concrete_TypeRef() as usize;

        Ok(PreparedHostFrame::Surface {
            stream_id: frame.stream_id,
            generation: frame.generation,
            width: frame.width,
            height: frame.height,
            surface: RetainedSurface::new(address, HeldPixelBuffer(buffer)),
        })
    }

    fn prepare_host_bgra_frame(
        &self,
        _host: HostOs,
        frame: HostBgraFrame,
    ) -> Result<PreparedHostFrame, SurfaceError> {
        let buffer = host_bgra_buffer(&frame)?;
        let address = buffer.as_concrete_TypeRef() as usize;

        Ok(PreparedHostFrame::Surface {
            stream_id: frame.stream_id,
            generation: frame.generation,
            width: frame.width,
            height: frame.height,
            surface: RetainedSurface::new(address, HeldPixelBuffer(buffer)),
        })
    }

    fn present_rotated(
        &self,
        _host: HostOs,
        surface: &RetainedSurface,
        clockwise_quarter_turns: u8,
    ) -> Result<AnyElement, SurfaceError> {
        rotated_buffer(surface, clockwise_quarter_turns)
            .map(|buffer| gpui::surface(buffer).size_full().into_any_element())
    }
}

/// the held buffer when this branch built the surface and metal can draw its format
pub(crate) fn drawable_buffer(surface: &RetainedSurface) -> Result<CVPixelBuffer, SurfaceError> {
    let Some(HeldPixelBuffer(buffer)) = surface.keeper::<HeldPixelBuffer>() else {
        return Err(SurfaceError::WrongKind {
            expected: "a CVPixelBuffer held by the macOS branch",
        });
    };
    let format = buffer.get_pixel_format();
    if format != DRAWABLE_PIXEL_FORMAT && format != BGRA_PIXEL_FORMAT {
        return Err(SurfaceError::WrongKind {
            expected: "a full-range NV12 ('420f') or a 32BGRA CVPixelBuffer",
        });
    }

    Ok(buffer.clone())
}

pub(crate) const METAL_SURFACE_YCBCR_TO_RGB: [[f32; 4]; 4] = [
    [1.0, 1.0, 1.0, 0.0],
    [0.0, -0.3441, 1.7720, 0.0],
    [1.4020, -0.7141, 0.0, 0.0],
    [-0.7010, 0.5291, -0.8860, 1.0],
];

pub(crate) const HELD_STREAMS: usize = 8;

const FIXED_ONE: f32 = 65_536.0;
const IDENTITY_TOLERANCE: f32 = 0.002;

/// affine map from a stream ycbcr to the ycbcr the metal surface matrix turns into the same rgb
/// out = matrix * ycbcr + offset, all in 0..1 and row major
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Reencode {
    pub(crate) matrix: [[f32; 3]; 3],
    pub(crate) offset: [f32; 3],
}

impl Reencode {
    /// built from the column major ycbcr1 to rgb matrix of the stream
    /// none only when the target matrix is singular
    pub(crate) fn for_metal(source: &[[f32; 4]; 4]) -> Option<Self> {
        let target = &METAL_SURFACE_YCBCR_TO_RGB;
        let inverse = invert(linear_part(target))?;
        let source_linear = linear_part(source);
        let mut matrix = [[0.0_f32; 3]; 3];
        for (row, out) in matrix.iter_mut().enumerate() {
            for (column, cell) in out.iter_mut().enumerate() {
                *cell = (0..3)
                    .map(|inner| inverse[row][inner] * source_linear[inner][column])
                    .sum();
            }
        }
        let shift = [
            source[3][0] - target[3][0],
            source[3][1] - target[3][1],
            source[3][2] - target[3][2],
        ];
        let mut offset = [0.0_f32; 3];
        for (row, out) in offset.iter_mut().enumerate() {
            *out = (0..3).map(|inner| inverse[row][inner] * shift[inner]).sum();
        }

        Some(Self { matrix, offset })
    }

    /// true when the stream is already full range bt601, planes are copied as they are
    pub(crate) fn is_identity(&self) -> bool {
        self.matrix.iter().enumerate().all(|(row, cells)| {
            cells.iter().enumerate().all(|(column, cell)| {
                let expected = if row == column { 1.0 } else { 0.0 };
                (cell - expected).abs() < IDENTITY_TOLERANCE
            })
        }) && self
            .offset
            .iter()
            .all(|value| value.abs() < IDENTITY_TOLERANCE)
    }
}

/// 3x3 linear part of a column major ycbcr1 to rgb matrix
fn linear_part(columns: &[[f32; 4]; 4]) -> [[f32; 3]; 3] {
    let mut linear = [[0.0_f32; 3]; 3];
    for (row, out) in linear.iter_mut().enumerate() {
        for (column, cell) in out.iter_mut().enumerate() {
            *cell = columns[column][row];
        }
    }
    linear
}

fn invert(m: [[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let cofactor = |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let det = m[0][0] * cofactor(1, 2, 1, 2) - m[0][1] * cofactor(1, 2, 0, 2)
        + m[0][2] * cofactor(1, 2, 0, 1);
    if det.abs() < f32::EPSILON {
        return None;
    }
    let adjugate = [
        [cofactor(1, 2, 1, 2), -cofactor(0, 2, 1, 2), cofactor(0, 1, 1, 2)],
        [-cofactor(1, 2, 0, 2), cofactor(0, 2, 0, 2), -cofactor(0, 1, 0, 2)],
        [cofactor(1, 2, 0, 1), -cofactor(0, 2, 0, 1), cofactor(0, 1, 0, 1)],
    ];
    Some(adjugate.map(|row| row.map(|cell| cell / det)))
}

/// fixed point lookup tables of a Reencode
/// one output channel is the sum of three 16.16 entries
/// indexed by the 8 bit y cb cr codes with the offset folded into the y table
pub(crate) struct ReencodeTables {
    channels: [[[i32; 256]; 3]; 3],
}

impl ReencodeTables {
    pub(crate) fn new(reencode: &Reencode) -> Self {
        let mut channels = [[[0_i32; 256]; 3]; 3];
        for (channel, tables) in channels.iter_mut().enumerate() {
            for (input, table) in tables.iter_mut().enumerate() {
                let bias = if input == 0 {
                    255.0 * reencode.offset[channel]
                } else {
                    0.0
                };
                for (code, entry) in table.iter_mut().enumerate() {
                    let value = reencode.matrix[channel][input] * code as f32 + bias;
                    *entry = (value * FIXED_ONE).round() as i32;
                }
            }
        }
        Self { channels }
    }

    /// one output code of channel (0 y 1 cb 2 cr) rounded and clamped to 0..=255
    pub(crate) fn code(&self, channel: usize, y: u8, cb: u8, cr: u8) -> u8 {
        let tables = &self.channels[channel];
        let sum = tables[0][usize::from(y)] + tables[1][usize::from(cb)] + tables[2][usize::from(cr)];
        ((sum + (1 << 15)) >> 16).clamp(0, 255) as u8
    }
}

/// plane geometry of a cpu frame, after checking the planes hold every row they claim
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct HostFrameGeometry {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) chroma_width: usize,
    pub(crate) chroma_height: usize,
    pub(crate) luma_stride: usize,
    pub(crate) chroma_stride: usize,
}

impl HostFrameGeometry {
    pub(crate) fn of(frame: &HostNv12Frame) -> Result<Self, SurfaceError> {
        let short = SurfaceError::WrongKind {
            expected: "NV12 planes as long as their rows and strides need",
        };
        let width = usize::try_from(frame.width).map_err(|_| short.clone())?;
        let height = usize::try_from(frame.height).map_err(|_| short.clone())?;
        let luma_stride = usize::try_from(frame.luma_stride).map_err(|_| short.clone())?;
        let chroma_stride = usize::try_from(frame.chroma_stride).map_err(|_| short.clone())?;
        let chroma_width = width.div_ceil(2);
        let chroma_height = height.div_ceil(2);
        if width == 0 || height == 0 || luma_stride < width || chroma_stride < 2 * chroma_width {
            return Err(short);
        }
        let luma_needed = (height - 1)
            .checked_mul(luma_stride)
            .and_then(|rows| rows.checked_add(width));
        let chroma_needed = (chroma_height - 1)
            .checked_mul(chroma_stride)
            .and_then(|rows| rows.checked_add(2 * chroma_width));
        match (luma_needed, chroma_needed) {
            (Some(luma), Some(chroma)) if frame.luma.len() >= luma && frame.chroma.len() >= chroma => {
                Ok(Self {
                    width,
                    height,
                    chroma_width,
                    chroma_height,
                    luma_stride,
                    chroma_stride,
                })
            }
            _ => Err(short),
        }
    }
}

/// one playing stream, its pool of iosurface backed 420f buffers and the picture last drawn
struct HostFrameStream {
    stream_id: u64,
    width: u32,
    height: u32,
    pool: CVPixelBufferPool,
    conversion: ConversionParams,
    reencode: Reencode,
    tables: Arc<ReencodeTables>,
    shown: Option<(u64, CVPixelBuffer)>,
    last_use: u64,
}

struct HostFrameStreams {
    streams: Vec<HostFrameStream>,
    uses: u64,
}

unsafe impl Send for HostFrameStreams {}

static HOST_FRAME_STREAMS: Mutex<HostFrameStreams> = Mutex::new(HostFrameStreams {
    streams: Vec::new(),
    uses: 0,
});

/// 420f buffer showing the frame
/// the stream last one when generation is unchanged
/// else one from the pool filled from the planes outside the lock
pub(crate) fn host_frame_buffer(frame: &HostNv12Frame) -> Result<CVPixelBuffer, SurfaceError> {
    let geometry = HostFrameGeometry::of(frame)?;
    let (buffer, reencode, tables) = {
        let mut held = HOST_FRAME_STREAMS.lock();
        held.uses += 1;
        let now = held.uses;
        let index = stream_slot(&mut held, frame)?;
        let stream = &mut held.streams[index];
        stream.last_use = now;
        if let Some((generation, buffer)) = &stream.shown
            && *generation == frame.generation
        {
            return Ok(buffer.clone());
        }
        if stream.conversion != frame.conversion {
            let (reencode, tables) = reencode_for(&frame.conversion)?;
            stream.conversion = frame.conversion;
            stream.reencode = reencode;
            stream.tables = tables;
        }
        let buffer = stream.pool.create_pixel_buffer().map_err(|status| SurfaceError::Platform {
            call: "CVPixelBufferPoolCreatePixelBuffer",
            status,
        })?;
        (buffer, stream.reencode, Arc::clone(&stream.tables))
    };
    fill_planes(&buffer, frame, &geometry, &reencode, &tables)?;
    let mut held = HOST_FRAME_STREAMS.lock();
    if let Some(stream) = held.streams.iter_mut().find(|stream| stream.holds(frame))
        && stream
            .shown
            .as_ref()
            .is_none_or(|(generation, _)| *generation < frame.generation)
    {
        stream.shown = Some((frame.generation, buffer.clone()));
    }
    Ok(buffer)
}

impl HostFrameStream {
    fn holds(&self, frame: &HostNv12Frame) -> bool {
        self.stream_id == frame.stream_id && self.width == frame.width && self.height == frame.height
    }
}

fn stream_slot(held: &mut HostFrameStreams, frame: &HostNv12Frame) -> Result<usize, SurfaceError> {
    if let Some(index) = held.streams.iter().position(|stream| stream.holds(frame)) {
        return Ok(index);
    }
    held.streams.retain(|stream| stream.stream_id != frame.stream_id);
    let (reencode, tables) = reencode_for(&frame.conversion)?;
    let pool = CVPixelBufferPool::new(None, Some(&pixel_buffer_attributes(frame.width, frame.height)))
        .map_err(|status| SurfaceError::Platform {
            call: "CVPixelBufferPoolCreate",
            status,
        })?;
    if held.streams.len() >= HELD_STREAMS
        && let Some(oldest) = held
            .streams
            .iter()
            .enumerate()
            .min_by_key(|(_, stream)| stream.last_use)
            .map(|(index, _)| index)
    {
        held.streams.swap_remove(oldest);
    }
    held.streams.push(HostFrameStream {
        stream_id: frame.stream_id,
        width: frame.width,
        height: frame.height,
        pool,
        conversion: frame.conversion,
        reencode,
        tables,
        shown: None,
        last_use: held.uses,
    });
    Ok(held.streams.len() - 1)
}

fn reencode_for(
    conversion: &ConversionParams,
) -> Result<(Reencode, Arc<ReencodeTables>), SurfaceError> {
    let reencode = Reencode::for_metal(&ycbcr_to_rgb_columns(conversion)).ok_or(
        SurfaceError::WrongKind {
            expected: "a YCbCr conversion the Metal surface matrix can express",
        },
    )?;
    Ok((reencode, Arc::new(ReencodeTables::new(&reencode))))
}

pub(crate) fn pixel_buffer_attributes(width: u32, height: u32) -> CFDictionary<CFString, CFType> {
    pool_attributes(DRAWABLE_PIXEL_FORMAT, width, height)
}

/// format and size and iosurface backing (CVMetalTextureCache needs it) and metal use
fn pool_attributes(format: u32, width: u32, height: u32) -> CFDictionary<CFString, CFType> {
    let io_surface = CFDictionary::<CFString, CFType>::from_CFType_pairs(&[]);
    CFDictionary::from_CFType_pairs(&[
        (
            CFString::from(CVPixelBufferKeys::PixelFormatType),
            CFNumber::from(i64::from(format)).as_CFType(),
        ),
        (
            CFString::from(CVPixelBufferKeys::Width),
            CFNumber::from(i64::from(width)).as_CFType(),
        ),
        (
            CFString::from(CVPixelBufferKeys::Height),
            CFNumber::from(i64::from(height)).as_CFType(),
        ),
        (
            CFString::from(CVPixelBufferKeys::IOSurfaceProperties),
            io_surface.as_CFType(),
        ),
        (
            CFString::from(CVPixelBufferKeys::MetalCompatibility),
            CFBoolean::true_value().as_CFType(),
        ),
    ])
}

/// one locked plane of a pixel buffer, rows rows with stride bytes apart
struct PlaneMut<'a> {
    bytes: &'a mut [u8],
    stride: usize,
}

fn fill_planes(
    buffer: &CVPixelBuffer,
    frame: &HostNv12Frame,
    geometry: &HostFrameGeometry,
    reencode: &Reencode,
    tables: &ReencodeTables,
) -> Result<(), SurfaceError> {
    let status = buffer.lock_base_address(0);
    if status != kCVReturnSuccess {
        return Err(SurfaceError::Platform {
            call: "CVPixelBufferLockBaseAddress",
            status,
        });
    }
    let written = unsafe { locked_plane(buffer, 0, geometry.height, geometry.width) }.and_then(
        |mut luma| {
            let mut chroma = unsafe {
                locked_plane(buffer, 1, geometry.chroma_height, 2 * geometry.chroma_width)
            }?;
            if reencode.is_identity() {
                copy_planes(frame, geometry, &mut luma, &mut chroma);
            } else {
                reencode_planes(frame, geometry, tables, &mut luma, &mut chroma);
            }
            Ok(())
        },
    );
    let status = buffer.unlock_base_address(0);
    written?;
    if status != kCVReturnSuccess {
        return Err(SurfaceError::Platform {
            call: "CVPixelBufferUnlockBaseAddress",
            status,
        });
    }
    Ok(())
}

/// plane index of a locked buffer when it holds at least rows rows of row_bytes
/// # Safety
/// buffer stays locked without the read only flag while the plane lives and no other borrow of
/// that plane exists
unsafe fn locked_plane(
    buffer: &CVPixelBuffer,
    index: usize,
    rows: usize,
    row_bytes: usize,
) -> Result<PlaneMut<'_>, SurfaceError> {
    let wrong = SurfaceError::WrongKind {
        expected: "a pool buffer with the frame's plane sizes",
    };
    let base = unsafe { buffer.get_base_address_of_plane(index) }.cast::<u8>();
    let stride = buffer.get_bytes_per_row_of_plane(index);
    let plane_rows = buffer.get_height_of_plane(index);
    if base.is_null() || plane_rows < rows || stride < row_bytes {
        return Err(wrong);
    }
    let length = stride.checked_mul(plane_rows).ok_or(wrong)?;
    let bytes = unsafe { std::slice::from_raw_parts_mut(base, length) };
    Ok(PlaneMut { bytes, stride })
}

fn copy_planes(
    frame: &HostNv12Frame,
    geometry: &HostFrameGeometry,
    luma: &mut PlaneMut<'_>,
    chroma: &mut PlaneMut<'_>,
) {
    for row in 0..geometry.height {
        let source = &frame.luma[row * geometry.luma_stride..][..geometry.width];
        luma.bytes[row * luma.stride..][..geometry.width].copy_from_slice(source);
    }
    let chroma_bytes = 2 * geometry.chroma_width;
    for row in 0..geometry.chroma_height {
        let source = &frame.chroma[row * geometry.chroma_stride..][..chroma_bytes];
        chroma.bytes[row * chroma.stride..][..chroma_bytes].copy_from_slice(source);
    }
}

fn reencode_planes(
    frame: &HostNv12Frame,
    geometry: &HostFrameGeometry,
    tables: &ReencodeTables,
    luma: &mut PlaneMut<'_>,
    chroma: &mut PlaneMut<'_>,
) {
    for row in 0..geometry.height {
        let source = &frame.luma[row * geometry.luma_stride..][..geometry.width];
        let pairs = &frame.chroma[(row / 2) * geometry.chroma_stride..][..2 * geometry.chroma_width];
        let target = &mut luma.bytes[row * luma.stride..][..geometry.width];
        for (column, (out, y)) in target.iter_mut().zip(source).enumerate() {
            let pair = (column / 2) * 2;
            *out = tables.code(0, *y, pairs[pair], pairs[pair + 1]);
        }
    }
    for row in 0..geometry.chroma_height {
        let pairs = &frame.chroma[row * geometry.chroma_stride..][..2 * geometry.chroma_width];
        let target = &mut chroma.bytes[row * chroma.stride..][..2 * geometry.chroma_width];
        for sample in 0..geometry.chroma_width {
            let mean = block_mean_luma(frame, geometry, row, sample);
            let (cb, cr) = (pairs[2 * sample], pairs[2 * sample + 1]);
            target[2 * sample] = tables.code(1, mean, cb, cr);
            target[2 * sample + 1] = tables.code(2, mean, cb, cr);
        }
    }
}

fn block_mean_luma(
    frame: &HostNv12Frame,
    geometry: &HostFrameGeometry,
    chroma_row: usize,
    sample: usize,
) -> u8 {
    let mut sum = 0_u32;
    let mut count = 0_u32;
    for row in (2 * chroma_row)..(2 * chroma_row + 2).min(geometry.height) {
        for column in (2 * sample)..(2 * sample + 2).min(geometry.width) {
            sum += u32::from(frame.luma[row * geometry.luma_stride + column]);
            count += 1;
        }
    }
    ((sum + count / 2) / count.max(1)) as u8
}

pub(crate) const HELD_BGRA_STREAMS: usize = 64;

pub(crate) const HELD_BGRA_POOLS: usize = 4;

/// iosurface backed 32bgra buffers of one picture size, shared by every stream of it
struct BgraPool {
    width: u32,
    height: u32,
    pool: CVPixelBufferPool,
    last_use: u64,
}

/// buffer a bgra stream last drew, kept so an unchanged generation needs no copy
struct BgraShown {
    stream_id: u64,
    generation: u64,
    width: u32,
    height: u32,
    buffer: CVPixelBuffer,
    last_use: u64,
}

struct HostBgraStreams {
    pools: Vec<BgraPool>,
    shown: Vec<BgraShown>,
    uses: u64,
}

unsafe impl Send for HostBgraStreams {}

static HOST_BGRA_STREAMS: Mutex<HostBgraStreams> = Mutex::new(HostBgraStreams {
    pools: Vec::new(),
    shown: Vec::new(),
    uses: 0,
});

/// 32bgra buffer showing the frame
/// the stream last one when generation and size are unchanged
/// else one from the size pool filled outside the lock
pub(crate) fn host_bgra_buffer(frame: &HostBgraFrame) -> Result<CVPixelBuffer, SurfaceError> {
    if !frame.rows_fit() {
        return Err(SurfaceError::WrongKind {
            expected: "BGRA rows as long as their width and stride need",
        });
    }
    let buffer = {
        let mut held = HOST_BGRA_STREAMS.lock();
        held.uses += 1;
        let now = held.uses;
        if let Some(shown) = held.shown.iter_mut().find(|shown| shown.shows(frame)) {
            shown.last_use = now;
            return Ok(shown.buffer.clone());
        }
        let index = bgra_pool_slot(&mut held, frame.width, frame.height)?;
        let slot = &mut held.pools[index];
        slot.last_use = now;
        slot.pool.create_pixel_buffer().map_err(|status| SurfaceError::Platform {
            call: "CVPixelBufferPoolCreatePixelBuffer",
            status,
        })?
    };
    fill_bgra(&buffer, frame)?;
    let mut held = HOST_BGRA_STREAMS.lock();
    held.uses += 1;
    let now = held.uses;
    let existing = held
        .shown
        .iter()
        .position(|shown| shown.stream_id == frame.stream_id);
    match existing {
        Some(index) => {
            let shown = &mut held.shown[index];
            let resized = (shown.width, shown.height) != (frame.width, frame.height);
            if resized || shown.generation < frame.generation {
                shown.generation = frame.generation;
                shown.width = frame.width;
                shown.height = frame.height;
                shown.buffer = buffer.clone();
            }
            shown.last_use = now;
        }
        None => {
            if held.shown.len() >= HELD_BGRA_STREAMS
                && let Some(oldest) = least_recent(held.shown.iter().map(|shown| shown.last_use))
            {
                held.shown.swap_remove(oldest);
            }
            held.shown.push(BgraShown {
                stream_id: frame.stream_id,
                generation: frame.generation,
                width: frame.width,
                height: frame.height,
                buffer: buffer.clone(),
                last_use: now,
            });
        }
    }
    Ok(buffer)
}

impl BgraShown {
    fn shows(&self, frame: &HostBgraFrame) -> bool {
        self.stream_id == frame.stream_id
            && self.generation == frame.generation
            && self.width == frame.width
            && self.height == frame.height
    }
}

/// index of the smallest use stamp, the slot given up first
pub(crate) fn least_recent(uses: impl Iterator<Item = u64>) -> Option<usize> {
    uses.enumerate()
        .min_by_key(|(_, last_use)| *last_use)
        .map(|(index, _)| index)
}

fn bgra_pool_slot(held: &mut HostBgraStreams, width: u32, height: u32) -> Result<usize, SurfaceError> {
    if let Some(index) = held
        .pools
        .iter()
        .position(|slot| slot.width == width && slot.height == height)
    {
        return Ok(index);
    }
    let pool = CVPixelBufferPool::new(None, Some(&pool_attributes(BGRA_PIXEL_FORMAT, width, height)))
        .map_err(|status| SurfaceError::Platform {
            call: "CVPixelBufferPoolCreate",
            status,
        })?;
    if held.pools.len() >= HELD_BGRA_POOLS
        && let Some(oldest) = least_recent(held.pools.iter().map(|slot| slot.last_use))
    {
        held.pools.swap_remove(oldest);
    }
    held.pools.push(BgraPool {
        width,
        height,
        pool,
        last_use: held.uses,
    });
    Ok(held.pools.len() - 1)
}

fn fill_bgra(buffer: &CVPixelBuffer, frame: &HostBgraFrame) -> Result<(), SurfaceError> {
    let status = buffer.lock_base_address(0);
    if status != kCVReturnSuccess {
        return Err(SurfaceError::Platform {
            call: "CVPixelBufferLockBaseAddress",
            status,
        });
    }
    let written = copy_bgra_rows(buffer, frame);
    let status = buffer.unlock_base_address(0);
    written?;
    if status != kCVReturnSuccess {
        return Err(SurfaceError::Platform {
            call: "CVPixelBufferUnlockBaseAddress",
            status,
        });
    }
    Ok(())
}

fn copy_bgra_rows(buffer: &CVPixelBuffer, frame: &HostBgraFrame) -> Result<(), SurfaceError> {
    let wrong = SurfaceError::WrongKind {
        expected: "a pool buffer with the frame's size",
    };
    let rows = usize::try_from(frame.height).map_err(|_| wrong.clone())?;
    let row_bytes = usize::try_from(frame.width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or_else(|| wrong.clone())?;
    let source_stride = usize::try_from(frame.stride).map_err(|_| wrong.clone())?;
    let base = unsafe { buffer.get_base_address() }.cast::<u8>();
    let target_stride = buffer.get_bytes_per_row();
    let target_rows = buffer.get_height();
    if base.is_null() || target_rows < rows || target_stride < row_bytes {
        return Err(wrong);
    }
    let length = target_stride.checked_mul(target_rows).ok_or(wrong)?;
    let target = unsafe { std::slice::from_raw_parts_mut(base, length) };
    for row in 0..rows {
        let source = &frame.pixels[row * source_stride..][..row_bytes];
        target[row * target_stride..][..row_bytes].copy_from_slice(source);
    }
    Ok(())
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn VTPixelRotationSessionCreate(
        allocator: CFAllocatorRef,
        session_out: *mut VTPixelRotationSessionRef,
    ) -> i32;
    fn VTPixelRotationSessionRotateImage(
        session: VTPixelRotationSessionRef,
        source: CVPixelBufferRef,
        destination: CVPixelBufferRef,
    ) -> i32;
    fn VTPixelRotationSessionInvalidate(session: VTPixelRotationSessionRef);
    fn VTSessionSetProperty(session: CFTypeRef, key: CFStringRef, value: CFTypeRef) -> i32;
    static kVTPixelRotationPropertyKey_Rotation: CFStringRef;
    static kVTRotation_CW90: CFStringRef;
    static kVTRotation_180: CFStringRef;
    static kVTRotation_CCW90: CFStringRef;
}

type VTPixelRotationSessionRef = *mut c_void;

/// videotoolbox rotation that turns a picture clockwise by quarter turns
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum VtRotation {
    Clockwise90,
    Half,
    CounterClockwise90,
}

impl VtRotation {
    /// none for no turn (taken modulo 4), the surface is drawn as it is
    pub(crate) const fn of_clockwise_quarter_turns(clockwise_quarter_turns: u8) -> Option<Self> {
        match clockwise_quarter_turns % 4 {
            1 => Some(Self::Clockwise90),
            2 => Some(Self::Half),
            3 => Some(Self::CounterClockwise90),
            _ => None,
        }
    }

    fn constant(self) -> CFStringRef {
        unsafe {
            match self {
                Self::Clockwise90 => kVTRotation_CW90,
                Self::Half => kVTRotation_180,
                Self::CounterClockwise90 => kVTRotation_CCW90,
            }
        }
    }
}

/// rotation session this branch created and owns, drop invalidates and releases it
struct RotationSession {
    session: VTPixelRotationSessionRef,
    rotation: Option<VtRotation>,
}

impl RotationSession {
    fn new() -> Result<Self, SurfaceError> {
        let mut session: VTPixelRotationSessionRef = std::ptr::null_mut();
        let status = unsafe { VTPixelRotationSessionCreate(std::ptr::null(), &raw mut session) };
        if status != 0 || session.is_null() {
            return Err(SurfaceError::Platform {
                call: "VTPixelRotationSessionCreate",
                status,
            });
        }
        Ok(Self {
            session,
            rotation: None,
        })
    }

    /// writes source turned by rotation into destination
    /// destination sides are already swapped for a quarter turn as the vt call requires
    fn rotate(
        &mut self,
        rotation: VtRotation,
        source: &CVPixelBuffer,
        destination: &CVPixelBuffer,
    ) -> Result<(), SurfaceError> {
        if self.rotation != Some(rotation) {
            let status = unsafe {
                VTSessionSetProperty(
                    self.session.cast_const(),
                    kVTPixelRotationPropertyKey_Rotation,
                    rotation.constant().cast(),
                )
            };
            if status != 0 {
                return Err(SurfaceError::Platform {
                    call: "VTSessionSetProperty",
                    status,
                });
            }
            self.rotation = Some(rotation);
        }
        let status = unsafe {
            VTPixelRotationSessionRotateImage(
                self.session,
                source.as_concrete_TypeRef(),
                destination.as_concrete_TypeRef(),
            )
        };
        if status != 0 {
            return Err(SurfaceError::Platform {
                call: "VTPixelRotationSessionRotateImage",
                status,
            });
        }
        Ok(())
    }
}

impl Drop for RotationSession {
    fn drop(&mut self) {
        unsafe {
            VTPixelRotationSessionInvalidate(self.session);
            CFRelease(self.session.cast_const());
        }
    }
}

pub(crate) const ROTATED_SURFACES_KEPT: usize = 8;

pub(crate) const ROTATED_POOLS: usize = 4;

/// one turned picture
/// the source is held so its pool cant reuse the buffer for a newer picture while the entry stands
/// plus the turned buffer drawn for it
struct RotatedSurface {
    source: CVPixelBuffer,
    clockwise_quarter_turns: u8,
    rotated: CVPixelBuffer,
    last_use: u64,
}

/// iosurface backed buffers of one format and turned size
struct RotatedPool {
    format: u32,
    width: u32,
    height: u32,
    pool: CVPixelBufferPool,
    last_use: u64,
}

struct RotatedSurfaces {
    session: Option<RotationSession>,
    pools: Vec<RotatedPool>,
    kept: Vec<RotatedSurface>,
    uses: u64,
}

unsafe impl Send for RotatedSurfaces {}

static ROTATED_SURFACES: Mutex<RotatedSurfaces> = Mutex::new(RotatedSurfaces {
    session: None,
    pools: Vec::new(),
    kept: Vec::new(),
    uses: 0,
});

/// held surface turned clockwise by quarter turns (modulo 4) into a buffer of its format
/// sides swapped for an odd count, no turn hands back the surface itself
pub(crate) fn rotated_buffer(
    surface: &RetainedSurface,
    clockwise_quarter_turns: u8,
) -> Result<CVPixelBuffer, SurfaceError> {
    let source = drawable_buffer(surface)?;
    let Some(rotation) = VtRotation::of_clockwise_quarter_turns(clockwise_quarter_turns) else {
        return Ok(source);
    };
    let turns = clockwise_quarter_turns % 4;
    let mut held = ROTATED_SURFACES.lock();
    held.uses += 1;
    let now = held.uses;
    let source_ref = source.as_concrete_TypeRef();
    if let Some(kept) = held.kept.iter_mut().find(|kept| {
        kept.clockwise_quarter_turns == turns && kept.source.as_concrete_TypeRef() == source_ref
    }) {
        kept.last_use = now;
        return Ok(kept.rotated.clone());
    }
    let too_large = SurfaceError::WrongKind {
        expected: "a surface whose sides fit in 32 bits",
    };
    let width = u32::try_from(source.get_width()).map_err(|_| too_large.clone())?;
    let height = u32::try_from(source.get_height()).map_err(|_| too_large)?;
    let (rotated_width, rotated_height) = rotated_size(width, height, turns);
    let index = rotated_pool_slot(&mut held, source.get_pixel_format(), rotated_width, rotated_height)?;
    let slot = &mut held.pools[index];
    slot.last_use = now;
    let destination = slot.pool.create_pixel_buffer().map_err(|status| SurfaceError::Platform {
        call: "CVPixelBufferPoolCreatePixelBuffer",
        status,
    })?;
    let mut session = match held.session.take() {
        Some(session) => session,
        None => RotationSession::new()?,
    };
    let turned = session.rotate(rotation, &source, &destination);
    held.session = Some(session);
    turned?;
    if held.kept.len() >= ROTATED_SURFACES_KEPT
        && let Some(oldest) = least_recent(held.kept.iter().map(|kept| kept.last_use))
    {
        held.kept.swap_remove(oldest);
    }
    held.kept.push(RotatedSurface {
        source,
        clockwise_quarter_turns: turns,
        rotated: destination.clone(),
        last_use: now,
    });
    Ok(destination)
}

fn rotated_pool_slot(
    held: &mut RotatedSurfaces,
    format: u32,
    width: u32,
    height: u32,
) -> Result<usize, SurfaceError> {
    if let Some(index) = held
        .pools
        .iter()
        .position(|slot| (slot.format, slot.width, slot.height) == (format, width, height))
    {
        return Ok(index);
    }
    let pool = CVPixelBufferPool::new(None, Some(&pool_attributes(format, width, height)))
        .map_err(|status| SurfaceError::Platform {
            call: "CVPixelBufferPoolCreate",
            status,
        })?;
    if held.pools.len() >= ROTATED_POOLS
        && let Some(oldest) = least_recent(held.pools.iter().map(|slot| slot.last_use))
    {
        held.pools.swap_remove(oldest);
    }
    held.pools.push(RotatedPool {
        format,
        width,
        height,
        pool,
        last_use: held.uses,
    });
    Ok(held.pools.len() - 1)
}
