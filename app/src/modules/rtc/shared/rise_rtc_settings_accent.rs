use gpui::{Hsla, Rgba};

/// icon colors that guide the call and room settings sheets, a single column
/// shared by direct call and voice room and theme independent by design
pub enum RiseRTCSettingsAccent {}

impl RiseRTCSettingsAccent {
    pub fn screen() -> Hsla {
        Rgba { r: 0.20, g: 0.72, b: 0.42, a: 1.0 }.into()
    }

    pub fn quality() -> Hsla {
        Rgba { r: 0.35, g: 0.47, b: 0.95, a: 1.0 }.into()
    }

    pub fn stream_audio() -> Hsla {
        Rgba { r: 0.16, g: 0.66, b: 0.72, a: 1.0 }.into()
    }

    pub fn microphone() -> Hsla {
        Rgba { r: 0.96, g: 0.55, b: 0.16, a: 1.0 }.into()
    }

    pub fn output() -> Hsla {
        Rgba { r: 0.60, g: 0.38, b: 0.92, a: 1.0 }.into()
    }
}
