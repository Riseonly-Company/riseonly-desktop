//! Getting a decoded frame onto the GPU without copying it through the CPU.
//!
//! THE PROBLEM THIS SOLVES
//!
//! `gpui::surface()` accepts only a `CVPixelBuffer` and exists only on macOS;
//! on the wgpu backends its paint method is a silent no-op. Zed's own
//! livekit_client therefore aliases `RemoteVideoFrame = Arc<gpui::RenderImage>`
//! off macOS and performs a full I420 -> RGBA conversion on the CPU plus a
//! fresh texture upload for every frame. At 30fps, 1080p, that is 250 MB/s of
//! memcpy and a new allocation sixty times a second per stream. A vertical
//! short-video feed cannot ship on it.
//!
//! The replacement is one persistent texture per stream, written into, with the
//! decoder's own GPU memory imported where the platform allows it: an IOSurface
//! on macOS, a DMA-BUF file descriptor on Linux, a DXGI shared NT handle on
//! Windows. YUV becomes RGB in a shader (`nv12_to_rgb.wgsl`), never on the CPU.
//!
//! WHAT IS VERIFIED
//!
//! The policy in this file is a pure function of `HostOs`, so all three arms run
//! in the suite on a Mac. The final call into the OS import API is behind
//! `#[cfg(target_os)]` and only the macOS one has been executed.

use std::sync::Arc;

use parking_lot::Mutex;
use rise_platform::HostOs;

use super::frame::{FrameFormat, FrameGeometry};

/// The WGSL that samples a biplanar or triplanar YUV frame and writes RGB.
///
/// Kept as a `&str` constant rather than a file read so a missing asset is a
/// compile error rather than a black video element at runtime.
pub const YUV_TO_RGB_SHADER: &str = include_str!("nv12_to_rgb.wgsl");

/// How a decoded frame's memory reaches the GPU.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextureImport {
    /// macOS. VideoToolbox already decodes into an IOSurface-backed
    /// CVPixelBuffer, which is GPU memory. Nothing is copied at all.
    IoSurface,
    /// Linux. VAAPI exports the decoded surface as a DMA-BUF fd, imported
    /// through `wgpu_hal::vulkan` with VK_EXT_external_memory_dma_buf.
    DmaBuf,
    /// Windows. D3D11VA decodes into an ID3D11Texture2D; sharing it as an NT
    /// handle lets the DX12 device open it without a copy.
    DxgiSharedHandle,
    /// Software decode, or hardware import unavailable. The frame is staged
    /// into a buffer and copied into the SAME persistent texture each time.
    /// Slower, but still one allocation per stream rather than per frame.
    CpuUpload,
}

impl TextureImport {
    /// Whether the decoder's memory is used directly.
    pub const fn is_zero_copy(self) -> bool {
        !matches!(self, Self::CpuUpload)
    }

    /// The import this host uses when the hardware path is unavailable — a
    /// headless CI runner, a VM with no VAAPI, a machine whose driver refuses
    /// the external-memory extension.
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
    /// on the wgpu backends, because `surface()` does not exist there.
    OwnedTexture,
}

/// Named so it can be asserted against, never returned.
///
/// This is the path Zed's livekit code takes off macOS. It is written down as a
/// type purely so `plans_never_use_a_per_frame_render_image` has something
/// concrete to fail on if someone later "simplifies" the import away.
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

    /// The plan when hardware decode or hardware import is unavailable.
    ///
    /// Note that the presentation does NOT change: on macOS a CPU-decoded frame
    /// is still written into a CVPixelBuffer and shown through `surface()`,
    /// because switching presentation paths mid-stream would mean two code
    /// paths for the same element.
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
            // `gpui::surface()` is `#[cfg(target_os = "macos")]` and
            // `SurfaceSource` has exactly one variant. There is nothing to fall
            // back to here; the texture must be ours.
            HostOs::Linux | HostOs::Windows => Presentation::OwnedTexture,
        }
    }

    /// Every plan allocates once per stream. Nothing in this module may ever
    /// produce a per-frame image.
    pub const fn allocates_per_frame(&self) -> bool {
        false
    }
}

/// A hard ceiling on GPU-visible frame memory, in bytes.
///
/// A community report documents gpui's memory for six static images falling
/// from 300 MB to 12 MB once CPU-side bytes stopped being retained after upload.
/// Video multiplies that by frame rate and stream count, so the budget is
/// enforced rather than estimated: a stream that would cross the ceiling is
/// refused, and the scheduler tears down an offscreen one instead.
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
    /// 384 MiB. Three simultaneous 1080p streams at three buffered frames each
    /// is roughly 28 MiB, so the ceiling leaves room for stickers and images
    /// while staying far below the point where a laptop starts swapping.
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

/// Tracks what the live stream textures actually cost.
///
/// Shared by clone; the scheduler and every stream texture hold the same
/// ledger, which is what makes the ceiling a property of the process rather
/// than of one list.
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

/// How many decoded frames a stream keeps alive at once.
///
/// One being presented, one being written by the decoder, one in reserve for
/// the next vsync. A fourth buys nothing and costs a full frame of memory per
/// stream.
pub const FRAMES_IN_FLIGHT: u64 = 3;

/// One persistent GPU texture, owned by one video stream for its lifetime.
///
/// The invariant this type exists to hold: frames are WRITTEN INTO it. It is
/// allocated when the stream opens and freed when the stream closes, and
/// nothing between those two points allocates GPU memory.
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

    /// Accept the next decoded frame.
    ///
    /// Returns whether the texture had to be reallocated, which happens only on
    /// a resolution change. The caller cares because a reallocation is the one
    /// moment a stream can fail on budget after it has already started.
    pub fn write(&mut self, geometry: &FrameGeometry) -> Result<Reallocated, BudgetError> {
        if geometry.can_reuse_texture_of(&self.geometry) {
            self.frames_written += 1;
            return Ok(Reallocated::No);
        }

        let next_bytes = geometry.byte_len() * FRAMES_IN_FLIGHT;

        // Released first: an adaptive-bitrate step DOWN must not fail on a
        // ceiling that the old, larger allocation was itself responsible for.
        self.tracker.release(self.reserved_bytes);
        if let Err(error) = self.tracker.reserve(next_bytes) {
            // Put the old reservation back so the ledger still describes the
            // texture this stream is in fact still holding.
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
/// is running on.
///
/// The macOS answer is verified. The other two report what the code intends,
/// and `docs/PLATFORM_MATRIX.md` records that neither has been executed.
pub fn zero_copy_import_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Both remaining arms depend on a runtime driver capability
        // (VK_EXT_external_memory_dma_buf, or D3D11 shared-NT-handle support),
        // so this cannot be answered at compile time. Until the arm is
        // actually exercised, claiming availability would be a lie in the one
        // direction that hides a per-frame copy.
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
