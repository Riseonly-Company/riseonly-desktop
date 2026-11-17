use std::rc::Rc;

use gpui::{App, FontWeight, IntoElement, ParentElement, Pixels, RenderOnce, Size, Styled, Window, black, div, px, size, white};
use rise_i18n::tr;
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize, LoaderUiType};
use rise_ui::swiftui::frame::{EdgeSet, SwiftUiAlignedFrame, SwiftUiFrame};
use rise_ui::swiftui::shapes::SwiftUiShapes;
use rise_ui::swiftui::stacks::{Alignment, HorizontalAlignment, StackAlign, spacer_min_h, vstack, zstack};

use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

#[derive(IntoElement)]
pub struct PostMediaUploadOverlay {
    pub post: Rc<GetPostFeedResponse>,
    pub cell_size: Size<Pixels>,
}

impl PostMediaUploadOverlay {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self { post: post.into(), cell_size: size(px(120.0), px(150.0)) }
    }

    pub fn cell_size(mut self, cell_size: Size<Pixels>) -> Self {
        self.cell_size = cell_size;
        self
    }

    fn ring_progress(&self) -> f64 {
        if self.post.is_compressing {
            return (self.post.compression_progress / 100.0).clamp(0.08, 1.0);
        }
        if self.post.upload_progress >= 99.5 {
            return 1.0;
        }
        (self.post.upload_progress / 100.0).clamp(0.08, 1.0)
    }

    fn uses_determinate_ring(&self) -> bool {
        self.post.is_compressing || self.post.upload_progress > 0.0
    }

    fn status_text(&self) -> String {
        if self.post.is_compressing {
            return format!("{} {}%", tr("chat_media_upload_processing"), self.post.compression_progress as i64);
        }
        if self.post.upload_progress >= 99.5 {
            return tr("chat_media_upload_sending");
        }
        if self.post.upload_progress > 0.0 {
            return format!("{} {}%", tr("create_post_uploading"), self.post.upload_progress as i64);
        }
        tr("chat_media_upload_loading")
    }
}

impl RenderOnce for PostMediaUploadOverlay {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let side = f32::from(self.cell_size.width).min(f32::from(self.cell_size.height));
        let status = div()
            .text_size(px(10.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(white().opacity(0.95))
            .line_clamp(2)
            .padding_edges(EdgeSet::HORIZONTAL, 7.0)
            .padding_edges(EdgeSet::VERTICAL, 4.0)
            .capsule()
            .bg(black().opacity(0.45))
            .child(self.status_text());
        let ring = LoaderUi::new()
            .size(LoaderUiSize::Custom((side * 0.38).min(44.0)))
            .progress(self.ring_progress())
            .r#type(LoaderUiType::Progress)
            .thickness(2.2)
            .track_color(Some(white().opacity(0.28)))
            .ring_animated(!self.uses_determinate_ring())
            .speed(3.0)
            .gap(3.0);

        zstack(Alignment::TopLeading)
            .size_full()
            .child(div().size_full().bg(black().opacity(0.32)))
            .child(
                vstack(0.0)
                    .size_full()
                    .align(HorizontalAlignment::Leading)
                    .child(div().p(px(6.0)).child(status))
                    .child(spacer_min_h(0.0)),
            )
            .child(ring.frame_max(Alignment::Center))
    }
}
