//! The binding to each OS's external-memory API.
//!
//! This is the only file in the texture path that contains `#[cfg(target_os)]`,
//! and it contains no decisions: by the time anything here is called,
//! `ImportPlan` has already chosen the mechanism. What is left is the call
//! itself, which is the one part that genuinely cannot be exercised on a Mac.
//!
//! Every arm reports whether it did the thing, in the style of
//! `rise_platform::gpui_shim::PlatformSupport`, so a caller can fall back to a
//! CPU upload instead of showing a black rectangle.

use rise_platform::HostOs;

use super::external_texture::TextureImport;
use super::frame::FrameGeometry;

/// Why an import could not be performed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImportError {
    /// The mechanism does not exist on the OS this build is running on. A
    /// programming error: the plan and the binding disagreed.
    WrongHost {
        expected: TextureImport,
        host: HostOs,
    },
    /// The driver or OS refused. Recoverable — the caller degrades to
    /// `TextureImport::CpuUpload`.
    Refused(String),
    /// The arm exists in source but has never been executed on real hardware.
    /// Returned rather than silently attempting an unverified unsafe call.
    NotExercised {
        host: HostOs,
        mechanism: TextureImport,
    },
}

/// An imported texture, identified by whatever the backend uses to name it.
///
/// Deliberately opaque: `app/` must never learn which OS it is on, and a raw
/// `wgpu::Texture` or `CVPixelBuffer` in this signature would leak that.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImportedTexture {
    mechanism: TextureImport,
    raw: u64,
}

impl ImportedTexture {
    pub const fn mechanism(&self) -> TextureImport {
        self.mechanism
    }

    /// The backend-specific identifier. Only the renderer for the matching
    /// mechanism may interpret this.
    pub const fn raw(&self) -> u64 {
        self.raw
    }
}

/// Import decoder-owned memory as a texture.
///
/// `raw_handle` is the IOSurface id, the DMA-BUF fd, or the DXGI shared handle,
/// according to `mechanism`.
pub fn import(
    mechanism: TextureImport,
    raw_handle: u64,
    geometry: &FrameGeometry,
) -> Result<ImportedTexture, ImportError> {
    let host = HostOs::current();

    match mechanism {
        TextureImport::IoSurface => import_io_surface(host, raw_handle, geometry),
        TextureImport::DmaBuf => import_dma_buf(host, raw_handle, geometry),
        TextureImport::DxgiSharedHandle => import_dxgi(host, raw_handle, geometry),
        TextureImport::CpuUpload => Ok(ImportedTexture {
            mechanism: TextureImport::CpuUpload,
            raw: raw_handle,
        }),
    }
}

fn import_io_surface(
    #[cfg_attr(target_os = "macos", allow(unused_variables))] host: HostOs,
    raw_handle: u64,
    geometry: &FrameGeometry,
) -> Result<ImportedTexture, ImportError> {
    #[cfg(target_os = "macos")]
    {
        let _ = geometry;
        // VideoToolbox already decoded into this IOSurface, and gpui's
        // `surface()` takes the CVPixelBuffer that wraps it directly. There is
        // no import step to perform and nothing to copy: the "import" is
        // keeping the buffer alive until the frame has been presented.
        Ok(ImportedTexture {
            mechanism: TextureImport::IoSurface,
            raw: raw_handle,
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (raw_handle, geometry);
        Err(ImportError::WrongHost {
            expected: TextureImport::IoSurface,
            host,
        })
    }
}

fn import_dma_buf(
    host: HostOs,
    raw_handle: u64,
    geometry: &FrameGeometry,
) -> Result<ImportedTexture, ImportError> {
    #[cfg(target_os = "linux")]
    {
        let _ = (raw_handle, geometry);
        // VAAPI exports the decoded surface with vaExportSurfaceHandle, giving
        // one fd per plane plus a DRM format modifier. Importing it means a
        // wgpu-hal vulkan device with VK_EXT_external_memory_dma_buf and
        // VK_EXT_image_drm_format_modifier, binding the fd's memory to an image
        // created with the exported modifier — NOT with OPTIMAL tiling, which
        // is the mistake that produces a correctly sized, garbled picture.
        //
        // Unverified. Nobody has run this, and reporting success from an
        // unexecuted unsafe path is worse than falling back to a CPU upload.
        Err(ImportError::NotExercised {
            host,
            mechanism: TextureImport::DmaBuf,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (raw_handle, geometry);
        Err(ImportError::WrongHost {
            expected: TextureImport::DmaBuf,
            host,
        })
    }
}

fn import_dxgi(
    host: HostOs,
    raw_handle: u64,
    geometry: &FrameGeometry,
) -> Result<ImportedTexture, ImportError> {
    #[cfg(target_os = "windows")]
    {
        let _ = (raw_handle, geometry);
        // D3D11VA decodes into an ID3D11Texture2D. Sharing it needs the texture
        // created with D3D11_RESOURCE_MISC_SHARED_NTHANDLE, then
        // ID3D12Device::OpenSharedHandle on the wgpu-hal dx12 device. The
        // legacy non-NT shared handle cannot be opened by D3D12 at all, which
        // is the trap here.
        //
        // Unverified, same reasoning as the Linux arm.
        Err(ImportError::NotExercised {
            host,
            mechanism: TextureImport::DxgiSharedHandle,
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (raw_handle, geometry);
        Err(ImportError::WrongHost {
            expected: TextureImport::DxgiSharedHandle,
            host,
        })
    }
}

/// Whether a failed import can be retried as a CPU upload.
///
/// A refusal and an unexercised arm both degrade; a host mismatch is a bug in
/// the plan and must not be papered over.
pub const fn is_recoverable(error: &ImportError) -> bool {
    matches!(
        error,
        ImportError::Refused(_) | ImportError::NotExercised { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture::frame::{FrameFormat, FrameGeometry};

    fn hd() -> FrameGeometry {
        FrameGeometry::packed(FrameFormat::Nv12, 1920, 1080).unwrap()
    }

    #[test]
    fn a_cpu_upload_always_succeeds_on_every_host() {
        let imported = import(TextureImport::CpuUpload, 0, &hd()).unwrap();
        assert_eq!(imported.mechanism(), TextureImport::CpuUpload);
    }

    #[test]
    fn the_hosts_own_mechanism_is_the_only_one_that_does_not_report_wrong_host() {
        let host = HostOs::current();
        let native = match host {
            HostOs::MacOs => TextureImport::IoSurface,
            HostOs::Linux => TextureImport::DmaBuf,
            HostOs::Windows => TextureImport::DxgiSharedHandle,
        };

        for mechanism in [
            TextureImport::IoSurface,
            TextureImport::DmaBuf,
            TextureImport::DxgiSharedHandle,
        ] {
            let result = import(mechanism, 42, &hd());

            if mechanism == native {
                assert!(
                    !matches!(result, Err(ImportError::WrongHost { .. })),
                    "{mechanism:?} is native to {host:?}"
                );
            } else {
                assert_eq!(
                    result,
                    Err(ImportError::WrongHost {
                        expected: mechanism,
                        host
                    }),
                    "{mechanism:?} on {host:?}"
                );
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_io_surface_import_is_a_no_op_that_preserves_the_handle() {
        let imported = import(TextureImport::IoSurface, 9001, &hd()).unwrap();

        assert_eq!(imported.mechanism(), TextureImport::IoSurface);
        assert_eq!(
            imported.raw(),
            9001,
            "VideoToolbox already decoded into GPU memory; anything else here is a copy"
        );
    }

    #[test]
    fn a_wrong_host_is_a_bug_and_is_not_recoverable() {
        assert!(!is_recoverable(&ImportError::WrongHost {
            expected: TextureImport::DmaBuf,
            host: HostOs::MacOs,
        }));
    }

    #[test]
    fn a_refusal_and_an_unexercised_arm_both_degrade_to_a_cpu_upload() {
        assert!(is_recoverable(&ImportError::Refused(
            "no vaapi device".into()
        )));
        assert!(is_recoverable(&ImportError::NotExercised {
            host: HostOs::Linux,
            mechanism: TextureImport::DmaBuf,
        }));
    }
}
