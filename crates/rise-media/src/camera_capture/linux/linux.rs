use std::ffi::{c_int, c_ulong, c_void, CString};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;

use crate::camera_capture::camera_capture::*;
use crate::camera_capture::v4l2_device::*;
use crate::texture::frame::{FrameFormat, FrameGeometry};

pub(crate) struct NativeCamera;

const SYSFS_NODES: &str = "/sys/class/video4linux";
const BUFFER_COUNT: u32 = 4;
const POLL_TIMEOUT_MS: c_int = 200;

const O_RDWR: c_int = 0o2;
const O_NONBLOCK: c_int = 0o4000;
const O_CLOEXEC: c_int = 0o2000000;
const PROT_READ: c_int = 1;
const PROT_WRITE: c_int = 2;
const MAP_SHARED: c_int = 1;
const POLLIN: i16 = 1;
const EINTR: i32 = 4;
const ENOENT: i32 = 2;
const EAGAIN: i32 = 11;
const EACCES: i32 = 13;
const EPERM: i32 = 1;
const EBUSY: i32 = 16;
const ENODEV: i32 = 19;
const EINVAL: i32 = 22;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn open(path: *const std::ffi::c_char, flags: c_int, ...) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn poll(fds: *mut PollFd, count: c_ulong, timeout: c_int) -> c_int;
}

#[allow(dead_code)]
#[repr(C)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[allow(dead_code)]
#[repr(C)]
struct FmtDesc {
    index: u32,
    kind: u32,
    flags: u32,
    description: [u8; 32],
    pixel_format: u32,
    mbus_code: u32,
    reserved: [u32; 3],
}

#[allow(dead_code)]
#[repr(C)]
struct FrmSizeEnum {
    index: u32,
    pixel_format: u32,
    kind: u32,
    data: [u32; 6],
    reserved: [u32; 2],
}

#[allow(dead_code)]
#[repr(C)]
struct FrmIvalEnum {
    index: u32,
    pixel_format: u32,
    width: u32,
    height: u32,
    kind: u32,
    data: [u32; 6],
    reserved: [u32; 2],
}

#[allow(dead_code)]
#[repr(C)]
struct PixFormat {
    width: u32,
    height: u32,
    pixel_format: u32,
    field: u32,
    bytes_per_line: u32,
    size_image: u32,
    colorspace: u32,
    private: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[allow(dead_code)]
#[repr(C)]
struct Format {
    kind: u32,
    padding: u32,
    pix: PixFormat,
    rest: [u8; 152],
}

#[allow(dead_code)]
#[repr(C)]
struct StreamParm {
    kind: u32,
    capability: u32,
    capture_mode: u32,
    numerator: u32,
    denominator: u32,
    extended_mode: u32,
    read_buffers: u32,
    reserved: [u32; 4],
    rest: [u8; 160],
}

#[allow(dead_code)]
#[repr(C)]
struct RequestBuffers {
    count: u32,
    kind: u32,
    memory: u32,
    capabilities: u32,
    flags: u8,
    reserved: [u8; 3],
}

#[allow(dead_code)]
#[repr(C)]
struct Buffer {
    index: u32,
    kind: u32,
    bytes_used: u32,
    flags: u32,
    field: u32,
    timestamp: [i64; 2],
    timecode: [u32; 4],
    sequence: u32,
    memory: u32,
    m: [u32; 2],
    length: u32,
    reserved2: u32,
    request_fd: i32,
}

#[repr(C)]
struct Control {
    id: u32,
    value: i32,
}

const _: () = assert!(size_of::<Capability>() == 104);
const _: () = assert!(size_of::<FmtDesc>() == 64);
const _: () = assert!(size_of::<FrmSizeEnum>() == 44);
const _: () = assert!(size_of::<FrmIvalEnum>() == 52);
const _: () = assert!(size_of::<Format>() == 208);
const _: () = assert!(size_of::<StreamParm>() == 204);
const _: () = assert!(size_of::<RequestBuffers>() == 20);
const _: () = assert!(size_of::<Buffer>() == 88);

const VIDIOC_QUERYCAP: u64 = ioc(true, false, 0, size_of::<Capability>());
const VIDIOC_ENUM_FMT: u64 = ioc(true, true, 2, size_of::<FmtDesc>());
const VIDIOC_S_FMT: u64 = ioc(true, true, 5, size_of::<Format>());
const VIDIOC_REQBUFS: u64 = ioc(true, true, 8, size_of::<RequestBuffers>());
const VIDIOC_QUERYBUF: u64 = ioc(true, true, 9, size_of::<Buffer>());
const VIDIOC_QBUF: u64 = ioc(true, true, 15, size_of::<Buffer>());
const VIDIOC_DQBUF: u64 = ioc(true, true, 17, size_of::<Buffer>());
const VIDIOC_STREAMON: u64 = ioc(false, true, 18, size_of::<c_int>());
const VIDIOC_STREAMOFF: u64 = ioc(false, true, 19, size_of::<c_int>());
const VIDIOC_S_PARM: u64 = ioc(true, true, 22, size_of::<StreamParm>());
const VIDIOC_G_CTRL: u64 = ioc(true, true, 27, size_of::<Control>());
const VIDIOC_ENUM_FRAMESIZES: u64 = ioc(true, true, 74, size_of::<FrmSizeEnum>());
const VIDIOC_ENUM_FRAMEINTERVALS: u64 = ioc(true, true, 75, size_of::<FrmIvalEnum>());

fn zeroed<T>() -> T {
    unsafe { std::mem::zeroed() }
}

fn xioctl<T>(fd: c_int, request: u64, argument: &mut T) -> io::Result<()> {
    loop {
        let result = unsafe { ioctl(fd, request as c_ulong, argument as *mut T) };
        if result != -1 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(EINTR) {
            return Err(error);
        }
    }
}

fn open_node(path: &str) -> Result<OwnedFd, CameraCaptureError> {
    let c_path = CString::new(path).map_err(|_| CameraCaptureError::OpenFailed(path.to_owned()))?;
    let fd = unsafe { open(c_path.as_ptr(), O_RDWR | O_NONBLOCK | O_CLOEXEC) };
    if fd < 0 {
        let error = io::Error::last_os_error();
        return Err(match error.raw_os_error() {
            Some(EBUSY) => CameraCaptureError::Busy,
            Some(EACCES) | Some(EPERM) => CameraCaptureError::AccessDenied,
            Some(ENOENT) | Some(ENODEV) => CameraCaptureError::NoCamera,
            _ => CameraCaptureError::OpenFailed(error.to_string()),
        });
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn supported_encodings(fd: c_int) -> Vec<(u32, PixelEncoding)> {
    let mut found = Vec::new();
    for index in 0.. {
        let mut desc: FmtDesc = zeroed();
        desc.index = index;
        desc.kind = BUF_TYPE_VIDEO_CAPTURE;
        if xioctl(fd, VIDIOC_ENUM_FMT, &mut desc).is_err() {
            break;
        }
        if let Some(encoding) = encoding_of(desc.pixel_format) {
            found.push((desc.pixel_format, encoding));
        }
    }
    found
}

fn intervals(fd: c_int, pixel_format: u32, width: u32, height: u32) -> Vec<(u32, u32)> {
    let mut found = Vec::new();
    for index in 0.. {
        let mut interval: FrmIvalEnum = zeroed();
        interval.index = index;
        interval.pixel_format = pixel_format;
        interval.width = width;
        interval.height = height;
        if xioctl(fd, VIDIOC_ENUM_FRAMEINTERVALS, &mut interval).is_err() {
            break;
        }
        if interval.kind == FRMIVAL_TYPE_DISCRETE {
            found.push((interval.data[0], interval.data[1]));
        } else {
            let min = (interval.data[0], interval.data[1]);
            let max = (interval.data[2], interval.data[3]);
            found.push(stepwise_interval(min, max, MINIMUM_FRAME_RATE));
            break;
        }
    }
    found
}

fn modes(fd: c_int) -> Vec<CameraMode> {
    let mut modes = Vec::new();
    for (pixel_format, encoding) in supported_encodings(fd) {
        let mut sizes = Vec::new();
        for index in 0.. {
            let mut size: FrmSizeEnum = zeroed();
            size.index = index;
            size.pixel_format = pixel_format;
            if xioctl(fd, VIDIOC_ENUM_FRAMESIZES, &mut size).is_err() {
                break;
            }
            if size.kind == FRMSIZE_TYPE_DISCRETE {
                sizes.push((size.data[0], size.data[1]));
            } else {
                let d = size.data;
                sizes.extend(stepwise_sizes((d[0], d[3]), (d[1], d[4]), (d[2], d[5])));
                break;
            }
        }
        for (width, height) in sizes {
            let found = intervals(fd, pixel_format, width, height);
            if found.is_empty() {
                modes.push(mode_of(encoding, width, height, 0, 0));
            }
            for (numerator, denominator) in found {
                modes.push(mode_of(encoding, width, height, numerator, denominator));
            }
        }
    }
    modes
}

fn read_attribute(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn describe(node: &str) -> Option<CameraDevice> {
    let path = format!("/dev/{node}");
    let fd = open_node(&path).ok()?;
    let mut capability: Capability = zeroed();
    xioctl(fd.as_raw_fd(), VIDIOC_QUERYCAP, &mut capability).ok()?;
    if !is_streaming_camera(capability.capabilities, capability.device_caps) {
        return None;
    }
    if supported_encodings(fd.as_raw_fd()).is_empty() {
        return None;
    }
    let mut control = Control { id: CID_CAMERA_ORIENTATION, value: 0 };
    let orientation = xioctl(fd.as_raw_fd(), VIDIOC_G_CTRL, &mut control).ok().map(|()| control.value);
    let sysfs = Path::new(SYSFS_NODES).join(node);
    let name = read_attribute(&sysfs.join("name"));
    let removable = read_attribute(&sysfs.join("device/../removable"));
    let (position, built_in) = position_of(orientation, removable.as_deref());
    Some(CameraDevice {
        id: path,
        name: display_name(name.as_deref(), &capability.card),
        position,
        built_in,
    })
}

fn in_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

fn enumerate() -> Result<Vec<CameraDevice>, CameraCaptureError> {
    let mut nodes: Vec<(u32, String)> = match fs::read_dir(SYSFS_NODES) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| node_number(&name).map(|number| (number, name)))
            .collect(),
        Err(_) => Vec::new(),
    };
    nodes.sort();
    let devices: Vec<CameraDevice> = nodes.iter().filter_map(|(_, node)| describe(node)).collect();
    if devices.is_empty() && in_flatpak() {
        return Err(CameraCaptureError::Unavailable(
            "inside Flatpak the cameras are reached through the Camera portal and PipeWire, which \
             this build does not link yet"
                .to_owned(),
        ));
    }
    Ok(devices)
}

impl CameraSource for NativeCamera {
    fn support(&self) -> Result<(), CameraCaptureError> {
        if enumerate()?.is_empty() { Err(CameraCaptureError::NoCamera) } else { Ok(()) }
    }

    fn recording_support(&self) -> Result<(), CameraCaptureError> {
        Err(encoder_unavailable())
    }

    fn devices(&self) -> Result<Vec<CameraDevice>, CameraCaptureError> {
        enumerate()
    }

    fn open(
        &self,
        device: &CameraDevice,
        frames: FrameSink,
    ) -> Result<Box<dyn CameraSession>, CameraCaptureError> {
        Ok(Box::new(LinuxCameraSession::start(device.clone(), frames)?))
    }
}

fn encoder_unavailable() -> CameraCaptureError {
    CameraCaptureError::EncoderUnavailable(
        "the LGPL FFmpeg build has no H.264 encoder and no MP4 muxer".to_owned(),
    )
}

struct Mapping {
    pointer: *mut u8,
    length: usize,
}

unsafe impl Send for Mapping {}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            munmap(self.pointer.cast(), self.length);
        }
    }
}

struct Negotiated {
    mode: CameraMode,
    width: u32,
    height: u32,
    stride: u32,
}

fn negotiate(fd: c_int) -> Result<Negotiated, CameraCaptureError> {
    let mode = choose_mode(&modes(fd)).ok_or_else(|| {
        CameraCaptureError::OpenFailed("the camera offers no YUYV, NV12 or MJPEG mode".to_owned())
    })?;
    let pixel_format = pixel_format_of(mode.encoding)
        .ok_or_else(|| CameraCaptureError::OpenFailed("unexpected encoding".to_owned()))?;
    let mut format: Format = zeroed();
    format.kind = BUF_TYPE_VIDEO_CAPTURE;
    format.pix.width = mode.width;
    format.pix.height = mode.height;
    format.pix.pixel_format = pixel_format;
    format.pix.field = FIELD_NONE;
    xioctl(fd, VIDIOC_S_FMT, &mut format).map_err(|error| open_error(&error))?;
    let pixel_format = format.pix.pixel_format;
    let encoding = encoding_of(pixel_format)
        .ok_or_else(|| CameraCaptureError::OpenFailed("the driver changed the pixel format".to_owned()))?;
    if mode.interval_numerator != 0 {
        let mut parm: StreamParm = zeroed();
        parm.kind = BUF_TYPE_VIDEO_CAPTURE;
        parm.numerator = mode.interval_numerator;
        parm.denominator = mode.interval_denominator;
        let _ = xioctl(fd, VIDIOC_S_PARM, &mut parm);
    }
    let width = format.pix.width;
    let height = format.pix.height;
    let minimum_stride = match encoding {
        PixelEncoding::Yuyv => width * 2,
        _ => width,
    };
    Ok(Negotiated {
        mode: CameraMode { encoding, width, height, ..mode },
        width,
        height,
        stride: format.pix.bytes_per_line.max(minimum_stride),
    })
}

fn open_error(error: &io::Error) -> CameraCaptureError {
    match error.raw_os_error() {
        Some(EBUSY) => CameraCaptureError::Busy,
        Some(ENODEV) => CameraCaptureError::NoCamera,
        _ => CameraCaptureError::OpenFailed(error.to_string()),
    }
}

fn map_buffers(fd: c_int) -> Result<Vec<Mapping>, CameraCaptureError> {
    let mut request: RequestBuffers = zeroed();
    request.count = BUFFER_COUNT;
    request.kind = BUF_TYPE_VIDEO_CAPTURE;
    request.memory = MEMORY_MMAP;
    xioctl(fd, VIDIOC_REQBUFS, &mut request).map_err(|error| open_error(&error))?;
    if request.count == 0 {
        return Err(CameraCaptureError::OpenFailed("the driver gave no buffers".to_owned()));
    }
    let mut mappings = Vec::with_capacity(request.count as usize);
    for index in 0..request.count {
        let mut buffer: Buffer = zeroed();
        buffer.index = index;
        buffer.kind = BUF_TYPE_VIDEO_CAPTURE;
        buffer.memory = MEMORY_MMAP;
        xioctl(fd, VIDIOC_QUERYBUF, &mut buffer).map_err(|error| open_error(&error))?;
        let length = buffer.length as usize;
        let pointer = unsafe {
            mmap(std::ptr::null_mut(), length, PROT_READ | PROT_WRITE, MAP_SHARED, fd, i64::from(buffer.m[0]))
        };
        if pointer as isize == -1 {
            return Err(CameraCaptureError::OpenFailed(io::Error::last_os_error().to_string()));
        }
        mappings.push(Mapping { pointer: pointer.cast(), length });
        xioctl(fd, VIDIOC_QBUF, &mut buffer).map_err(|error| open_error(&error))?;
    }
    Ok(mappings)
}

struct LinuxCameraSession {
    device: CameraDevice,
    mode: CameraMode,
    stop: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<CameraCaptureError>>>,
    thread: Option<JoinHandle<()>>,
}

impl LinuxCameraSession {
    fn start(device: CameraDevice, frames: FrameSink) -> Result<Self, CameraCaptureError> {
        let fd = open_node(&device.id)?;
        let negotiated = negotiate(fd.as_raw_fd())?;
        let mappings = map_buffers(fd.as_raw_fd())?;
        let mut kind = BUF_TYPE_VIDEO_CAPTURE as c_int;
        xioctl(fd.as_raw_fd(), VIDIOC_STREAMON, &mut kind).map_err(|error| open_error(&error))?;
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        let mode = negotiated.mode;
        let mirrored = device.position == CameraPosition::Front || device.built_in;
        let worker = Worker {
            fd,
            mappings,
            format: negotiated,
            frames,
            mirrored,
            stop: stop.clone(),
            failure: failure.clone(),
            scratch: Vec::new(),
        };
        let thread = std::thread::Builder::new()
            .name("camera-capture".to_owned())
            .spawn(move || worker.run())
            .map_err(|error| CameraCaptureError::OpenFailed(error.to_string()))?;
        Ok(Self { device, mode, stop, failure, thread: Some(thread) })
    }
}

impl CameraSession for LinuxCameraSession {
    fn device(&self) -> &CameraDevice {
        &self.device
    }

    fn mode(&self) -> CameraMode {
        self.mode
    }

    fn start_segment(&mut self, _path: &Path) -> Result<(), CameraCaptureError> {
        Err(encoder_unavailable())
    }

    fn stop_segment(&mut self) -> Result<(), CameraCaptureError> {
        Err(CameraCaptureError::NotRecording)
    }

    fn failure(&self) -> Option<CameraCaptureError> {
        self.failure.lock().clone()
    }
}

impl Drop for LinuxCameraSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Worker {
    fd: OwnedFd,
    mappings: Vec<Mapping>,
    format: Negotiated,
    frames: FrameSink,
    mirrored: bool,
    stop: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<CameraCaptureError>>>,
    scratch: Vec<u8>,
}

impl Worker {
    fn run(mut self) {
        let result = self.pump();
        let mut kind = BUF_TYPE_VIDEO_CAPTURE as c_int;
        let _ = xioctl(self.fd.as_raw_fd(), VIDIOC_STREAMOFF, &mut kind);
        if let Err(error) = result {
            tracing::warn!(%error, "camera capture stopped");
            *self.failure.lock() = Some(error);
        }
        self.mappings.clear();
    }

    fn pump(&mut self) -> Result<(), CameraCaptureError> {
        let fd = self.fd.as_raw_fd();
        while !self.stop.load(Ordering::Acquire) {
            let mut poll_fd = PollFd { fd, events: POLLIN, revents: 0 };
            let ready = unsafe { poll(&mut poll_fd, 1, POLL_TIMEOUT_MS) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(EINTR) {
                    continue;
                }
                return Err(CameraCaptureError::StreamFailed(error.to_string()));
            }
            if ready == 0 {
                continue;
            }
            let mut buffer: Buffer = zeroed();
            buffer.kind = BUF_TYPE_VIDEO_CAPTURE;
            buffer.memory = MEMORY_MMAP;
            match xioctl(fd, VIDIOC_DQBUF, &mut buffer) {
                Ok(()) => {}
                Err(error) if error.raw_os_error() == Some(EAGAIN) => continue,
                Err(error) if error.raw_os_error() == Some(ENODEV) => return Err(CameraCaptureError::NoCamera),
                Err(error) => return Err(CameraCaptureError::StreamFailed(error.to_string())),
            }
            let captured_at = RiseClock::monotonic();
            let frame = self.convert(buffer.index as usize, buffer.bytes_used as usize, captured_at);
            if let Err(error) = xioctl(fd, VIDIOC_QBUF, &mut buffer) {
                if error.raw_os_error() != Some(EINVAL) {
                    return Err(CameraCaptureError::StreamFailed(error.to_string()));
                }
            }
            match frame {
                Ok(frame) => (self.frames)(frame),
                Err(error) => tracing::debug!(%error, "camera frame dropped"),
            }
        }
        Ok(())
    }

    fn convert(&mut self, index: usize, used: usize, captured_at: Duration) -> Result<CameraFrame, CameraCaptureError> {
        let mapping = self.mappings.get(index).ok_or(CameraCaptureError::BadFrame("buffer index"))?;
        let used = used.min(mapping.length);
        let bytes = unsafe { std::slice::from_raw_parts(mapping.pointer, used) };
        let Negotiated { mode, width, height, stride, .. } = self.format;
        match mode.encoding {
            PixelEncoding::Yuyv => {
                yuyv_to_bgra(bytes, width, height, stride, &mut self.scratch)?;
                bgra_frame(std::mem::take(&mut self.scratch), width, height, captured_at, self.mirrored)
            }
            PixelEncoding::Nv12 => {
                let geometry = FrameGeometry::with_strides(FrameFormat::Nv12, width, height, &[stride, stride])
                    .map_err(|_| CameraCaptureError::BadFrame("NV12 geometry"))?;
                if (bytes.len() as u64) < geometry.byte_len() {
                    return Err(CameraCaptureError::BadFrame("NV12 frame is shorter than its geometry"));
                }
                Ok(CameraFrame { geometry, pixels: bytes.into(), captured_at, mirrored: self.mirrored })
            }
            PixelEncoding::Mjpeg => decode_jpeg(bytes.to_vec(), captured_at, self.mirrored),
            PixelEncoding::Bgra => Err(CameraCaptureError::BadFrame("V4L2 BGRA is not negotiated")),
        }
    }
}

fn decode_jpeg(bytes: Vec<u8>, captured_at: Duration, mirrored: bool) -> Result<CameraFrame, CameraCaptureError> {
    let image = gpui::Image::from_bytes(gpui::ImageFormat::Jpeg, bytes);
    let render = image
        .to_image_data(gpui::SvgRenderer::new(Arc::new(())))
        .map_err(|_| CameraCaptureError::BadFrame("MJPEG frame does not decode"))?;
    let size = render.size(0);
    let pixels = render.as_bytes(0).ok_or(CameraCaptureError::BadFrame("MJPEG frame is empty"))?;
    let width = u32::try_from(size.width.0).unwrap_or(0);
    let height = u32::try_from(size.height.0).unwrap_or(0);
    bgra_frame(pixels.to_vec(), width, height, captured_at, mirrored)
}
