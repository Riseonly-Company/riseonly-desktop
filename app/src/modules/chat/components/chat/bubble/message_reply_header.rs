use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, div, px,
};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::main_text::MainText;
use rise_ui::rich_text::RichTextMarkup;
use rise_ui::swiftui::button_style::HOVER_OPACITY;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rich_text::RichText;
use rise_ui::swiftui::text::TruncationMode;

use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::formatted_text_ui::{FormattedText, FormattedTextOptions, FormattedTextStyle};
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::shared::utils::message_reply_preview::MessageReplyPreview;
use crate::modules::chat::stores::message::message_actions::message_types::MessageReplyTo;

pub type MessageReplyHeaderOpenSource = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct MessageReplyHeader {
    id: ElementId,
    reply: MessageReplyTo,
    is_own: bool,
    on_open_source: Option<MessageReplyHeaderOpenSource>,
    max_width: Option<f32>,
}

impl MessageReplyHeader {
    pub fn new(id: impl Into<ElementId>, reply: MessageReplyTo) -> Self {
        Self { id: id.into(), reply, is_own: false, on_open_source: None, max_width: None }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn on_open_source(mut self, on_open_source: impl Into<Option<MessageReplyHeaderOpenSource>>) -> Self {
        self.on_open_source = on_open_source.into();
        self
    }

    /// bubble content width
    /// a one line quote answers its whole unwrapped line
    /// so without a cap the bubble ran past the chat column
    pub fn max_width(mut self, max_width: Option<f32>) -> Self {
        self.max_width = max_width.filter(|width| *width > 0.0);
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn quote(&self, theme: &AppTheme, accent: Hsla) -> gpui::Div {
        let thumb_url = self.thumb_url();
        let style = self.preview_style(theme);
        let options = FormattedTextOptions { no_url_subtitle: true, ..FormattedTextOptions::default() };
        let line = RichTextMarkup::preview_line(&self.preview_text(thumb_url.is_some()));
        let formatted = FormattedText::format(&line, &style, &options, &[]);

        hstack(6.0)
            .w_full()
            .when_some(self.max_width, |row, width| row.max_w(px(width)))
            .min_h(px(34.0))
            .child(div().flex_none().frame_size(3.0, 34.0).continuous_corners(1.5).bg(accent))
            .when_some(thumb_url, |row, thumb| {
                row.child(
                    div().flex_none().frame_size(32.0, 32.0).clip_shape(Shape::continuous(4.0)).child(
                        CleverImageUi::new(thumb)
                            .id(self.child_id("thumb"))
                            .content_mode(ContentMode::Fill)
                            .corner_radius(4.0)
                            .persist_to_disk(true)
                            .maximum_decode_pixel_dimension(96_i64),
                    ),
                )
            })
            .child(
                vstack(1.0)
                    .align(HorizontalAlignment::Leading)
                    .flex_grow(1.0)
                    .flex_shrink(1.0)
                    .min_w_0()
                    .child(
                        MainText::new(self.reply.sender_name.clone())
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(accent)
                            .number_of_lines(1),
                    )
                    .child(
                        TruncationMode::Tail.apply(
                            div()
                                .min_w_0()
                                .line_clamp(1)
                                .swiftui_font(Font::system_size(style.font_size))
                                .text_color(style.base_color)
                                .child(RichText::new(self.child_id("preview"), formatted.attributed_body.clone())),
                        ),
                    ),
            )
            .child(spacer_min_w(0.0))
    }

    fn preview_style(&self, theme: &AppTheme) -> FormattedTextStyle {
        let muted = if self.is_own { theme.content_on(theme.primary._100).opacity(0.78) } else { theme.text.secondary };
        let mut style = FormattedTextStyle::new(muted, muted, muted);
        style.font_size = 14.0;
        style.code_foreground = Some(muted);
        style.code_background = Some(muted.opacity(0.16));
        style
    }

    fn preview_text(&self, has_thumbnail: bool) -> String {
        MessageReplyPreview::label(&self.reply.content_type, &self.reply.content, has_thumbnail)
    }

    fn thumb_url(&self) -> Option<String> {
        MessageReplyPreview::thumbnail_source(&self.reply.media_items)
    }
}

impl RenderOnce for MessageReplyHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let accent = if self.is_own { theme.content_on(theme.primary._100).opacity(0.85) } else { theme.primary._100 };
        let quote = self.quote(&theme, accent);

        match self.on_open_source.clone() {
            Some(on_open_source) => quote
                .cursor_pointer()
                .hover(|style: StyleRefinement| style.opacity(HOVER_OPACITY))
                .bubble_tap_target_priority(self.id.clone(), move |window: &mut Window, cx: &mut App| {
                    on_open_source(window, cx)
                })
                .into_any_element(),
            None => quote.into_any_element(),
        }
    }
}
