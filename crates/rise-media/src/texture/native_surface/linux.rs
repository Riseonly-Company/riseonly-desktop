use gpui::{AnyElement, IntoElement, OwnedSurfaceFrame, Styled};
use rise_platform::HostOs;

use super::native_surface::*;

pub(super) struct NativeSurfaceBinding;

impl SurfaceBinding for NativeSurfaceBinding {
    fn present_host_frame(
        &self,
        _host: HostOs,
        frame: &HostNv12Frame,
    ) -> Result<AnyElement, SurfaceError> {
        let owned = OwnedSurfaceFrame::nv12(
            frame.stream_id,
            frame.generation,
            frame.width,
            frame.height,
            frame.luma.clone(),
            frame.luma_stride,
            frame.chroma.clone(),
            frame.chroma_stride,
        )
        .ok_or(SurfaceError::WrongKind {
            expected: "NV12 planes as long as their rows and strides need",
        })?
        .with_ycbcr_to_rgb(ycbcr_to_rgb_columns(&frame.conversion));

        Ok(gpui::surface(owned).size_full().into_any_element())
    }

    fn present_host_bgra_frame(
        &self,
        _host: HostOs,
        frame: &HostBgraFrame,
    ) -> Result<AnyElement, SurfaceError> {
        let owned = OwnedSurfaceFrame::bgra(
            frame.stream_id,
            frame.generation,
            frame.width,
            frame.height,
            frame.pixels.clone(),
            frame.stride,
        )
        .ok_or(SurfaceError::WrongKind {
            expected: "BGRA rows as long as their width and stride need",
        })?;

        Ok(gpui::surface(owned).size_full().into_any_element())
    }
}
