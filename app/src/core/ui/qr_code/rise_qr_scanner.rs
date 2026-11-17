use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Task};
use image::DynamicImage;
use image::imageops::FilterType;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::FrameFormat;
use rise_media::camera_capture::{CameraCaptureError, CameraFrame, FrameSink, VideoNoteCamera};
use rise_platform::capture_permission::{self, CaptureDecision, CaptureKind};
use rise_platform::paths::temporary_directory;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::push_notifications::push_deep_link_store::PushDeepLink;
use crate::core::push_notifications::universal_link_parser::UniversalLinkParser;
use crate::modules::chat::components::chat::video_note::video_note_recorder::VideoNoteCaptureSession;

/// scanned string becomes a url and goes through the same universal link parser
/// used for universal links so the route table isnt duplicated here
pub enum RiseQRScanResolver {}

impl RiseQRScanResolver {
    const MAXIMUM_PAYLOAD_BYTES: usize = 2048;
    const MAXIMUM_SCAN_SIDE: u32 = 1600;

    pub fn deep_link(raw_value: &str, environment: AppEnvironment) -> Option<PushDeepLink> {
        let value = trimming_whitespaces_and_newlines(raw_value);
        if value.is_empty() || value.len() > Self::MAXIMUM_PAYLOAD_BYTES {
            return None;
        }

        let mut candidates = vec![value.to_owned()];
        if !value.contains("://") {
            candidates.push(format!("https://{value}"));
        }
        candidates.iter().find_map(|candidate| {
            let url = Url::from_string(candidate)?;
            UniversalLinkParser::parse(&url, environment)
        })
    }

    /// desktop has no vision or detector, rqrr reads the code from the luminance of the picture
    /// runs off the main thread
    pub fn payload(image: &DynamicImage) -> Option<String> {
        let image = if image.width().max(image.height()) > Self::MAXIMUM_SCAN_SIDE {
            image.resize(Self::MAXIMUM_SCAN_SIDE, Self::MAXIMUM_SCAN_SIDE, FilterType::Triangle)
        } else {
            image.clone()
        };
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let found = Self::payload_in_luma(width as usize, height as usize, |x, y| {
            let [red, green, blue, alpha] = rgba.get_pixel(x as u32, y as u32).0;
            let luma = (u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114) / 1000;
            let alpha = u32::from(alpha);
            ((luma * alpha + 255 * (255 - alpha)) / 255) as u8
        });
        tracing::warn!(
            target: "riseonly::X930",
            width,
            height,
            pass = found.as_ref().map_or("none", |(_, pass)| *pass),
            "qr picture scan"
        );
        found.map(|(payload, _)| payload)
    }

    pub fn frame_payload(frame: &CameraFrame) -> Option<String> {
        let geometry = &frame.geometry;
        let plane = geometry.planes().first()?;
        let width = geometry.width() as usize;
        let height = geometry.height() as usize;
        let stride = plane.stride as usize;
        let offset = usize::try_from(plane.offset).ok()?;
        let pixels: &[u8] = &frame.pixels;
        let last_row_end = offset
            .checked_add(stride.checked_mul(height.checked_sub(1)?)?)?
            .checked_add(width.checked_mul(plane.bytes_per_sample as usize)?)?;
        if pixels.len() < last_row_end {
            return None;
        }
        let found = match geometry.format() {
            FrameFormat::Bgra8 => Self::payload_in_luma(width, height, |x, y| {
                let index = offset + y * stride + x * 4;
                let blue = u32::from(pixels[index]);
                let green = u32::from(pixels[index + 1]);
                let red = u32::from(pixels[index + 2]);
                ((red * 299 + green * 587 + blue * 114) / 1000) as u8
            }),
            FrameFormat::Nv12 | FrameFormat::I420 => {
                Self::payload_in_luma(width, height, |x, y| pixels[offset + y * stride + x])
            }
            FrameFormat::P010 => Self::payload_in_luma(width, height, |x, y| pixels[offset + y * stride + x * 2 + 1]),
        };
        found.map(|(payload, _)| payload)
    }

    /// rqrr alone reads only plain codes, styled ones like dot modules or a logo need more passes
    /// tries the picture as is then binarized by otsu then sampled on the grid between finders
    /// then dark modules grown until dots touch and inverted, second value names the pass that read it
    fn payload_in_luma(
        width: usize,
        height: usize,
        mut luma: impl FnMut(usize, usize) -> u8,
    ) -> Option<(String, &'static str)> {
        if width == 0 || height == 0 {
            return None;
        }
        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                pixels.push(luma(x, y));
            }
        }
        if let Some(payload) = Self::decode_pixels(width, height, &pixels) {
            return Some((payload, "raw"));
        }
        let threshold = Self::otsu_threshold(&pixels);
        let binary: Vec<u8> = pixels.iter().map(|&value| if value <= threshold { 0 } else { 255 }).collect();
        if let Some(payload) = Self::decode_pixels(width, height, &binary) {
            return Some((payload, "otsu"));
        }
        if let Some(payload) = Self::payload_in_modules(width, height, &binary) {
            return Some((payload, "modules"));
        }
        let step = (width.max(height) / 500).max(1);
        for radius in [step, step * 2] {
            let grown = Self::grow_dark(width, height, &binary, radius);
            if let Some(payload) = Self::decode_pixels(width, height, &grown) {
                return Some((payload, "grown"));
            }
        }
        let inverted: Vec<u8> = binary.iter().map(|&value| 255 - value).collect();
        if let Some(payload) = Self::decode_pixels(width, height, &inverted) {
            return Some((payload, "inverted"));
        }
        Self::payload_in_modules(width, height, &inverted).map(|payload| (payload, "inverted modules"))
    }

    /// fallback for flat pictures like a saved card or a screenshot
    /// rounded finders pull rqrr corners inward so its timing scan and grid size come out wrong
    /// this finds finders by their dark region, samples the straight grid and hands bits to rqrr
    fn payload_in_modules(width: usize, height: usize, binary: &[u8]) -> Option<String> {
        let finders = Self::finders(width, height, binary);
        if finders.len() < 3 || finders.len() > Self::MAXIMUM_FINDERS {
            return None;
        }
        for (corner_index, corner) in finders.iter().enumerate() {
            for (right_index, right) in finders.iter().enumerate() {
                for (below_index, below) in finders.iter().enumerate() {
                    if corner_index == right_index || corner_index == below_index || right_index == below_index {
                        continue;
                    }
                    if let Some(payload) = Self::sample_grid(width, height, binary, *corner, *right, *below) {
                        return Some(payload);
                    }
                }
            }
        }
        None
    }

    const MAXIMUM_FINDERS: usize = 12;

    fn finders(width: usize, height: usize, binary: &[u8]) -> Vec<QrFinder> {
        let (labels, regions) = Self::dark_regions(width, height, binary);
        let mut finders = Vec::new();
        for (index, ring) in regions.iter().enumerate() {
            let (ring_width, ring_height) = (ring.width(), ring.height());
            if ring_width < 7.0 || (ring_width - ring_height).abs() > (ring_width / 5.0).max(2.0) {
                continue;
            }
            let (centre_x, centre_y) = ring.centre();
            let centre = centre_y.round() as usize * width + centre_x.round() as usize;
            let Some(&stone_label) = labels.get(centre) else {
                continue;
            };
            if stone_label == u32::MAX || stone_label as usize == index {
                continue;
            }
            let Some(stone) = regions.get(stone_label as usize) else {
                continue;
            };
            let (stone_x, stone_y) = stone.centre();
            let centred =
                (stone_x - centre_x).abs() <= ring_width * 0.1 && (stone_y - centre_y).abs() <= ring_height * 0.1;
            let sized = (0.25..=0.6).contains(&(stone.width() / ring_width))
                && (0.25..=0.6).contains(&(stone.height() / ring_height));
            if centred && sized {
                finders.push(QrFinder { x: centre_x, y: centre_y, module: (ring_width + ring_height) / 14.0 });
            }
        }
        finders
    }

    fn dark_regions(width: usize, height: usize, binary: &[u8]) -> (Vec<u32>, Vec<DarkRegion>) {
        let mut labels = vec![u32::MAX; binary.len()];
        let mut regions: Vec<DarkRegion> = Vec::new();
        let mut stack = Vec::new();
        for start in 0..binary.len() {
            if binary[start] != 0 || labels[start] != u32::MAX {
                continue;
            }
            let label = regions.len() as u32;
            labels[start] = label;
            stack.push(start);
            let mut region =
                DarkRegion { min_x: start % width, min_y: start / width, max_x: start % width, max_y: start / width };
            while let Some(index) = stack.pop() {
                let (x, y) = (index % width, index / width);
                region.min_x = region.min_x.min(x);
                region.max_x = region.max_x.max(x);
                region.min_y = region.min_y.min(y);
                region.max_y = region.max_y.max(y);
                let mut neighbours = [None; 4];
                if x > 0 {
                    neighbours[0] = Some(index - 1);
                }
                if x + 1 < width {
                    neighbours[1] = Some(index + 1);
                }
                if y > 0 {
                    neighbours[2] = Some(index - width);
                }
                if y + 1 < height {
                    neighbours[3] = Some(index + width);
                }
                for neighbour in neighbours.into_iter().flatten() {
                    if binary[neighbour] == 0 && labels[neighbour] == u32::MAX {
                        labels[neighbour] = label;
                        stack.push(neighbour);
                    }
                }
            }
            regions.push(region);
        }
        (labels, regions)
    }

    fn sample_grid(
        width: usize,
        height: usize,
        binary: &[u8],
        corner: QrFinder,
        right: QrFinder,
        below: QrFinder,
    ) -> Option<String> {
        let module = (corner.module + right.module + below.module) / 3.0;
        let (row_x, row_y) = (right.x - corner.x, right.y - corner.y);
        let (column_x, column_y) = (below.x - corner.x, below.y - corner.y);
        let row_length = row_x.hypot(row_y);
        let column_length = column_x.hypot(column_y);
        if (row_length - column_length).abs() > 0.1 * row_length.max(column_length)
            || (row_x * column_x + row_y * column_y).abs() > 0.1 * row_length * column_length
            || row_x * column_y - row_y * column_x <= 0.0
        {
            return None;
        }
        let estimate = ((row_length + column_length) / 2.0 / module).round() + 7.0;
        let version = ((estimate - 17.0) / 4.0).round() as i64;
        for version in [version, version - 1, version + 1] {
            if !(1..=40).contains(&version) {
                continue;
            }
            let size = (version * 4 + 17) as usize;
            let span = (size - 7) as f64;
            let cells: Vec<bool> = (0..size * size)
                .map(|index| {
                    let column = (index % size) as f64 - 3.0;
                    let row = (index / size) as f64 - 3.0;
                    let x = corner.x + (column * row_x + row * column_x) / span;
                    let y = corner.y + (column * row_y + row * column_y) / span;
                    Self::module_is_dark(width, height, binary, x, y)
                })
                .collect();
            let grid = rqrr::Grid::new(rqrr::SimpleGrid::from_func(size, |x, y| cells[y * size + x]));
            if let Ok((_, content)) = grid.decode()
                && !content.is_empty()
            {
                return Some(content);
            }
        }
        None
    }

    fn module_is_dark(width: usize, height: usize, binary: &[u8], x: f64, y: f64) -> bool {
        let (x, y) = (x.round() as i64, y.round() as i64);
        let mut dark = 0;
        for dy in -1..=1_i64 {
            for dx in -1..=1_i64 {
                let sample_x = (x + dx).clamp(0, width as i64 - 1) as usize;
                let sample_y = (y + dy).clamp(0, height as i64 - 1) as usize;
                if binary[sample_y * width + sample_x] == 0 {
                    dark += 1;
                }
            }
        }
        dark >= 5
    }

    fn decode_pixels(width: usize, height: usize, pixels: &[u8]) -> Option<String> {
        let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(width, height, |x, y| pixels[y * width + x]);
        prepared
            .detect_grids()
            .into_iter()
            .filter_map(|grid| grid.decode().ok().map(|(_, content)| content))
            .find(|content| !content.is_empty())
    }

    fn otsu_threshold(pixels: &[u8]) -> u8 {
        let mut histogram = [0u64; 256];
        for &value in pixels {
            histogram[usize::from(value)] += 1;
        }
        let total = pixels.len() as f64;
        let weighted_total: f64 = histogram.iter().enumerate().map(|(level, &count)| level as f64 * count as f64).sum();
        let mut dark_count = 0.0;
        let mut dark_weighted = 0.0;
        let mut best_level = 127u8;
        let mut best_variance = -1.0;
        for (level, &count) in histogram.iter().enumerate() {
            dark_count += count as f64;
            if dark_count == 0.0 {
                continue;
            }
            let light_count = total - dark_count;
            if light_count == 0.0 {
                break;
            }
            dark_weighted += level as f64 * count as f64;
            let dark_mean = dark_weighted / dark_count;
            let light_mean = (weighted_total - dark_weighted) / light_count;
            let variance = dark_count * light_count * (dark_mean - light_mean).powi(2);
            if variance > best_variance {
                best_variance = variance;
                best_level = level as u8;
            }
        }
        best_level
    }

    fn grow_dark(width: usize, height: usize, pixels: &[u8], radius: usize) -> Vec<u8> {
        let mut rows = vec![0u8; pixels.len()];
        for y in 0..height {
            let row = &pixels[y * width..(y + 1) * width];
            for x in 0..width {
                let from = x.saturating_sub(radius);
                let to = (x + radius).min(width - 1);
                rows[y * width + x] = row[from..=to].iter().copied().min().unwrap_or(255);
            }
        }
        let mut grown = vec![0u8; pixels.len()];
        for x in 0..width {
            for y in 0..height {
                let from = y.saturating_sub(radius);
                let to = (y + radius).min(height - 1);
                grown[y * width + x] = (from..=to).map(|row| rows[row * width + x]).min().unwrap_or(255);
            }
        }
        grown
    }
}

/// bounding box of one four connected dark region of a binarized picture
#[derive(Clone, Copy)]
struct DarkRegion {
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
}

impl DarkRegion {
    fn width(&self) -> f64 {
        (self.max_x - self.min_x + 1) as f64
    }

    fn height(&self) -> f64 {
        (self.max_y - self.min_y + 1) as f64
    }

    fn centre(&self) -> (f64, f64) {
        ((self.min_x + self.max_x) as f64 / 2.0, (self.min_y + self.max_y) as f64 / 2.0)
    }
}

/// center of a finder pattern and the module side it implies, in picture pixels
#[derive(Clone, Copy)]
struct QrFinder {
    x: f64,
    y: f64,
    module: f64,
}

/// camera hands frames over on its own thread and the code is looked for there
/// one frame at a time and at most every SCAN_INTERVAL
pub struct RiseQRMetadataDelegate {
    is_delivering: Arc<AtomicBool>,
    is_scanning: AtomicBool,
    last_scan_at: Mutex<Option<Duration>>,
    payloads: mpsc::UnboundedSender<String>,
}

impl RiseQRMetadataDelegate {
    const SCAN_INTERVAL: Duration = Duration::from_millis(200);

    fn new(is_delivering: Arc<AtomicBool>, payloads: mpsc::UnboundedSender<String>) -> Arc<Self> {
        Arc::new(Self { is_delivering, is_scanning: AtomicBool::new(false), last_scan_at: Mutex::new(None), payloads })
    }

    fn metadata_output(self: &Arc<Self>, frame: &CameraFrame) {
        if !self.is_delivering.load(Ordering::Acquire) {
            return;
        }
        {
            let mut last = self.last_scan_at.lock().unwrap_or_else(PoisonError::into_inner);
            if last.is_some_and(|last| frame.captured_at.saturating_sub(last) < Self::SCAN_INTERVAL) {
                return;
            }
            *last = Some(frame.captured_at);
        }
        if self.is_scanning.swap(true, Ordering::AcqRel) {
            return;
        }
        let delegate = Arc::clone(self);
        let frame = frame.clone();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let payload = RiseQRScanResolver::frame_payload(&frame);
            delegate.is_scanning.store(false, Ordering::Release);
            if let Some(payload) = payload {
                let _ = delegate.payloads.unbounded_send(payload);
            }
        }));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RiseQRScannerControllerPhase {
    Idle,
    Running,
    Denied,
    Unavailable,
}

pub struct RiseQRScannerController {
    phase: RiseQRScannerControllerPhase,
    torch_enabled: bool,
    has_torch: bool,

    pub session: VideoNoteCaptureSession,
    pub on_payload: Option<Rc<dyn Fn(String, &mut App)>>,

    camera: Arc<Mutex<Option<VideoNoteCamera>>>,
    generation: Arc<AtomicU64>,
    is_delivering: Arc<AtomicBool>,
    _payload_task: Option<Task<()>>,
    delegate: Arc<RiseQRMetadataDelegate>,
}

impl RiseQRScannerController {
    const CAPTURE_DIRECTORY_NAME: &'static str = "qr-scan";
    const CAPTURE_STEM: &'static str = "qr";

    pub fn new(cx: &mut Context<Self>) -> Self {
        let is_delivering = Arc::new(AtomicBool::new(true));
        let (sender, mut receiver) = mpsc::unbounded::<String>();
        let delegate = RiseQRMetadataDelegate::new(Arc::clone(&is_delivering), sender);
        let payload_task = cx.spawn(async move |this, cx| {
            while let Some(payload) = receiver.next().await {
                let delivered = this.update(cx, |this, cx| this.deliver(payload, cx));
                if delivered.is_err() {
                    return;
                }
            }
        });
        Self {
            phase: RiseQRScannerControllerPhase::Idle,
            torch_enabled: false,
            has_torch: false,
            session: VideoNoteCaptureSession::default(),
            on_payload: None,
            camera: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
            is_delivering,
            _payload_task: Some(payload_task),
            delegate,
        }
    }

    pub fn phase(&self) -> RiseQRScannerControllerPhase {
        self.phase
    }

    pub fn torch_enabled(&self) -> bool {
        self.torch_enabled
    }

    pub fn has_torch(&self) -> bool {
        self.has_torch
    }

    pub fn start(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if self.phase == RiseQRScannerControllerPhase::Running {
            return Task::ready(());
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        cx.spawn(async move |this, cx| {
            let granted = Self::request_access().await;
            if !granted {
                let _ = this.update(cx, |this, cx| {
                    if this.generation.load(Ordering::Acquire) == generation {
                        this.set_phase(RiseQRScannerControllerPhase::Denied, cx);
                    }
                });
                return;
            }
            let Ok((camera, sink, generation_counter)) = this.update(cx, |this, _| {
                (Arc::clone(&this.camera), this.frame_sink(), Arc::clone(&this.generation))
            }) else {
                return;
            };
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let opened = VideoNoteCamera::open(
                        false,
                        temporary_directory().join(Self::CAPTURE_DIRECTORY_NAME),
                        Self::CAPTURE_STEM.to_owned(),
                        sink,
                    );
                    match opened {
                        Ok(mut opened) => {
                            let has_torch = opened.set_torch(false).is_ok();
                            let mut slot = camera.lock().unwrap_or_else(PoisonError::into_inner);
                            if generation_counter.load(Ordering::Acquire) != generation {
                                return Err(None);
                            }
                            *slot = Some(opened);
                            Ok(has_torch)
                        }
                        Err(error) => Err(Some(error)),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation.load(Ordering::Acquire) != generation {
                    return;
                }
                match opened {
                    Ok(has_torch) => {
                        this.has_torch = has_torch;
                        this.is_delivering.store(true, Ordering::Release);
                        this.set_phase(RiseQRScannerControllerPhase::Running, cx);
                    }
                    Err(Some(CameraCaptureError::AccessDenied)) => {
                        this.set_phase(RiseQRScannerControllerPhase::Denied, cx);
                    }
                    Err(Some(_)) => this.set_phase(RiseQRScannerControllerPhase::Unavailable, cx),
                    Err(None) => {}
                }
            });
        })
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.set_torch(false, cx);
        self.is_delivering.store(false, Ordering::Release);
        let taken = {
            let mut slot = self.camera.lock().unwrap_or_else(PoisonError::into_inner);
            self.generation.fetch_add(1, Ordering::AcqRel);
            slot.take()
        };
        cx.background_executor()
            .spawn(async move {
                drop(taken);
            })
            .detach();
        self.session.clear();
        if self.phase == RiseQRScannerControllerPhase::Running {
            self.set_phase(RiseQRScannerControllerPhase::Idle, cx);
        }
    }

    pub fn pause_delivery(&mut self) {
        self.is_delivering.store(false, Ordering::Release);
    }

    pub fn resume_delivery(&mut self) {
        self.is_delivering.store(true, Ordering::Release);
    }

    pub fn toggle_torch(&mut self, cx: &mut Context<Self>) {
        self.set_torch(!self.torch_enabled, cx);
    }

    fn set_torch(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let applied = self
            .camera
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .map(|camera| camera.set_torch(enabled).is_ok())
            .unwrap_or(false);
        let torch_enabled = applied && enabled;
        if self.torch_enabled != torch_enabled {
            self.torch_enabled = torch_enabled;
            cx.notify();
        }
    }

    fn set_phase(&mut self, phase: RiseQRScannerControllerPhase, cx: &mut Context<Self>) {
        if self.phase != phase {
            self.phase = phase;
            cx.notify();
        }
    }

    async fn request_access() -> bool {
        match capture_permission::ensure_access(CaptureKind::Camera).await {
            Ok(CaptureDecision::Granted) => true,
            Ok(CaptureDecision::Declined | CaptureDecision::Blocked) => false,
            Err(_) => true,
        }
    }

    fn frame_sink(&self) -> FrameSink {
        let preview = self.session.frame_sink();
        let delegate = Arc::clone(&self.delegate);
        Arc::new(move |frame: CameraFrame| {
            delegate.metadata_output(&frame);
            preview(frame);
        })
    }

    fn deliver(&mut self, payload: String, cx: &mut Context<Self>) {
        if !self.is_delivering.load(Ordering::Acquire) {
            return;
        }
        self.is_delivering.store(false, Ordering::Release);
        if let Some(on_payload) = self.on_payload.clone() {
            on_payload(payload, cx);
        }
    }
}
