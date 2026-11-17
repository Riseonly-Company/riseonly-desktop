use std::rc::Rc;

use gpui::{App, ElementId, FontWeight, IntoElement, RenderOnce, SharedString, Window, black, div, px, white};
use rise_theme::color;
use rise_ui::IconUi;
use rise_ui::swiftui::font::Font;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rounded_gradient::rounded_gradient_fill;
use rise_ui::swiftui::transition::UnitPoint;

use crate::modules::music::widgets::music_cover_placeholder::MusicCoverPlaceholder;
use crate::modules::music::widgets::music_marquee_title::MusicMarqueeTitle;
use crate::modules::music::widgets::music_track_cover_image::MusicCoverImage;

const TITLE_FONT: Font = Font::system_weight(13.0, FontWeight::SEMIBOLD);
const SUBTITLE_FONT: Font = Font::system_weight(11.0, FontWeight::NORMAL);

struct AlbumCardState {
    content_width: f32,
}

#[derive(IntoElement)]
pub struct AlbumCard {
    id: ElementId,
    title: SharedString,
    subtitle: SharedString,
    cover_colors: Vec<String>,
    cover_image_url: Option<String>,
    icon_overlay: Option<SharedString>,
    size: f32,
    corner_radius: f32,
    fills_width: bool,
    on_tap: Option<ButtonAction>,
}

impl AlbumCard {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
        cover_colors: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            cover_colors,
            cover_image_url: None,
            icon_overlay: None,
            size: 140.0,
            corner_radius: 12.0,
            fills_width: false,
            on_tap: None,
        }
    }

    pub fn cover_image_url(mut self, cover_image_url: impl Into<Option<String>>) -> Self {
        self.cover_image_url = cover_image_url.into();
        self
    }

    pub fn icon_overlay(mut self, icon_overlay: impl Into<Option<SharedString>>) -> Self {
        self.icon_overlay = icon_overlay.into();
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn fills_width(mut self, fills_width: bool) -> Self {
        self.fills_width = fills_width;
        self
    }

    pub fn on_tap(mut self, on_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_tap = Some(Rc::new(on_tap));
        self
    }

    fn child_id(&self, suffix: &'static str) -> ElementId {
        ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), SharedString::new_static(suffix))
    }

    fn text_block(&self, width: f32, cx: &App) -> Div {
        let theme = rise_ui::theme(cx);
        vstack(2.0)
            .align(HorizontalAlignment::Leading)
            .w(px(width))
            .overflow_hidden()
            .child(
                MusicMarqueeTitle::new(self.child_id("title"), self.title.clone())
                    .font(TITLE_FONT)
                    .foreground(theme.text.primary)
                    .line_height(16.0)
                    .shows_copy_menu(false)
                    .width(width),
            )
            .child(
                MusicMarqueeTitle::new(self.child_id("subtitle"), self.subtitle.clone())
                    .font(SUBTITLE_FONT)
                    .foreground(theme.text.secondary)
                    .line_height(14.0)
                    .shows_copy_menu(false)
                    .width(width),
            )
    }

    fn cover_square(&self, side: f32) -> Div {
        let corner_radius = self.corner_radius;
        let mut square = div().relative().flex_none().size(px(side)).clip_shape(Shape::continuous(corner_radius));
        if self.cover_colors.is_empty() {
            square = square.child(
                div().absolute().inset_0().child(MusicCoverPlaceholder::new().icon_scale(0.34).corner_radius(corner_radius)),
            );
        } else {
            let gradient = LinearGradient::colors(
                self.cover_colors.iter().map(|hex| color(hex)),
                UnitPoint::TOP_LEADING,
                UnitPoint::BOTTOM_TRAILING,
            );
            square = square.child(div().absolute().inset_0().child(rounded_gradient_fill(gradient, corner_radius)));
        }
        if let Some(url) = self.cover_image_url.as_ref().filter(|url| !url.is_empty()) {
            square = square.child(
                div().absolute().inset_0().child(
                    MusicCoverImage::new(self.child_id("cover"), url.clone())
                        .fallback_colors(self.cover_colors.clone())
                        .size(side)
                        .corner_radius(corner_radius)
                        .shows_loading_indicator(false),
                ),
            );
        }
        if let Some(icon) = &self.icon_overlay {
            let glyph = px(side * 0.28);
            let filled = icon.ends_with(".fill");
            square = square.child(
                zstack(Alignment::Center)
                    .absolute()
                    .inset_0()
                    .child(div().relative().top(px(2.0)).children(IconUi::sized(icon, glyph, black().opacity(0.3), filled)))
                    .children(IconUi::sized(icon, glyph, white(), filled)),
            );
        }
        square
    }
}

impl RenderOnce for AlbumCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| AlbumCardState { content_width: 0.0 });
        let content_width = state.read(cx).content_width;
        let measured = state.clone();

        let cover: Div = if self.fills_width {
            let mut slot = div().relative().w_full().aspect_square();
            if content_width > 0.0 {
                slot = slot.child(div().absolute().top_0().left_0().child(self.cover_square(content_width)));
            }
            slot.child(measure(self.child_id("width"), move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                measured.update(cx, |state, cx| {
                    if (state.content_width - width).abs() > f32::EPSILON {
                        state.content_width = width;
                        cx.notify();
                    }
                });
            }))
        } else {
            self.cover_square(self.size)
        };

        let text = (!self.fills_width || content_width > 0.0)
            .then(|| self.text_block(if self.fills_width { content_width } else { self.size }, cx));

        let mut card = button_chrome(
            vstack(6.0).align(HorizontalAlignment::Leading).min_w_0().child(cover).children(text).id(self.id.clone()),
            false,
            self.on_tap.clone(),
        );
        card = if self.fills_width { card.w_full() } else { card.max_w(px(self.size)) };
        card
    }
}
