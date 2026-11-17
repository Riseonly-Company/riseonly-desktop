use rise_platform::HostOs;

use crate::texture::external_texture::TextureImport;
use crate::texture::frame::FrameGeometry;

/// why an import could not be performed
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImportError {
    WrongHost {
        expected: TextureImport,
        host: HostOs,
    },
    Refused(String),
    NotExercised {
        host: HostOs,
        mechanism: TextureImport,
    },
}

/// imported texture, opaque so a caller cant learn which os it is on
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImportedTexture {
    mechanism: TextureImport,
    raw: u64,
}

impl ImportedTexture {
    pub(super) const fn new(mechanism: TextureImport, raw: u64) -> Self {
        Self { mechanism, raw }
    }

    pub const fn mechanism(&self) -> TextureImport {
        self.mechanism
    }

    /// backend specific id, only the renderer for its mechanism may interpret it
    pub const fn raw(&self) -> u64 {
        self.raw
    }
}

/// imports decoder owned memory as a texture
/// raw_handle is the iosurface id or the dma buf fd or the dxgi shared handle, by mechanism
pub fn import(
    mechanism: TextureImport,
    raw_handle: u64,
    geometry: &FrameGeometry,
) -> Result<ImportedTexture, ImportError> {
    crate::decode_guard::decode_guard::check_frame_bytes(geometry.byte_len())
        .map_err(|refusal| ImportError::Refused(refusal.to_string()))?;

    let host = HostOs::current();

    match mechanism {
        TextureImport::CpuUpload => {
            Ok(ImportedTexture::new(TextureImport::CpuUpload, raw_handle))
        }
        native if native == TextureImport::native(host) => {
            super::os::import_native(host, raw_handle, geometry)
        }
        expected => Err(ImportError::WrongHost { expected, host }),
    }
}

/// whether a failed import can be retried as a cpu upload
/// a host mismatch is a bug and cant
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
        let native = TextureImport::native(host);

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
