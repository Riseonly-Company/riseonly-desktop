use gpui::{App, Hsla, IntoElement, RenderOnce, Window, black, container_query, div, px, white};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_theme::avatar_palette::AvatarPalette;
use rise_theme::color;
use rise_ui::IconUi;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::UnitPoint;

use crate::modules::music::shared::utils::music::music_media_url::MusicMediaURL;

pub enum MusicCoverMedia {}

impl MusicCoverMedia {
    pub fn has_remote_cover(url: &str) -> bool {
        trimming_whitespaces_and_newlines(url).to_lowercase().starts_with("http") && !Self::is_generated_avatar(url)
    }

    /// an artist with no cover has the servers letter avatar
    /// drawn as a cover it was a grey letter on transparent and a white smear in the hero
    pub fn is_generated_avatar(url: &str) -> bool {
        AvatarPalette::is_generated_avatar_url(Some(url))
    }

    /// fetch candidates of a cover
    /// a generated letter avatar is no cover so the placeholder draws
    pub fn fetch_sources(url: &str) -> Vec<String> {
        if Self::is_generated_avatar(url) {
            return Vec::new();
        }
        MusicMediaURL::cover_url_strings_for_fetch(Some(url))
    }
}

#[derive(IntoElement)]
pub struct MusicCoverPlaceholder {
    icon_scale: f32,
    corner_radius: f32,
    colors: Vec<String>,
    shows_note: bool,
}

impl Default for MusicCoverPlaceholder {
    fn default() -> Self {
        Self { icon_scale: 0.34, corner_radius: 0.0, colors: Vec::new(), shows_note: true }
    }
}

impl MusicCoverPlaceholder {
    const DEFAULT_COLORS: [&'static str; 3] = ["#5C1A2E", "#C44569", "#8B2942"];

    pub fn new() -> Self {
        Self::default()
    }

    pub fn icon_scale(mut self, icon_scale: f32) -> Self {
        self.icon_scale = icon_scale;
        self
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn colors(mut self, colors: Vec<String>) -> Self {
        self.colors = colors;
        self
    }

    pub fn shows_note(mut self, shows_note: bool) -> Self {
        self.shows_note = shows_note;
        self
    }

    fn resolved_colors(&self) -> Vec<Hsla> {
        if self.colors.is_empty() {
            Self::DEFAULT_COLORS.iter().map(|hex| color(hex)).collect()
        } else {
            self.colors.iter().map(|hex| color(hex)).collect()
        }
    }

    /// note glyph alone, for a caller that paints the gradient itself
    pub fn note(side: f32, icon_scale: f32) -> impl IntoElement {
        let glyph = px((side * icon_scale).max(14.0));
        zstack(Alignment::Center)
            .size_full()
            .child(
                div()
                    .relative()
                    .top(px(2.0))
                    .children(IconUi::sized("music.note", glyph, black().opacity(0.22), false)),
            )
            .children(IconUi::sized("music.note", glyph, white().opacity(0.9), false))
    }
}

impl RenderOnce for MusicCoverPlaceholder {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let gradient = LinearGradient::colors(
            self.resolved_colors(),
            UnitPoint::TOP_LEADING,
            UnitPoint::BOTTOM_TRAILING,
        );
        let icon_scale = self.icon_scale;

        div()
            .relative()
            .size_full()
            .clip_shape(Shape::continuous(self.corner_radius))
            .child(div().absolute().inset_0().child(gradient.view()))
            .when(self.shows_note, |placeholder| {
                placeholder.child(
                    container_query(move |size, _, _| {
                        let side = f32::from(size.width).min(f32::from(size.height));
                        Self::note(side, icon_scale)
                    })
                    .absolute()
                    .inset_0(),
                )
            })
    }
}
