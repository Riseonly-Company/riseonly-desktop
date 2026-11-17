pub const FALLBACK_FRAMES_PER_SECOND: u32 = 60;

/// highest frame rate the display holding the window shows
/// call it on the main thread where gpui hands out the window
pub fn maximum_frames_per_second(window: &gpui::Window) -> u32 {
    frames_per_second_or_fallback(super::os::reported_refresh_rate(window))
}

/// policy over what an os layer reports, none or 0 or 1 mean no answer and give the fallback rate
/// windows reports 0 or 1 for the default rate and wayland 0 for a mode without one
/// anything else is the display rate
pub fn frames_per_second_or_fallback(reported: Option<u32>) -> u32 {
    match reported {
        Some(rate) if rate > 1 => rate,
        _ => FALLBACK_FRAMES_PER_SECOND,
    }
}

/// wayland gives refresh in mhz, this rounds to whole fps (59940 is 60 and 143856 is 144)
pub fn frames_per_second_from_millihertz(millihertz: u32) -> u32 {
    (millihertz + 500) / 1000
}
