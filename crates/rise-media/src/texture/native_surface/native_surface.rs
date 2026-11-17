use std::any::Any;
use std::ffi::c_void;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui::AnyElement;
use parking_lot::Mutex;
use rise_platform::HostOs;
use thiserror::Error;

use crate::texture::color::ConversionParams;

/// decoder owned gpu memory the os branch holds its own reference to
/// cloning shares the reference and dropping the last clone releases it
/// callers never learn what is inside
#[derive(Clone)]
pub struct RetainedSurface {
    address: usize,
    keep: Arc<dyn Any + Send + Sync>,
}

impl RetainedSurface {
    /// an os branch builds one after taking its own reference to the memory at address
    /// keep owns that reference and releases it on drop
    pub fn new<K: Any + Send + Sync>(address: usize, keep: K) -> Self {
        Self {
            address,
            keep: Arc::new(keep),
        }
    }

    /// surface address, for identity and logs only
    /// the memory is reached through the branch
    pub const fn address(&self) -> usize {
        self.address
    }

    /// branch own holder, when this surface was built by that branch
    pub fn keeper<K: Any>(&self) -> Option<&K> {
        self.keep.downcast_ref::<K>()
    }
}

impl PartialEq for RetainedSurface {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
    }
}

impl Eq for RetainedSurface {}

impl Hash for RetainedSurface {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.address.hash(state);
    }
}

impl fmt::Debug for RetainedSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RetainedSurface({:#x})", self.address)
    }
}

/// why a decoder surface could not be held or drawn
/// every case degrades to a cpu upload
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum SurfaceError {
    #[error("{host:?} has no platform surface path yet: {reason}")]
    NotOnThisHost { host: HostOs, reason: &'static str },
    #[error("the decoder handed over no surface")]
    Null,
    #[error("the surface is not {expected}")]
    WrongKind { expected: &'static str },
    #[error("{call} failed with status {status}")]
    Platform { call: &'static str, status: i32 },
}

/// what is missing on each host before a decoded surface can be drawn without a copy
pub const fn unwired_reason(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => "the macOS branch did not override the surface binding",
        HostOs::Linux => {
            "a VAAPI surface reaches wgpu only as a DMA-BUF (vaExportSurfaceHandle, P58), not yet imported: P27 lin step 2"
        }
        HostOs::Windows => "the Windows branch did not override the surface binding",
    }
}

/// what is missing on each host before a cpu decoded picture can be drawn
pub const fn host_frame_unwired_reason(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => {
            "the macOS branch draws a CPU picture only as the pool surface prepare_host_frame filled"
        }
        HostOs::Linux => "the Linux branch did not override the host frame binding",
        HostOs::Windows => "the Windows branch did not override the host frame binding",
    }
}

/// what is missing on each host before a cpu picture with alpha can be drawn
pub const fn bgra_frame_unwired_reason(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => {
            "the macOS branch draws a BGRA picture only as the 32BGRA pool surface prepare_host_bgra_frame filled: P148"
        }
        HostOs::Linux => "the wgpu renderer has no owned BGRA surface yet: P147",
        HostOs::Windows => "the DirectX 11 renderer has no owned BGRA surface yet: P146",
    }
}

/// what is missing on each host before a branch prepared surface can be drawn turned
pub const fn rotated_surface_unwired_reason(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => {
            "the macOS branch turns a surface it holds with VTPixelRotationSession and has no other path"
        }
        HostOs::Linux => "the Linux branch did not override the turned drawing of a decoder surface",
        HostOs::Windows => {
            "the Windows branch turns only CPU pictures; a D3D11VA slot is drawn as decoded"
        }
    }
}

/// decoded picture with alpha in cpu memory, bgra premultiplied
/// pixels holds height rows of width * 4 bytes, rows stride bytes apart
/// a webm sticker joins its colour and alpha streams into one off the main thread
#[derive(Clone)]
pub struct HostBgraFrame {
    pub stream_id: u64,
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixels: Arc<[u8]>,
}

impl HostBgraFrame {
    /// true if the size is not empty and pixels is long enough for the rows and stride
    pub fn rows_fit(&self) -> bool {
        let row = u64::from(self.width) * 4;
        let stride = u64::from(self.stride);
        self.width > 0
            && self.height > 0
            && stride >= row
            && stride * (u64::from(self.height) - 1) + row <= self.pixels.len() as u64
    }
}

impl fmt::Debug for HostBgraFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostBgraFrame")
            .field("stream_id", &self.stream_id)
            .field("generation", &self.generation)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// decoded nv12 picture in cpu memory, software decode or a hardware frame copied down
/// luma has height rows of width bytes
/// chroma has ceil(height / 2) rows of interleaved cb cr pairs, rows are stride bytes apart
#[derive(Clone)]
pub struct HostNv12Frame {
    pub stream_id: u64,
    pub generation: u64,
    pub width: u32,
    pub height: u32,
    pub luma: Arc<[u8]>,
    pub luma_stride: u32,
    pub chroma: Arc<[u8]>,
    pub chroma_stride: u32,
    pub conversion: ConversionParams,
}

impl fmt::Debug for HostNv12Frame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostNv12Frame")
            .field("stream_id", &self.stream_id)
            .field("generation", &self.generation)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// cpu picture after prepare_host_frame
/// what present_host_frame draws on the main thread without touching a pixel
#[derive(Clone, Debug)]
pub enum PreparedHostFrame {
    Planes(HostNv12Frame),
    Bgra(HostBgraFrame),
    Surface {
        stream_id: u64,
        generation: u64,
        width: u32,
        height: u32,
        surface: RetainedSurface,
    },
}

impl PreparedHostFrame {
    pub const fn stream_id(&self) -> u64 {
        match self {
            Self::Planes(frame) => frame.stream_id,
            Self::Bgra(frame) => frame.stream_id,
            Self::Surface { stream_id, .. } => *stream_id,
        }
    }

    pub const fn generation(&self) -> u64 {
        match self {
            Self::Planes(frame) => frame.generation,
            Self::Bgra(frame) => frame.generation,
            Self::Surface { generation, .. } => *generation,
        }
    }

    /// picture width and height before any turn, however it is held
    pub const fn size(&self) -> (u32, u32) {
        match self {
            Self::Planes(frame) => (frame.width, frame.height),
            Self::Bgra(frame) => (frame.width, frame.height),
            Self::Surface { width, height, .. } => (*width, *height),
        }
    }
}

/// clockwise quarter turns for a rotation in degrees (0 90 180 270), none for any other angle
pub const fn clockwise_quarter_turns_of_degrees(rotation_degrees: u16) -> Option<u8> {
    match rotation_degrees {
        0 => Some(0),
        90 => Some(1),
        180 => Some(2),
        270 => Some(3),
        _ => None,
    }
}

/// size of a width by height picture after quarter turns (modulo 4)
/// an odd count swaps the sides
pub const fn rotated_size(width: u32, height: u32, clockwise_quarter_turns: u8) -> (u32, u32) {
    if clockwise_quarter_turns % 2 == 1 {
        (height, width)
    } else {
        (width, height)
    }
}

/// stream id a turned copy of a stream draws under
/// so one scene can show it turned and as is without the renderer rebuilding textures every draw
pub const fn rotated_stream_id(stream_id: u64, clockwise_quarter_turns: u8) -> u64 {
    stream_id ^ (((clockwise_quarter_turns % 4) as u64) << 62)
}

/// turns a plane of columns by rows elements with rows stride bytes apart, clockwise into packed rows
/// none when the plane is shorter than its rows
fn rotate_plane(
    plane: &[u8],
    stride: usize,
    columns: usize,
    rows: usize,
    element: usize,
    clockwise_quarter_turns: u8,
) -> Option<Vec<u8>> {
    let row_bytes = columns.checked_mul(element)?;
    if columns == 0 || rows == 0 || stride < row_bytes {
        return None;
    }
    let needed = stride.checked_mul(rows - 1)?.checked_add(row_bytes)?;
    if plane.len() < needed {
        return None;
    }
    let turns = clockwise_quarter_turns % 4;
    let out_columns = if turns % 2 == 1 { rows } else { columns };
    let mut out = vec![0_u8; row_bytes.checked_mul(rows)?];
    for row in 0..rows {
        for column in 0..columns {
            let (to_column, to_row) = match turns {
                1 => (rows - 1 - row, column),
                2 => (columns - 1 - column, rows - 1 - row),
                3 => (row, columns - 1 - column),
                _ => (column, row),
            };
            let from = row * stride + column * element;
            let to = (to_row * out_columns + to_column) * element;
            out[to..to + element].copy_from_slice(&plane[from..from + element]);
        }
    }
    Some(out)
}

/// nv12 picture turned clockwise by quarter turns, packed rows and the turned stream id
/// none when a plane is shorter than its rows and stride need
pub fn rotate_host_nv12_frame(frame: &HostNv12Frame, clockwise_quarter_turns: u8) -> Option<HostNv12Frame> {
    let turns = clockwise_quarter_turns % 4;
    let columns = usize::try_from(frame.width).ok()?;
    let rows = usize::try_from(frame.height).ok()?;
    let luma = rotate_plane(
        &frame.luma,
        usize::try_from(frame.luma_stride).ok()?,
        columns,
        rows,
        1,
        turns,
    )?;
    let chroma = rotate_plane(
        &frame.chroma,
        usize::try_from(frame.chroma_stride).ok()?,
        columns.div_ceil(2),
        rows.div_ceil(2),
        2,
        turns,
    )?;
    let (width, height) = rotated_size(frame.width, frame.height, turns);
    Some(HostNv12Frame {
        stream_id: rotated_stream_id(frame.stream_id, turns),
        generation: frame.generation,
        width,
        height,
        luma: Arc::from(luma),
        luma_stride: width,
        chroma: Arc::from(chroma),
        chroma_stride: width.div_ceil(2) * 2,
        conversion: frame.conversion,
    })
}

/// flips a plane of columns by rows elements left to right into packed rows
/// none when the plane is shorter than its rows
fn mirror_plane(plane: &[u8], stride: usize, columns: usize, rows: usize, element: usize) -> Option<Vec<u8>> {
    let row_bytes = columns.checked_mul(element)?;
    if columns == 0 || rows == 0 || stride < row_bytes {
        return None;
    }
    let needed = stride.checked_mul(rows - 1)?.checked_add(row_bytes)?;
    if plane.len() < needed {
        return None;
    }
    let mut out = Vec::with_capacity(row_bytes.checked_mul(rows)?);
    for row in 0..rows {
        let start = row * stride;
        for pixel in plane[start..start + row_bytes].chunks_exact(element).rev() {
            out.extend_from_slice(pixel);
        }
    }
    Some(out)
}

/// nv12 picture flipped left to right with packed rows, for a viewer who sees their own front camera
/// gpui surface cant flip so the producer flips the copy it shows, the published frame stays as is
/// none when a plane is shorter than its rows and stride need
pub fn mirror_host_nv12_frame(frame: &HostNv12Frame) -> Option<HostNv12Frame> {
    let columns = usize::try_from(frame.width).ok()?;
    let rows = usize::try_from(frame.height).ok()?;
    let luma = mirror_plane(&frame.luma, usize::try_from(frame.luma_stride).ok()?, columns, rows, 1)?;
    let chroma = mirror_plane(
        &frame.chroma,
        usize::try_from(frame.chroma_stride).ok()?,
        columns.div_ceil(2),
        rows.div_ceil(2),
        2,
    )?;
    Some(HostNv12Frame {
        stream_id: frame.stream_id,
        generation: frame.generation,
        width: frame.width,
        height: frame.height,
        luma: Arc::from(luma),
        luma_stride: frame.width,
        chroma: Arc::from(chroma),
        chroma_stride: frame.width.div_ceil(2) * 2,
        conversion: frame.conversion,
    })
}

/// bgra picture turned clockwise by quarter turns, packed rows and the turned stream id
/// none when its rows dont fit
pub fn rotate_host_bgra_frame(frame: &HostBgraFrame, clockwise_quarter_turns: u8) -> Option<HostBgraFrame> {
    if !frame.rows_fit() {
        return None;
    }
    let turns = clockwise_quarter_turns % 4;
    let pixels = rotate_plane(
        &frame.pixels,
        usize::try_from(frame.stride).ok()?,
        usize::try_from(frame.width).ok()?,
        usize::try_from(frame.height).ok()?,
        4,
        turns,
    )?;
    let (width, height) = rotated_size(frame.width, frame.height, turns);
    Some(HostBgraFrame {
        stream_id: rotated_stream_id(frame.stream_id, turns),
        generation: frame.generation,
        width,
        height,
        stride: width.checked_mul(4)?,
        pixels: Arc::from(pixels),
    })
}

/// prepared cpu picture turned clockwise by quarter turns
/// a branch surface isnt a cpu picture so it comes back none, same for planes that dont fit
pub fn rotate_prepared_host_frame(frame: &PreparedHostFrame, clockwise_quarter_turns: u8) -> Option<PreparedHostFrame> {
    match frame {
        PreparedHostFrame::Planes(planes) => {
            rotate_host_nv12_frame(planes, clockwise_quarter_turns).map(PreparedHostFrame::Planes)
        }
        PreparedHostFrame::Bgra(pixels) => {
            rotate_host_bgra_frame(pixels, clockwise_quarter_turns).map(PreparedHostFrame::Bgra)
        }
        PreparedHostFrame::Surface { .. } => None,
    }
}

/// conversion as the column major 4x4 a surface shader applies to ycbcr1
/// rgb = M (yuv - offset)
/// first three columns are M and the last is -M offset
pub fn ycbcr_to_rgb_columns(params: &ConversionParams) -> [[f32; 4]; 4] {
    let matrix = &params.matrix;
    let mut columns = [[0.0_f32; 4]; 4];
    for (column, out) in columns.iter_mut().take(3).enumerate() {
        *out = [matrix[0][column], matrix[1][column], matrix[2][column], 0.0];
    }
    let mut translation = [0.0_f32, 0.0, 0.0, 1.0];
    for (channel, row) in matrix.iter().enumerate() {
        translation[channel] = -(row[0] * params.offset[0]
            + row[1] * params.offset[1]
            + row[2] * params.offset[2]);
    }
    columns[3] = translation;
    columns
}

pub trait SurfaceBinding {
    /// takes the os own reference to the decoder surface at address
    /// # Safety
    /// address is null or points at a live surface of the kind this hosts hardware decoder makes
    /// for the duration of the call
    unsafe fn retain(
        &self,
        host: HostOs,
        address: *mut c_void,
    ) -> Result<RetainedSurface, SurfaceError> {
        let _ = address;
        Err(SurfaceError::NotOnThisHost {
            host,
            reason: unwired_reason(host),
        })
    }

    /// element that draws the surface into its own bounds, scaled to fit
    fn present(&self, host: HostOs, surface: &RetainedSurface) -> Result<AnyElement, SurfaceError> {
        let _ = surface;
        Err(SurfaceError::NotOnThisHost {
            host,
            reason: unwired_reason(host),
        })
    }

    /// turns a cpu picture into what the main thread draws
    /// runs on the producer thread so every per pixel step of a branch belongs here
    /// default hands the planes to the renderer unchanged
    fn prepare_host_frame(
        &self,
        host: HostOs,
        frame: HostNv12Frame,
    ) -> Result<PreparedHostFrame, SurfaceError> {
        let _ = host;
        Ok(PreparedHostFrame::Planes(frame))
    }

    /// element that draws the planes of a cpu picture into its own bounds, scaled to fit
    fn present_host_frame(
        &self,
        host: HostOs,
        frame: &HostNv12Frame,
    ) -> Result<AnyElement, SurfaceError> {
        let _ = frame;
        Err(SurfaceError::NotOnThisHost {
            host,
            reason: host_frame_unwired_reason(host),
        })
    }

    /// turns a bgra picture into what the main thread draws, on the producer thread
    /// default checks the rows and hands them on
    fn prepare_host_bgra_frame(
        &self,
        host: HostOs,
        frame: HostBgraFrame,
    ) -> Result<PreparedHostFrame, SurfaceError> {
        let _ = host;
        if !frame.rows_fit() {
            return Err(SurfaceError::WrongKind {
                expected: "BGRA rows as long as their width and stride need",
            });
        }
        Ok(PreparedHostFrame::Bgra(frame))
    }

    fn present_host_bgra_frame(
        &self,
        host: HostOs,
        frame: &HostBgraFrame,
    ) -> Result<AnyElement, SurfaceError> {
        let _ = frame;
        Err(SurfaceError::NotOnThisHost {
            host,
            reason: bgra_frame_unwired_reason(host),
        })
    }

    /// turns a prepared surface by 1 to 3 quarter turns ahead of drawing, on the producer thread
    /// so present_rotated on the main thread finds the turn done
    /// default stages nothing
    fn prepare_rotated(
        &self,
        host: HostOs,
        surface: &RetainedSurface,
        clockwise_quarter_turns: u8,
    ) -> Result<(), SurfaceError> {
        let _ = (host, surface, clockwise_quarter_turns);
        Ok(())
    }

    /// element that draws a prepared surface turned by 1 to 3 quarter turns into its own bounds
    /// scaled to fit, cpu pictures are turned by the hub on the producer thread
    fn present_rotated(
        &self,
        host: HostOs,
        surface: &RetainedSurface,
        clockwise_quarter_turns: u8,
    ) -> Result<AnyElement, SurfaceError> {
        let _ = (surface, clockwise_quarter_turns);
        Err(SurfaceError::NotOnThisHost {
            host,
            reason: rotated_surface_unwired_reason(host),
        })
    }
}

/// holds a hardware decoder or screen capture surface past its producer own frame release
/// # Safety
/// same contract as SurfaceBinding retain
pub unsafe fn retain_decoder_surface(address: *mut c_void) -> Result<RetainedSurface, SurfaceError> {
    unsafe { super::os::NativeSurfaceBinding.retain(HostOs::current(), address) }
}

/// draws a held surface
/// an error means the frame goes through TextureImport CpuUpload
pub fn present(surface: &RetainedSurface) -> Result<AnyElement, SurfaceError> {
    super::os::NativeSurfaceBinding.present(HostOs::current(), surface)
}

/// prepares a cpu picture for drawing
/// call it where the picture is produced, never on the main thread
/// on macos it re encodes the planes into a pool surface
pub fn prepare_host_frame(frame: HostNv12Frame) -> Result<PreparedHostFrame, SurfaceError> {
    super::os::NativeSurfaceBinding.prepare_host_frame(HostOs::current(), frame)
}

/// prepares a bgra picture with premultiplied alpha for drawing
/// call it where the picture is produced, never on the main thread
pub fn prepare_host_bgra_frame(frame: HostBgraFrame) -> Result<PreparedHostFrame, SurfaceError> {
    super::os::NativeSurfaceBinding.prepare_host_bgra_frame(HostOs::current(), frame)
}

/// draws a prepared cpu picture on the main thread
/// an error means this host shows the poster frame instead of video
pub fn present_host_frame(frame: &PreparedHostFrame) -> Result<AnyElement, SurfaceError> {
    let host = HostOs::current();
    match frame {
        PreparedHostFrame::Planes(planes) => {
            super::os::NativeSurfaceBinding.present_host_frame(host, planes)
        }
        PreparedHostFrame::Bgra(pixels) => {
            super::os::NativeSurfaceBinding.present_host_bgra_frame(host, pixels)
        }
        PreparedHostFrame::Surface { surface, .. } => {
            super::os::NativeSurfaceBinding.present(host, surface)
        }
    }
}

static ROTATED: Mutex<Vec<(u64, u8, PreparedHostFrame)>> = Mutex::new(Vec::new());

const ROTATED_KEPT: usize = 8;

const PLANES_FIT: &str = "planes as long as their rows and strides need";

const NOT_STAGED: &str = "a picture turned where it was produced (prepare_host_frame_rotation)";

fn quarter_turns(rotation_degrees: u16) -> Result<u8, SurfaceError> {
    clockwise_quarter_turns_of_degrees(rotation_degrees).ok_or(SurfaceError::WrongKind {
        expected: "a rotation of 0, 90, 180 or 270 degrees",
    })
}

/// turns a prepared picture by rotation_degrees (0 90 180 270)
/// keeps it for present_host_frame_rotated, call it right after prepare_host_frame
/// never on the main thread, cpu pictures are turned pixel by pixel here
pub fn prepare_host_frame_rotation(frame: &PreparedHostFrame, rotation_degrees: u16) -> Result<(), SurfaceError> {
    let turns = quarter_turns(rotation_degrees)?;
    if turns == 0 {
        return Ok(());
    }
    if let PreparedHostFrame::Surface { surface, .. } = frame {
        return super::os::NativeSurfaceBinding.prepare_rotated(HostOs::current(), surface, turns);
    }
    if staged_rotation(frame, turns).is_some() {
        return Ok(());
    }
    let rotated = rotate_prepared_host_frame(frame, turns).ok_or(SurfaceError::WrongKind { expected: PLANES_FIT })?;
    let (stream_id, generation) = (frame.stream_id(), frame.generation());
    let mut kept = ROTATED.lock();
    if let Some(at) = kept.iter().position(|(kept_stream, kept_turns, _)| (*kept_stream, *kept_turns) == (stream_id, turns)) {
        if kept[at].2.generation() >= generation {
            return Ok(());
        }
        kept.remove(at);
    } else if kept.len() >= ROTATED_KEPT {
        kept.remove(0);
    }
    kept.push((stream_id, turns, rotated));
    Ok(())
}

/// turned copy of the frame that prepare_host_frame_rotation staged, found with no pixel work
/// its own generation or a newer one of the same stream
/// none when nothing that new was staged
pub fn staged_rotation(frame: &PreparedHostFrame, clockwise_quarter_turns: u8) -> Option<PreparedHostFrame> {
    let turns = clockwise_quarter_turns % 4;
    let (stream_id, generation) = (frame.stream_id(), frame.generation());
    ROTATED
        .lock()
        .iter()
        .find(|(kept_stream, kept_turns, staged)| {
            *kept_stream == stream_id && *kept_turns == turns && staged.generation() >= generation
        })
        .map(|(_, _, staged)| staged.clone())
}

/// draws a prepared picture turned clockwise by rotation_degrees (0 90 180 270)
/// 90 and 270 swap width and height, 0 is present_host_frame
/// main thread, cpu picture must be staged by prepare_host_frame_rotation or its an error
pub fn present_host_frame_rotated(frame: &PreparedHostFrame, rotation_degrees: u16) -> Result<AnyElement, SurfaceError> {
    let turns = quarter_turns(rotation_degrees)?;
    if turns == 0 {
        return present_host_frame(frame);
    }
    if let PreparedHostFrame::Surface { surface, .. } = frame {
        return super::os::NativeSurfaceBinding.present_rotated(HostOs::current(), surface, turns);
    }
    let rotated = staged_rotation(frame, turns).ok_or(SurfaceError::WrongKind { expected: NOT_STAGED })?;
    present_host_frame(&rotated)
}
