use gpui::{
    App, Div, FontFeatures, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce,
    SharedString, StyleRefinement, Styled, Window, div, px,
};
use rise_theme::theme::theme_store::ThemeStore;

use crate::main_text::{
    TextAlignT, ignores_text_size_delta, line_spacing, margin_edges, natural_line_height,
    scaled_text_floor, scaling_factor, spaced_element, spaced_lines, system_font, text_size,
};
use crate::swiftui::environment::layout_direction;
use crate::swiftui::minimum_scale_factor::MinimumScaleText;
use crate::swiftui::text::TruncationMode;

#[derive(IntoElement)]
pub struct SecondaryText {
    text: SharedString,
    px: f32,
    tac: TextAlignT,
    color: Option<Hsla>,
    line_height: Option<f32>,
    ml: f32,
    mt: f32,
    mb: f32,
    strikethrough: bool,
    line_limit: Option<usize>,
    minimum_scale_factor: Option<f32>,
    glyph_features: FontFeatures,
    base: Div,
}

impl SecondaryText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            px: 16.0,
            tac: TextAlignT::Auto,
            color: None,
            line_height: None,
            ml: 0.0,
            mt: 0.0,
            mb: 0.0,
            strikethrough: false,
            line_limit: None,
            minimum_scale_factor: None,
            glyph_features: FontFeatures::default(),
            base: div(),
        }
    }

    /// opentype features of the font for this line
    /// the system font set in render would replace font_features so they travel with the font
    pub fn glyph_features(mut self, glyph_features: FontFeatures) -> Self {
        self.glyph_features = glyph_features;
        self
    }

    pub fn px(mut self, px: f32) -> Self {
        self.px = px;
        self
    }

    pub fn tac(mut self, tac: TextAlignT) -> Self {
        self.tac = tac;
        self
    }

    pub fn color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.color = color.into();
        self
    }

    pub fn line_height(mut self, line_height: impl Into<Option<f32>>) -> Self {
        self.line_height = line_height.into();
        self
    }

    pub fn ml(mut self, ml: f32) -> Self {
        self.ml = ml;
        self
    }

    pub fn mt(mut self, mt: f32) -> Self {
        self.mt = mt;
        self
    }

    pub fn mb(mut self, mb: f32) -> Self {
        self.mb = mb;
        self
    }

    pub fn strikethrough(mut self, strikethrough: bool) -> Self {
        self.strikethrough = strikethrough;
        self
    }

    /// line limit set outside the text, zero means no limit
    pub fn line_limit(mut self, line_limit: impl Into<Option<usize>>) -> Self {
        self.line_limit = line_limit.into().filter(|lines| *lines > 0);
        self
    }

    /// minimum scale factor set outside the text, shrinks to fit the line limit then truncates
    pub fn minimum_scale_factor(mut self, factor: f32) -> Self {
        self.minimum_scale_factor = Some(factor);
        self
    }

    pub(crate) fn effective_line_limit(&self) -> Option<usize> {
        self.line_limit
    }

    pub(crate) fn effective_minimum_scale_factor(&self) -> Option<f32> {
        scaling_factor(self.minimum_scale_factor)
    }

    pub(crate) fn resolved_color(&self, text_secondary: Hsla) -> Hsla {
        self.color.unwrap_or(text_secondary)
    }
}

impl Styled for SecondaryText {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for SecondaryText {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (text_secondary, text_delta) = {
            let store = ThemeStore::shared(cx);
            let store = store.read(cx);
            (store.current_theme().text.secondary, store.text_delta())
        };
        let resolved_color = self.resolved_color(text_secondary);
        let size = text_size(self.px, text_delta, ignores_text_size_delta(cx));
        let line_spacing = line_spacing(self.line_height, size);
        let direction = layout_direction(cx);

        let font = system_font(FontWeight::NORMAL, self.glyph_features.clone());
        let natural = natural_line_height(&font, size, cx);
        let line_limit = self.effective_line_limit();
        let scale_factor = self.effective_minimum_scale_factor();

        let mut base = self
            .base
            .font(font)
            .text_size(px(size))
            .line_height(px(natural + line_spacing))
            .text_color(resolved_color)
            .text_align(self.tac.text_alignment().resolve(direction));
        if self.strikethrough {
            base = base.line_through();
        }
        let content = match scale_factor {
            Some(factor) => {
                let scaled = MinimumScaleText::new(self.text, factor)
                    .line_limit(line_limit)
                    .truncation(TruncationMode::Tail)
                    .line_spacing(line_spacing);
                spaced_element(scaled, line_spacing)
            }
            None => spaced_lines(self.text, line_spacing),
        };
        if scale_factor.is_some() && line_limit.is_some() {
            base = scaled_text_floor(base);
        }
        if scale_factor.is_none()
            && let Some(lines) = line_limit
        {
            if lines == 1 {
                if base.style().min_size.width.is_none() {
                    base = base.min_w_0();
                }
                base = base.whitespace_nowrap();
            }
            base = TruncationMode::Tail.apply(base.line_clamp(lines));
        }
        let edges = margin_edges(self.mt, self.ml, 0.0, self.mb, direction);
        if edges.is_zero() {
            base.child(content)
        } else {
            base.child(edges.apply(div()).child(content))
        }
    }
}
