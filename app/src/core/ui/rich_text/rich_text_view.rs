use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, ElementId, IntoElement, RenderOnce, SharedString, Window};
use rise_core::foundation::foundation_url::Url;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::rich_text::{RichBlock, RichTextMarkup, RiseIndexedRichBlock};
use rise_ui::swiftui::open_url::{OpenURLAction, OpenURLActionResult, OpenURLKey, open_url};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::formatted_text_ui::{FormattedText, FormattedTextOptions, FormattedTextStyle, FormattedTextUi};
use crate::core::ui::rich_text::code_block_view::CodeBlockView;
use crate::modules::chat::components::chat::bubble::chat_mention_tap_layer::{
    ChatMentionTapLayerModifiers, ChatMentionTapsOptions,
};

/// callback for a tap on a mention or link or hashtag, gets the tapped value
pub type RichTextTap = Rc<dyn Fn(&str, &mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RichSurface {
    Standard,
    OwnBubble,
    OtherBubble,
}

impl FormattedTextStyle {
    /// standard text style for the theme and font size, line spacing is 3
    pub fn app_standard(theme: &AppTheme, font_size: f32) -> FormattedTextStyle {
        Self::app_standard_line_spacing(theme, font_size, 3.0)
    }

    /// same as app_standard but with your own line spacing
    pub fn app_standard_line_spacing(theme: &AppTheme, font_size: f32, line_spacing: f32) -> FormattedTextStyle {
        let mut style = FormattedTextStyle::new(
            theme.text.primary,
            FormattedTextStyle::standard_link_color(),
            theme.text.secondary,
        );
        style.font_size = (font_size + ThemeStore::text_delta_snapshot()).max(1.0);
        style.line_spacing = line_spacing;
        style.code_foreground = Some(theme.text.primary);
        style.code_background = Some(theme.text.primary.opacity(0.16));
        style
    }
}

#[derive(IntoElement)]
pub struct RichTextView {
    id: ElementId,
    text: String,
    style: FormattedTextStyle,
    block_spacing: f32,
    code_font_size: f32,
    options: FormattedTextOptions,
    on_mention_tap: Option<RichTextTap>,
    on_link_tap: Option<RichTextTap>,
    on_hashtag_tap: Option<RichTextTap>,
}

impl RichTextView {
    pub fn new(id: impl Into<ElementId>, text: impl Into<String>, style: FormattedTextStyle) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            style,
            block_spacing: 8.0,
            code_font_size: 13.5,
            options: FormattedTextOptions::default(),
            on_mention_tap: None,
            on_link_tap: None,
            on_hashtag_tap: None,
        }
    }

    pub fn block_spacing(mut self, block_spacing: f32) -> Self {
        self.block_spacing = block_spacing;
        self
    }

    pub fn code_font_size(mut self, code_font_size: f32) -> Self {
        self.code_font_size = code_font_size;
        self
    }

    pub fn options(mut self, options: FormattedTextOptions) -> Self {
        self.options = options;
        self
    }

    pub fn on_mention_tap(mut self, on_mention_tap: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_mention_tap = Some(Rc::new(on_mention_tap));
        self
    }

    pub fn on_link_tap(mut self, on_link_tap: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_link_tap = Some(Rc::new(on_link_tap));
        self
    }

    pub fn on_hashtag_tap(mut self, on_hashtag_tap: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_hashtag_tap = Some(Rc::new(on_hashtag_tap));
        self
    }

    fn child_id(&self, name: impl Into<SharedString>) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), name.into())
    }

    fn paragraph_view(&self, id: ElementId, paragraph: &str, open_url: &OpenURLAction) -> AnyElement {
        let text = FormattedTextUi::new(id.clone(), paragraph.to_owned(), self.style.clone())
            .options(self.options.clone())
            .shows_subtitle(false)
            .frame_max_width(Alignment::Leading);
        if let Some(on_mention_tap) = self.on_mention_tap.clone() {
            let taps = text.chat_mention_taps(
                ElementId::NamedChild(Arc::new(id), SharedString::new_static("mention-taps")),
                paragraph.to_owned(),
                self.style.clone(),
                ChatMentionTapsOptions {
                    options: self.options.clone(),
                    on_link: self.on_link_tap.clone(),
                    ..ChatMentionTapsOptions::default()
                },
                on_mention_tap,
            );
            div().w_full().min_w_0().child(taps).into_any_element()
        } else if let Some(on_hashtag_tap) = self.on_hashtag_tap.clone() {
            let open_url = open_url.clone();
            let action = OpenURLAction::new(move |url: &Url, window, cx| {
                if let Some(tag) = FormattedText::hashtag(url) {
                    on_hashtag_tap(&tag, window, cx);
                } else {
                    open_url.call_as_function(url, window, cx);
                }
                OpenURLActionResult::Handled
            });
            env_scope::<OpenURLKey>(action).child(text).into_any_element()
        } else {
            text.into_any_element()
        }
    }
}

impl RenderOnce for RichTextView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let open_url = open_url(cx);
        let blocks = RichTextMarkup::blocks(&self.text);
        let entries = RiseIndexedRichBlock::list(&blocks);
        let mut stack = vstack(self.block_spacing).align(HorizontalAlignment::Leading);
        for entry in entries {
            let id = self.child_id(entry.id);
            let child = match entry.block {
                RichBlock::Paragraph(paragraph) => self.paragraph_view(id, &paragraph, &open_url),
                RichBlock::Code { language, code } => CodeBlockView::new(id, language, code)
                    .font_size(self.code_font_size)
                    .frame_max_width(Alignment::Leading)
                    .into_any_element(),
            };
            stack = stack.child(child);
        }
        stack.w_full().min_w_0().frame_max_width(Alignment::Leading)
    }
}

