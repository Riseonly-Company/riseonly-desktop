use gpui::{App, FontWeight, SharedString, Window, hsla, white};
use rise_ui::IconUi;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;

/// shared corner capsule for media grids like stories
/// riffs and riff publications so plates cant drift apart
/// caller owns placement, corner alignment and the 6pt inset from the cell edge
#[derive(IntoElement)]
pub struct MediaCountPlate {
    text: SharedString,
    system_image: Option<SharedString>,
}

impl MediaCountPlate {
    pub fn new(text: impl Into<SharedString>, system_image: Option<SharedString>) -> Self {
        Self { text: text.into(), system_image }
    }

    pub fn view_count(view_count: i64) -> Self {
        Self::new(RiseFormat::compact_count(view_count.max(0)), Some(SharedString::from("eye.fill")))
    }
}

impl RenderOnce for MediaCountPlate {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        hstack(3.)
            .when_some(self.system_image, |row, system_image| {
                row.children(IconUi::sized(&system_image, px(9.), white(), false))
            })
            .child(
                div()
                    .swiftui_font(Font::system(11., FontWeight::SEMIBOLD, FontDesign::Default))
                    .text_color(white())
                    .child(self.text),
            )
            .padding_edges(EdgeSet::HORIZONTAL, 5.)
            .padding_edges(EdgeSet::VERTICAL, 2.)
            .bg(hsla(0., 0., 0., 0.55))
            .capsule()
    }
}
