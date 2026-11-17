use rise_platform::HostOs;

use crate::texture::external_texture::TextureImport;
use crate::texture::frame::FrameGeometry;
use crate::texture::import::*;

pub(crate) fn import_native(
    _host: HostOs,
    raw_handle: u64,
    _geometry: &FrameGeometry,
) -> Result<ImportedTexture, ImportError> {
    Ok(ImportedTexture::new(TextureImport::IoSurface, raw_handle))
}
