use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Div, Font, FontFeatures, FontStyle, FontWeight, HighlightStyle, Hsla, IntoElement,
    ParentElement, RenderOnce, SharedString, StyleRefinement, StyledText, Styled, Window, div, px,
};
use rise_theme::FONT_FAMILY;
use rise_theme::theme::theme_store::ThemeStore;

use crate::expandable_text_ui::right_to_left_hand_lined;
use crate::swiftui::environment::{EnvKey, LayoutDirection, env, layout_direction};
use crate::swiftui::minimum_scale_factor::MinimumScaleText;
use crate::swiftui::stacks::Alignment;
use crate::swiftui::text::{TextAlignment, TruncationMode};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAlignT {
    #[default]
    Auto,
    Left,
    Center,
    Right,
}

impl TextAlignT {
    pub fn text_alignment(&self) -> TextAlignment {
        match self {
            Self::Auto | Self::Left => TextAlignment::Leading,
            Self::Center => TextAlignment::Center,
            Self::Right => TextAlignment::Trailing,
        }
    }

    pub fn frame_alignment(&self) -> Alignment {
        match self {
            Self::Auto | Self::Left => Alignment::Leading,
            Self::Center => Alignment::Center,
            Self::Right => Alignment::Trailing,
        }
    }
}

#[derive(IntoElement)]
pub struct MainText {
    text: SharedString,
    px: f32,
    tac: TextAlignT,
    font_weight: FontWeight,
    color: Option<Hsla>,
    primary: bool,
    secondary: bool,
    line_height: Option<f32>,
    margin_top: f32,
    m_r: f32,
    m_l: f32,
    m_b: f32,
    number_of_lines: usize,
    monospaced_digits: bool,
    line_limit: Option<usize>,
    truncation: TruncationMode,
    minimum_scale_factor: Option<f32>,
    color_runs: Vec<(Range<usize>, Hsla)>,
    right_to_left_wrap_width: Option<f32>,
    base: Div,
}

impl MainText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            px: 16.0,
            tac: TextAlignT::Auto,
            font_weight: FontWeight::NORMAL,
            color: None,
            primary: false,
            secondary: false,
            line_height: None,
            margin_top: 0.0,
            m_r: 0.0,
            m_l: 0.0,
            m_b: 0.0,
            number_of_lines: 0,
            monospaced_digits: false,
            line_limit: None,
            truncation: TruncationMode::Tail,
            minimum_scale_factor: None,
            color_runs: Vec::new(),
            right_to_left_wrap_width: None,
            base: div(),
        }
    }

    /// text wraps within width (plate width less padding)
    /// a right to left paragraph is then broken by its shaped width
    pub fn right_to_left_wrap_width(mut self, width: f32) -> Self {
        self.right_to_left_wrap_width = Some(width);
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

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.font_weight = font_weight;
        self
    }

    pub fn color(mut self, color: impl Into<Option<Hsla>>) -> Self {
        self.color = color.into();
        self
    }

    pub fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }

    pub fn secondary(mut self, secondary: bool) -> Self {
        self.secondary = secondary;
        self
    }

    pub fn line_height(mut self, line_height: impl Into<Option<f32>>) -> Self {
        self.line_height = line_height.into();
        self
    }

    pub fn margin_top(mut self, margin_top: f32) -> Self {
        self.margin_top = margin_top;
        self
    }

    pub fn m_r(mut self, m_r: f32) -> Self {
        self.m_r = m_r;
        self
    }

    pub fn m_l(mut self, m_l: f32) -> Self {
        self.m_l = m_l;
        self
    }

    pub fn m_b(mut self, m_b: f32) -> Self {
        self.m_b = m_b;
        self
    }

    pub fn number_of_lines(mut self, number_of_lines: usize) -> Self {
        self.number_of_lines = number_of_lines;
        self
    }

    pub fn monospaced_digits(mut self, monospaced_digits: bool) -> Self {
        self.monospaced_digits = monospaced_digits;
        self
    }

    /// line limit set from outside the text
    pub fn line_limit(mut self, line_limit: impl Into<Option<usize>>) -> Self {
        self.line_limit = line_limit.into();
        self
    }

    /// truncation mode set from outside the text
    pub fn truncation(mut self, truncation: TruncationMode) -> Self {
        self.truncation = truncation;
        self
    }

    /// minimum scale factor set outside the text, shrinks to fit the line limit and then truncates
    pub fn minimum_scale_factor(mut self, factor: f32) -> Self {
        self.minimum_scale_factor = Some(factor);
        self
    }

    /// per range text colors over the resolved color
    /// gpui text has no gradient mask so a gradient is drawn as colors sampled along the text
    pub fn color_runs(mut self, color_runs: Vec<(Range<usize>, Hsla)>) -> Self {
        self.color_runs = color_runs;
        self
    }

    /// factor that changes rendering, one or more means the text never shrinks
    pub(crate) fn effective_minimum_scale_factor(&self) -> Option<f32> {
        scaling_factor(self.minimum_scale_factor)
    }

    pub(crate) fn resolved_color(
        &self,
        primary_100: Hsla,
        text_primary: Hsla,
        text_secondary: Hsla,
        fill_content_color: Option<Hsla>,
    ) -> Hsla {
        if self.primary {
            return primary_100;
        }
        if self.secondary {
            return text_secondary;
        }
        if let Some(fill_content_color) = fill_content_color
            && (self.color.is_none() || self.color == Some(gpui::white()))
        {
            return fill_content_color;
        }
        self.color.unwrap_or(text_primary)
    }

    pub(crate) fn effective_line_limit(&self) -> Option<usize> {
        MainTextLineLimit {
            lines: self.number_of_lines,
        }
        .limit()
        .or(self.line_limit)
    }
}

impl Styled for MainText {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for MainText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (primary_100, text_primary, text_secondary, text_delta) = {
            let store = ThemeStore::shared(cx);
            let store = store.read(cx);
            let theme = store.current_theme();
            (
                theme.primary._100,
                theme.text.primary,
                theme.text.secondary,
                store.text_delta(),
            )
        };
        let resolved_color = self.resolved_color(
            primary_100,
            text_primary,
            text_secondary,
            fill_content_color(cx),
        );
        let size = text_size(self.px, text_delta, ignores_text_size_delta(cx));
        let line_spacing = line_spacing(self.line_height, size);
        let direction = layout_direction(cx);

        let features = if self.monospaced_digits {
            monospaced_digit_features()
        } else {
            FontFeatures::default()
        };
        let font = system_font(self.font_weight, features);
        let natural = natural_line_height(&font, size, cx);
        let line_limit = MainTextLineLimit {
            lines: self.effective_line_limit().unwrap_or(0),
        };
        let scale_factor = self.effective_minimum_scale_factor();
        let text = match self.right_to_left_wrap_width {
            Some(width) if scale_factor.is_none() && self.color_runs.is_empty() => right_to_left_hand_lined(
                window.text_system(),
                &self.text,
                &font,
                resolved_color,
                px(size),
                width,
            )
            .unwrap_or_else(|| self.text.clone()),
            _ => self.text.clone(),
        };

        let base = self
            .base
            .font(font)
            .text_size(px(size))
            .line_height(px(natural + line_spacing))
            .text_color(resolved_color)
            .text_align(self.tac.text_alignment().resolve(direction));
        let content = match scale_factor {
            Some(factor) => {
                let scaled = MinimumScaleText::new(text, factor)
                    .line_limit(line_limit.limit())
                    .truncation(self.truncation)
                    .line_spacing(line_spacing)
                    .color_runs(self.color_runs);
                spaced_element(scaled, line_spacing)
            }
            None if !self.color_runs.is_empty() => {
                let highlights = self.color_runs.into_iter().map(|(range, color)| {
                    (range, HighlightStyle { color: Some(color), ..HighlightStyle::default() })
                });
                spaced_element(StyledText::new(text).with_highlights(highlights), line_spacing)
            }
            None => spaced_lines(text, line_spacing),
        };
        let (base, content) = match (scale_factor, line_limit.limit()) {
            (Some(_), Some(_)) => (scaled_text_floor(base), content),
            (Some(_), None) => (base, content),
            (None, Some(lines)) => (
                line_limit.body(base, self.truncation),
                line_limit.cap(content, natural + line_spacing, lines),
            ),
            (None, None) => (line_limit.body(base, self.truncation), content),
        };
        let edges = margin_edges(self.margin_top, self.m_l, self.m_r, self.m_b, direction);
        if edges.is_zero() {
            base.child(content)
        } else {
            base.child(edges.apply(div()).child(content))
        }
    }
}

/// set on fixed height chrome like page headers
/// the text size setting scales messages and captions but a scaled title would outgrow its bar
pub struct IgnoresTextSizeDeltaKey;

impl EnvKey for IgnoresTextSizeDeltaKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        false
    }
}

/// ink picked by a filled surface for main text without its own color inside
/// so a label on the red fill isnt theme black
pub struct FillContentColorKey;

impl EnvKey for FillContentColorKey {
    type Value = Option<Hsla>;

    fn default_value() -> Self::Value {
        None
    }
}

pub fn ignores_text_size_delta(cx: &App) -> bool {
    env::<IgnoresTextSizeDeltaKey>(cx)
}

/// fill content color if a filled surface set one
pub fn fill_content_color(cx: &App) -> Option<Hsla> {
    env::<FillContentColorKey>(cx)
}

/// zero leaves the limit to the caller
/// an inner line limit none overrode one set outside, so one line titles wrapped
struct MainTextLineLimit {
    lines: usize,
}

impl MainTextLineLimit {
    fn limit(&self) -> Option<usize> {
        if self.lines > 0 { Some(self.lines) } else { None }
    }

    fn body(&self, content: Div, truncation: TruncationMode) -> Div {
        match self.limit() {
            Some(1) => {
                let mut content = content;
                if content.style().min_size.width.is_none() {
                    content = content.min_w_0();
                }
                truncation.apply(content.whitespace_nowrap().line_clamp(1))
            }
            Some(lines) => truncation.apply(content.line_clamp(lines)),
            None => content,
        }
    }

    fn cap(&self, content: Div, line_height: f32, lines: usize) -> Div {
        content.max_h(px(line_height * lines as f32)).overflow_y_hidden()
    }
}

/// a factor of one or more leaves the text as it is, so it renders on the plain path
pub(crate) fn scaling_factor(factor: Option<f32>) -> Option<f32> {
    factor.filter(|factor| *factor < 1.0)
}

pub(crate) fn text_size(px: f32, text_delta: f32, ignores_text_size_delta: bool) -> f32 {
    let delta = if ignores_text_size_delta { 0.0 } else { text_delta };
    (px + delta).max(1.0)
}

/// extra space between lines for a requested line height
pub(crate) fn line_spacing(line_height: Option<f32>, size: f32) -> f32 {
    match line_height {
        Some(lh) => (lh - size).max(0.0),
        None => 0.0,
    }
}

/// system font, the product ships one family on every os
pub(crate) fn system_font(weight: FontWeight, features: FontFeatures) -> Font {
    Font {
        family: SharedString::new_static(FONT_FAMILY),
        features,
        fallbacks: None,
        weight,
        style: FontStyle::Normal,
    }
}

/// monospaced digit features, opentype tabular figures
pub(crate) fn monospaced_digit_features() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".to_string(), 1)]))
}

pub(crate) fn natural_line_height(font: &Font, size: f32, cx: &App) -> f32 {
    let text_system = cx.text_system();
    let font_id = text_system.resolve_font(font);
    let ascent = f32::from(text_system.ascent(font_id, px(size)));
    let descent = f32::from(text_system.descent(font_id, px(size)));
    ascent + descent.abs()
}

pub(crate) fn spaced_lines(text: SharedString, line_spacing: f32) -> Div {
    spaced_element(text, line_spacing)
}

/// spaced_lines for a text element other than a plain string
pub(crate) fn scaled_text_floor(mut base: Div) -> Div {
    if base.style().min_size.width.is_none() { base.min_w_0() } else { base }
}

pub(crate) fn spaced_element(element: impl IntoElement, line_spacing: f32) -> Div {
    let block = div().child(element);
    if line_spacing > 0.0 {
        let half = px(-line_spacing / 2.0);
        block.mt(half).mb(half)
    } else {
        block
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MarginEdges {
    pub top: f32,
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
}

impl MarginEdges {
    pub fn is_zero(&self) -> bool {
        self.top == 0.0 && self.left == 0.0 && self.right == 0.0 && self.bottom == 0.0
    }

    pub fn apply(&self, element: Div) -> Div {
        element
            .pt(px(self.top))
            .pl(px(self.left))
            .pr(px(self.right))
            .pb(px(self.bottom))
    }
}

/// leading and trailing margins swap sides under right to left
pub(crate) fn margin_edges(
    top: f32,
    leading: f32,
    trailing: f32,
    bottom: f32,
    direction: LayoutDirection,
) -> MarginEdges {
    let (left, right) = if direction.is_right_to_left() {
        (trailing, leading)
    } else {
        (leading, trailing)
    };
    MarginEdges {
        top,
        left,
        right,
        bottom,
    }
}
