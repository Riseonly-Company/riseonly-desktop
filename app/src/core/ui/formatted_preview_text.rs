use std::collections::HashSet;
use std::sync::Arc;

use gpui::{App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px};
use rise_ui::swiftui::font::NATURAL_LINE_HEIGHT;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rich_text::{AttributedString, RichText};

use crate::core::ui::formatted_text_ui::{
    FormattedText, FormattedTextOptions, FormattedTextPreparation, FormattedTextRenderSeed, FormattedTextResult,
    FormattedTextStyle,
};
use crate::core::ui::spoiler_dust::SpoilerDust;

/// short message markup with no interaction, like a chat row or topic row or banner
/// bold italic strike code and spoilers keep their look, a spoiler opens only in the chat
/// code blocks and links and mentions show as plain text, result is cached per seed
#[derive(IntoElement)]
pub struct FormattedPreviewText {
    id: ElementId,
    seed: FormattedTextRenderSeed,
    preparation: Arc<FormattedTextPreparation<Arc<FormattedTextResult>>>,
    line_limit: usize,
    lead: Option<AttributedString>,
    tail_ellipsis: bool,
}

impl FormattedPreviewText {
    /// lead is drawn in front of the text in the same run, like the author name of a banner
    /// with a lead closed spoilers keep their plate and not dust
    /// the dust layer lays out the markup alone and would miss the shifted runs
    pub fn new(
        id: impl Into<ElementId>,
        markup: impl Into<String>,
        font_size: f32,
        color: Hsla,
        line_limit: usize,
        lead: Option<AttributedString>,
    ) -> Self {
        let mut style = FormattedTextStyle::new(color, color, color);
        style.font_size = font_size;
        style.line_spacing = 0.0;
        let options = FormattedTextOptions {
            no_url_format: true,
            no_url_subtitle: true,
            no_mention_format: true,
            literal_code_blocks: true,
            ..FormattedTextOptions::default()
        };
        let seed = FormattedTextRenderSeed::new(markup, style, options, Vec::new());
        let preparation = FormattedText::request_formatting(&seed);
        Self {
            id: id.into(),
            seed,
            preparation,
            line_limit,
            lead,
            tail_ellipsis: false,
        }
    }

    /// puts an ellipsis at the end of the last allowed line instead of cutting mid word
    /// opt in for boxes with a definite width like the in app banner
    /// list rows stay clipped as before
    pub fn tail_ellipsis(mut self) -> Self {
        self.tail_ellipsis = true;
        self
    }

    /// line limit without the overflow clip of gpui line_clamp
    /// a list row under min content measuring would count that as zero height
    fn line_limited(&self, mut text: gpui::Div) -> gpui::Div {
        if self.tail_ellipsis {
            text = text.min_w(px(0.0)).text_ellipsis();
        }
        text.text_style().line_clamp = Some(self.line_limit);
        text
    }

    fn text_box(&self) -> gpui::Div {
        let font_size = self.seed.style.font_size;
        let line_height = font_size * NATURAL_LINE_HEIGHT;
        let text = self.line_limited(
            div()
                .w_full()
                .swiftui_font(Font::system_size(font_size))
                .line_height(px(line_height))
                .text_color(self.seed.style.base_color),
        );
        if self.line_limit > 0 {
            text.max_h(px(line_height * self.line_limit as f32))
        } else {
            text
        }
    }
}

impl RenderOnce for FormattedPreviewText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let formatted = self.preparation.value_for_immediate_display();
        let none: HashSet<usize> = HashSet::new();
        if let Some(lead) = self.lead.clone() {
            let text = lead + formatted.body(&none, false);
            return div()
                .w_full()
                .overflow_hidden()
                .child(self.text_box().child(RichText::new(self.id.clone(), text)))
                .into_any_element();
        }
        let all = formatted.spoiler_indices();
        let text = formatted.body(&none, true);
        div()
            .w_full()
            .overflow_hidden()
            .child(
                self.text_box()
                    .child(RichText::new(self.id.clone(), text))
                    .spoiler_dust(&self.seed, None, &all, &none, window, cx),
            )
            .into_any_element()
    }
}
