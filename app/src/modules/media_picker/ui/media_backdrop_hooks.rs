use gpui::hsla;

pub const MEDIA_EDITOR_BACKDROP: gpui::Hsla = hsla(0.0, 0.0, 0.11, 0.9);

pub const MEDIA_SEND_BACKDROP_BLUR: f32 = 24.0;

/// dim over the blur, the editors dark grey but lighter so the blurred app shows through
/// no blur means the editors own dim
pub fn media_send_backdrop(cx: &gpui::App) -> gpui::Hsla {
    if gpui::BACKDROP_BLUR_SUPPORTED && !rise_ui::swiftui::environment_values::accessibility_reduce_transparency(cx) {
        hsla(0.0, 0.0, 0.11, 0.72)
    } else {
        MEDIA_EDITOR_BACKDROP
    }
}

/// hairline around a media preview on a themed surface
/// so a black photo on the dark sheet or a white one on the light sheet still shows where it ends
pub fn media_edge_color(theme: &rise_theme::AppTheme) -> gpui::Hsla {
    theme.content_on(theme.bg._100).opacity(0.16)
}
