use rise_platform::HostOs;

use super::import::*;
use crate::texture::external_texture::TextureImport;
use crate::texture::frame::FrameGeometry;

pub(super) fn import_native(
    host: HostOs,
    _raw_handle: u64,
    _geometry: &FrameGeometry,
) -> Result<ImportedTexture, ImportError> {
    Err(ImportError::NotExercised {
        host,
        mechanism: TextureImport::DmaBuf,
    })
}
