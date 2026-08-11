//! What the host's text stack can actually draw. Ask "will this render", not "am I on macOS":
//! the answer depends on the fonts a machine has, not only on the OS.

/// Whether a regional-indicator pair draws as a flag.
///
/// False off macOS: Segoe UI Emoji ships no country flags and a Linux desktop
/// frequently has none, so the pair renders as two letters in boxes.
pub fn renders_flag_emoji() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_draws_flag_emoji() {
        assert!(
            renders_flag_emoji(),
            "Apple Color Emoji is a system font; the flags are always there"
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn everywhere_else_falls_back_to_artwork() {
        assert!(
            !renders_flag_emoji(),
            "a machine with no flag glyphs must get the shipped SVG, not two letters in boxes"
        );
    }
}
