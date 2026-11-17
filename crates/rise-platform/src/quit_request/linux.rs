use crate::gpui_shim::PlatformSupport;

pub(super) fn install(_callback: Box<dyn Fn()>) -> PlatformSupport {
    PlatformSupport::Unsupported
}
