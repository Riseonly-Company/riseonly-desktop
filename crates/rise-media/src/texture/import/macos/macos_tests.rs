use crate::texture::external_texture::TextureImport;
use crate::texture::frame::{FrameFormat, FrameGeometry};
use crate::texture::import::*;

fn hd() -> FrameGeometry {
    FrameGeometry::packed(FrameFormat::Nv12, 1920, 1080).unwrap()
}

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
