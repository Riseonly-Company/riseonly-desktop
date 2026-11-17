use gpui::{Edges, Font, Hsla, Pixels, SharedString, TextRun, Window, px};

/// what text view size measuring uses, the text and its font and the container geometry
#[derive(Clone, Debug)]
pub struct UITextViewContentLayout {
    pub text: SharedString,
    pub font: Font,
    pub font_size: Pixels,
    pub line_height: Pixels,
    pub text_container_inset: Edges<Pixels>,
    pub line_fragment_padding: Pixels,
    pub width_tracks_text_view: bool,
}

impl UITextViewContentLayout {
    pub fn new(text: impl Into<SharedString>, font: Font, font_size: Pixels, line_height: Pixels) -> Self {
        Self {
            text: text.into(),
            font,
            font_size,
            line_height,
            text_container_inset: Edges { top: px(8.0), right: px(0.0), bottom: px(8.0), left: px(0.0) },
            line_fragment_padding: px(5.0),
            width_tracks_text_view: true,
        }
    }
}

pub trait UITextViewContentHeight {
    fn content_height(&self, for_view_width: Pixels, window: &Window) -> Pixels;
}

impl UITextViewContentHeight for UITextViewContentLayout {
    fn content_height(&self, for_view_width: Pixels, window: &Window) -> Pixels {
        let inset = self.text_container_inset;
        let pad = self.line_fragment_padding;
        let horizontal_chrome = inset.left + inset.right + pad * 2.0;
        let content_w = px(44.0).max(for_view_width - horizontal_chrome);
        let width = if for_view_width > px(0.0) { for_view_width } else { content_w + horizontal_chrome };
        let wrap_width = if self.width_tracks_text_view {
            (width - horizontal_chrome).max(px(0.0))
        } else {
            content_w
        };

        let text = if self.text.is_empty() { SharedString::from(" ") } else { self.text.clone() };
        let run = TextRun {
            len: text.len(),
            font: self.font.clone(),
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let text_height = window
            .text_system()
            .shape_text(text, self.font_size, &[run], Some(wrap_width), None)
            .map(|lines| {
                lines
                    .iter()
                    .map(|line| line.size(self.line_height).height)
                    .fold(px(0.0), |sum, height| sum + height)
            })
            .unwrap_or(self.line_height);
        text_height + inset.top + inset.bottom
    }
}
