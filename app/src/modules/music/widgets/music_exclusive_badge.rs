use gpui::{App, FontWeight, IntoElement, RenderOnce, Window, div, px};
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::swiftui::prelude::*;

#[derive(IntoElement, Default)]
pub struct MusicExclusiveBadge {
    compact: bool,
    icon_only: bool,
}

impl MusicExclusiveBadge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    pub fn icon_only(mut self, icon_only: bool) -> Self {
        self.icon_only = icon_only;
        self
    }
}

impl RenderOnce for MusicExclusiveBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let primary = rise_ui::theme(cx).primary._100;
        let compact = self.compact;
        let title_font = Font::system_weight(if compact { 10.0 } else { 11.0 }, FontWeight::SEMIBOLD);
        let glyph = px(if compact { 9.0 } else { 10.0 });

        hstack(if compact { 3.0 } else { 4.0 })
            .id("music-exclusive-badge")
            .aria_label(tr("music_exclusive_badge"))
            .flex_none()
            .px(px(if compact { 6.0 } else { 8.0 }))
            .py(px(if compact { 2.0 } else { 3.0 }))
            .bg(primary.opacity(0.14))
            .capsule()
            .child(
                zstack(Alignment::Center)
                    .child(div().opacity(0.0).swiftui_font(title_font).child(" "))
                    .children(IconUi::sized("sparkles", glyph, primary, false)),
            )
            .when(!self.icon_only, |badge| {
                badge.child(
                    div()
                        .whitespace_nowrap()
                        .swiftui_font(title_font)
                        .text_color(primary)
                        .child(tr("music_exclusive_badge")),
                )
            })
    }
}
