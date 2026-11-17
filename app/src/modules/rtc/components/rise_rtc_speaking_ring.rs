use gpui::{Hsla, Rgba};

/// one green for this person is talking, shared by the call screen
/// avatar ring and the minimized widget so they cant drift apart
pub enum RiseRTCSpeakingRing {}

impl RiseRTCSpeakingRing {
    pub fn color() -> Hsla {
        Rgba { r: 0.20, g: 0.84, b: 0.45, a: 1.0 }.into()
    }
}
