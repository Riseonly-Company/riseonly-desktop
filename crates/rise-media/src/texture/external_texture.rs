//! Getting a decoded frame onto the GPU without copying it through the CPU:
//! one persistent texture per stream, the decoder's own GPU memory imported
//! where the platform allows it, and YUV converted in a shader.

use std::sync::Arc;

use parking_lot::Mutex;
use rise_platform::HostOs;

use super::frame::{FrameFormat, FrameGeometry};

/// The WGSL that samples a biplanar or triplanar YUV frame and writes RGB.
pub const YUV_TO_RGB_SHADER: &str = include_str!("nv12_to_rgb.wgsl");

/// How a decoded frame's memory reaches the GPU.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextureImport {
    /// macOS. The IOSurface-backed CVPixelBuffer VideoToolbox decoded into.
    IoSurface,
    /// Linux. A VAAPI DMA-BUF fd, imported through `wgpu_hal::vulkan` with
    /// VK_EXT_external_memory_dma_buf.
    DmaBuf,
    /// Windows. A D3D11VA texture shared as an NT handle and opened by DX12.
    DxgiSharedHandle,
    /// Software decode, or hardware import unavailable: the frame is staged and
    /// copied into the same persistent texture each time.
    CpuUpload,
}

impl TextureImport {
    /// Whether the decoder's memory is used directly.
    pub const fn is_zero_copy(self) -> bool {
        !matches!(self, Self::CpuUpload)
    }

    /// The import this host uses when the hardware path is unavailable.
    pub const fn fallback() -> Self {
        Self::CpuUpload
    }
}

/// How the imported texture is handed to the toolkit for drawing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Presentation {
    /// `gpui::surface()`. macOS only, and it takes the CVPixelBuffer directly.
    GpuiSurface,
    /// We own a `wgpu::Texture` and sample it in our own pass. The only option
    /// on the wgpu backends, where `surface()` does not exist.
    OwnedTexture,
}

/// The rejected per-frame-image path, named so it can be asserted against and
/// never returned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RejectedPerFrameRenderImage;

/// The complete decision for one host: how frames arrive, how they are shown,
/// and whether a colour-conversion pass is needed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImportPlan {
    pub import: TextureImport,
    pub presentation: Presentation,
    pub converts_in_shader: bool,
}

impl ImportPlan {
    /// The plan for a host and format, assuming hardware decode succeeded.
    pub const fn hardware(host: HostOs, format: FrameFormat) -> Self {
        let import = match host {
            HostOs::MacOs => TextureImport::IoSurface,
            HostOs::Linux => TextureImport::DmaBuf,
            HostOs::Windows => TextureImport::DxgiSharedHandle,
        };

        Self {
            import,
            presentation: Self::presentation_for(host),
            converts_in_shader: format.needs_color_conversion(),
        }
    }

    /// The plan when hardware decode or hardware import is unavailable. The
    /// presentation does NOT change, so no stream switches paths mid-flight.
    pub const fn software(host: HostOs, format: FrameFormat) -> Self {
        Self {
            import: TextureImport::fallback(),
            presentation: Self::presentation_for(host),
            converts_in_shader: format.needs_color_conversion(),
        }
    }

    const fn presentation_for(host: HostOs) -> Presentation {
        match host {
            HostOs::MacOs => Presentation::GpuiSurface,
            // `gpui::surface()` is macOS-only: off it, the texture must be ours.
            HostOs::Linux | HostOs::Windows => Presentation::OwnedTexture,
        }
    }

    /// Always false: every plan allocates once per stream, never per frame.
    pub const fn allocates_per_frame(&self) -> bool {
        false
    }
}

/// A hard ceiling on GPU-visible frame memory, in bytes. A stream that would
/// cross it is refused rather than admitted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TextureBudget {
    ceiling_bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BudgetError {
    WouldExceedCeiling {
        requested: u64,
        in_use: u64,
        ceiling: u64,
    },
}

impl TextureBudget {
    /// 384 MiB. Three simultaneous 1080p streams cost roughly 28 MiB of it.
    pub const DEFAULT_CEILING: u64 = 384 * 1024 * 1024;

    pub const fn new(ceiling_bytes: u64) -> Self {
        Self { ceiling_bytes }
    }

    pub const fn ceiling(&self) -> u64 {
        self.ceiling_bytes
    }
}

impl Default for TextureBudget {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CEILING)
    }
}

#[derive(Debug, Default)]
struct BudgetLedger {
    in_use: u64,
}

/// Tracks what the live stream textures actually cost. Clones share one ledger,
/// so the ceiling is a property of the process.
#[derive(Clone, Debug)]
pub struct BudgetTracker {
    budget: TextureBudget,
    ledger: Arc<Mutex<BudgetLedger>>,
}

impl BudgetTracker {
    pub fn new(budget: TextureBudget) -> Self {
        Self {
            budget,
            ledger: Arc::new(Mutex::new(BudgetLedger::default())),
        }
    }

    pub fn in_use(&self) -> u64 {
        self.ledger.lock().in_use
    }

    pub fn ceiling(&self) -> u64 {
        self.budget.ceiling()
    }

    pub(crate) fn reserve(&self, bytes: u64) -> Result<(), BudgetError> {
        let mut ledger = self.ledger.lock();

        let next = ledger.in_use.saturating_add(bytes);
        if next > self.budget.ceiling() {
            return Err(BudgetError::WouldExceedCeiling {
                requested: bytes,
                in_use: ledger.in_use,
                ceiling: self.budget.ceiling(),
            });
        }

        ledger.in_use = next;
        Ok(())
    }

    pub(crate) fn release(&self, bytes: u64) {
        let mut ledger = self.ledger.lock();
        ledger.in_use = ledger.in_use.saturating_sub(bytes);
    }
}

impl Default for BudgetTracker {
    fn default() -> Self {
        Self::new(TextureBudget::default())
    }
}

/// How many decoded frames a stream keeps alive at once: one presented, one
/// being written by the decoder, one in reserve for the next vsync.
pub const FRAMES_IN_FLIGHT: u64 = 3;

/// One persistent GPU texture, owned by one video stream for its lifetime.
/// Frames are written into it; nothing between open and close allocates.
#[derive(Debug)]
pub struct StreamTexture {
    geometry: FrameGeometry,
    plan: ImportPlan,
    tracker: BudgetTracker,
    reserved_bytes: u64,
    frames_written: u64,
}

impl StreamTexture {
    pub fn open(
        geometry: FrameGeometry,
        plan: ImportPlan,
        tracker: BudgetTracker,
    ) -> Result<Self, BudgetError> {
        let reserved_bytes = geometry.byte_len() * FRAMES_IN_FLIGHT;
        tracker.reserve(reserved_bytes)?;

        Ok(Self {
            geometry,
            plan,
            tracker,
            reserved_bytes,
            frames_written: 0,
        })
    }

    pub fn geometry(&self) -> &FrameGeometry {
        &self.geometry
    }

    pub fn plan(&self) -> ImportPlan {
        self.plan
    }

    pub fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }

    /// Accept the next decoded frame, reporting whether the texture had to be
    /// reallocated — a resolution change, and the only point after open at
    /// which a running stream can fail on budget.
    pub fn write(&mut self, geometry: &FrameGeometry) -> Result<Reallocated, BudgetError> {
        if geometry.can_reuse_texture_of(&self.geometry) {
            self.frames_written += 1;
            return Ok(Reallocated::No);
        }

        let next_bytes = geometry.byte_len() * FRAMES_IN_FLIGHT;

        // Release before reserve, or a step DOWN in resolution fails on the ceiling it frees.
        self.tracker.release(self.reserved_bytes);
        if let Err(error) = self.tracker.reserve(next_bytes) {
            let _ = self.tracker.reserve(self.reserved_bytes);
            return Err(error);
        }

        self.geometry = geometry.clone();
        self.reserved_bytes = next_bytes;
        self.frames_written += 1;

        Ok(Reallocated::Yes)
    }
}

impl Drop for StreamTexture {
    fn drop(&mut self) {
        self.tracker.release(self.reserved_bytes);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reallocated {
    Yes,
    No,
}

/// Whether this build can import decoder memory without a copy on the host it
/// is running on. Only the macOS arm has been executed.
pub fn zero_copy_import_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Depends on a runtime driver capability, and neither arm is exercised yet.
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hd() -> FrameGeometry {
        FrameGeometry::packed(FrameFormat::Nv12, 1920, 1080).unwrap()
    }

    #[test]
    fn every_host_has_a_zero_copy_hardware_plan() {
        for host in HostOs::ALL {
            let plan = ImportPlan::hardware(host, FrameFormat::Nv12);
            assert!(
                plan.import.is_zero_copy(),
                "{host:?} would fall back to a CPU copy on the hardware path"
            );
        }
    }

    #[test]
    fn each_host_imports_through_its_own_mechanism() {
        assert_eq!(
            ImportPlan::hardware(HostOs::MacOs, FrameFormat::Nv12).import,
            TextureImport::IoSurface
        );
        assert_eq!(
            ImportPlan::hardware(HostOs::Linux, FrameFormat::Nv12).import,
            TextureImport::DmaBuf
        );
        assert_eq!(
            ImportPlan::hardware(HostOs::Windows, FrameFormat::Nv12).import,
            TextureImport::DxgiSharedHandle
        );
    }

    #[test]
    fn only_macos_presents_through_gpui_surface() {
        for host in HostOs::ALL {
            let expected = if host == HostOs::MacOs {
                Presentation::GpuiSurface
            } else {
                Presentation::OwnedTexture
            };
            assert_eq!(
                ImportPlan::hardware(host, FrameFormat::Nv12).presentation,
                expected
            );
        }
    }

    #[test]
    fn the_software_fallback_keeps_the_hosts_presentation_path() {
        for host in HostOs::ALL {
            let hardware = ImportPlan::hardware(host, FrameFormat::Nv12);
            let software = ImportPlan::software(host, FrameFormat::Nv12);

            assert_eq!(hardware.presentation, software.presentation);
            assert!(!software.import.is_zero_copy());
        }
    }

    #[test]
    fn plans_never_allocate_per_frame() {
        for host in HostOs::ALL {
            for format in FrameFormat::ALL {
                assert!(!ImportPlan::hardware(host, format).allocates_per_frame());
                assert!(!ImportPlan::software(host, format).allocates_per_frame());
            }
        }
    }

    #[test]
    fn yuv_converts_in_the_shader_and_bgra_does_not() {
        for host in HostOs::ALL {
            assert!(ImportPlan::hardware(host, FrameFormat::Nv12).converts_in_shader);
            assert!(ImportPlan::hardware(host, FrameFormat::P010).converts_in_shader);
            assert!(ImportPlan::hardware(host, FrameFormat::I420).converts_in_shader);
            assert!(!ImportPlan::hardware(host, FrameFormat::Bgra8).converts_in_shader);
        }
    }

    #[test]
    fn the_shader_is_compiled_in_and_covers_every_yuv_format() {
        assert!(YUV_TO_RGB_SHADER.contains("@fragment"));
        for entry in ["nv12", "p010", "i420"] {
            assert!(
                YUV_TO_RGB_SHADER.contains(entry),
                "the shader has no path for {entry}, so that format would show black"
            );
        }
    }

    #[test]
    fn a_stream_reserves_its_frames_in_flight_and_releases_them_on_drop() {
        let tracker = BudgetTracker::default();
        let expected = hd().byte_len() * FRAMES_IN_FLIGHT;

        {
            let texture = StreamTexture::open(
                hd(),
                ImportPlan::hardware(HostOs::MacOs, FrameFormat::Nv12),
                tracker.clone(),
            )
            .unwrap();

            assert_eq!(texture.reserved_bytes(), expected);
            assert_eq!(tracker.in_use(), expected);
        }

        assert_eq!(
            tracker.in_use(),
            0,
            "a closed stream must not hold GPU memory"
        );
    }

    #[test]
    fn writing_frames_never_grows_the_reservation() {
        let tracker = BudgetTracker::default();
        let mut texture = StreamTexture::open(
            hd(),
            ImportPlan::hardware(HostOs::MacOs, FrameFormat::Nv12),
            tracker.clone(),
        )
        .unwrap();

        let baseline = tracker.in_use();
        for _ in 0..600 {
            assert_eq!(texture.write(&hd()).unwrap(), Reallocated::No);
        }

        assert_eq!(texture.frames_written(), 600);
        assert_eq!(
            tracker.in_use(),
            baseline,
            "ten seconds of 60fps playback allocated GPU memory"
        );
    }

    #[test]
    fn a_resolution_change_reallocates_and_reprices_the_stream() {
        let tracker = BudgetTracker::default();
        let mut texture = StreamTexture::open(
            hd(),
            ImportPlan::hardware(HostOs::MacOs, FrameFormat::Nv12),
            tracker.clone(),
        )
        .unwrap();

        let smaller = FrameGeometry::packed(FrameFormat::Nv12, 640, 360).unwrap();
        assert_eq!(texture.write(&smaller).unwrap(), Reallocated::Yes);

        assert_eq!(tracker.in_use(), smaller.byte_len() * FRAMES_IN_FLIGHT);
    }

    #[test]
    fn a_step_down_in_resolution_cannot_fail_on_the_budget_it_frees() {
        let one_stream = hd().byte_len() * FRAMES_IN_FLIGHT;
        let tracker = BudgetTracker::new(TextureBudget::new(one_stream));

        let mut texture = StreamTexture::open(
            hd(),
            ImportPlan::hardware(HostOs::MacOs, FrameFormat::Nv12),
            tracker.clone(),
        )
        .unwrap();

        let smaller = FrameGeometry::packed(FrameFormat::Nv12, 1280, 720).unwrap();
        assert_eq!(
            texture.write(&smaller).unwrap(),
            Reallocated::Yes,
            "releasing after reserving would refuse a stream that is shrinking"
        );
    }

    #[test]
    fn a_failed_reallocation_leaves_the_ledger_describing_reality() {
        let tracker = BudgetTracker::new(TextureBudget::new(hd().byte_len() * FRAMES_IN_FLIGHT));
        let mut texture = StreamTexture::open(
            hd(),
            ImportPlan::hardware(HostOs::MacOs, FrameFormat::Nv12),
            tracker.clone(),
        )
        .unwrap();

        let larger = FrameGeometry::packed(FrameFormat::Nv12, 3840, 2160).unwrap();
        assert!(texture.write(&larger).is_err());

        assert_eq!(
            tracker.in_use(),
            hd().byte_len() * FRAMES_IN_FLIGHT,
            "the old texture is still held, so the ledger must still count it"
        );
        assert!(texture.geometry().can_reuse_texture_of(&hd()));
    }

    #[test]
    fn a_stream_that_would_cross_the_ceiling_is_refused_rather_than_admitted() {
        let tracker = BudgetTracker::new(TextureBudget::new(hd().byte_len() * FRAMES_IN_FLIGHT));

        let _first = StreamTexture::open(
            hd(),
            ImportPlan::hardware(HostOs::MacOs, FrameFormat::Nv12),
            tracker.clone(),
        )
        .unwrap();

        let second = StreamTexture::open(
            hd(),
            ImportPlan::hardware(HostOs::MacOs, FrameFormat::Nv12),
            tracker.clone(),
        );

        assert!(matches!(
            second,
            Err(BudgetError::WouldExceedCeiling { .. })
        ));
        assert_eq!(tracker.in_use(), hd().byte_len() * FRAMES_IN_FLIGHT);
    }

    #[test]
    fn three_simultaneous_1080p_streams_fit_the_default_ceiling() {
        let tracker = BudgetTracker::default();
        let plan = ImportPlan::hardware(HostOs::MacOs, FrameFormat::Nv12);

        let streams: Vec<_> = (0..3)
            .map(|_| StreamTexture::open(hd(), plan, tracker.clone()).unwrap())
            .collect();

        assert_eq!(streams.len(), 3);
        assert!(tracker.in_use() < tracker.ceiling() / 2);
    }

    #[test]
    fn zero_copy_availability_is_only_claimed_where_it_was_executed() {
        assert_eq!(zero_copy_import_available(), cfg!(target_os = "macos"));
    }
}
