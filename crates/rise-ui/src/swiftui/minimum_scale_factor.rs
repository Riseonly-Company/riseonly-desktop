use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    Hsla, IntoElement, LayoutId, Pixels, SharedString, Size, Style, TextRun, TruncateFrom, Window,
    WrappedLine, px,
};

use crate::swiftui::text::TruncationMode;

const TRUNCATION_AFFIX: &str = "\u{2026}";

const WRAPPED_SCALE_SEARCH_STEPS: usize = 8;

const SINGLE_LINE_SCALE_ATTEMPTS: usize = 3;

const SINGLE_LINE_SCALE_STEP: f32 = 0.98;

const MINIMUM_SCALE_FLOOR: f32 = 0.01;

/// clamps the factor to above zero and at most one, no growing
pub fn clamped_minimum_scale_factor(factor: f32) -> f32 {
    if factor.is_nan() {
        return 1.0;
    }
    factor.clamp(MINIMUM_SCALE_FLOOR, 1.0)
}

/// scale that fits one line of natural width into available width
/// never below minimum and never above one
pub fn single_line_scale(natural_width: f32, available_width: f32, minimum: f32) -> f32 {
    let minimum = clamped_minimum_scale_factor(minimum);
    if natural_width <= available_width || natural_width <= 0.0 {
        return 1.0;
    }
    (available_width.max(0.0) / natural_width).clamp(minimum, 1.0)
}

/// font scales so the glyph part of the line shrinks, line spacing is a fixed gap and stays
pub fn scaled_line_height(full_line_height: f32, line_spacing: f32, scale: f32) -> f32 {
    let spacing = line_spacing.max(0.0);
    let glyphs = (full_line_height - spacing).max(0.0);
    glyphs * scale + spacing
}

/// where gpui cuts a line for each truncation mode
pub fn truncate_from(mode: TruncationMode) -> TruncateFrom {
    match mode {
        TruncationMode::Head => TruncateFrom::Start,
        TruncationMode::Tail => TruncateFrom::End,
        TruncationMode::Middle => TruncateFrom::Middle,
    }
}

/// rows a shaped text takes, each hard line plus its soft wraps
pub fn visual_line_count(lines: &[WrappedLine]) -> usize {
    lines
        .iter()
        .map(|line| line.wrap_boundaries().len() + 1)
        .sum()
}

/// letters of scripts gpui keeps whole when wrapping, between two of them a soft wrap cuts a word
pub fn is_word_letter(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '\u{00C0}'..='\u{024F}' | '\u{0300}'..='\u{036F}' | '\u{0400}'..='\u{04FF}' | '\u{1E00}'..='\u{1EFF}')
}

pub fn wraps_inside_word(lines: &[WrappedLine]) -> bool {
    lines.iter().any(|line| {
        let text = line.text.as_str();
        line.wrap_boundaries().iter().any(|boundary| {
            let Some(glyph) = line
                .unwrapped_layout
                .runs
                .get(boundary.run_ix)
                .and_then(|run| run.glyphs.get(boundary.glyph_ix))
            else {
                return false;
            };
            let Some((before, after)) = text.get(..glyph.index).zip(text.get(glyph.index..)) else {
                return false;
            };
            match (before.chars().next_back(), after.chars().next()) {
                (Some(previous), Some(next)) => is_word_letter(previous) && is_word_letter(next),
                _ => false,
            }
        })
    })
}

/// text that shrinks to fit before it truncates
/// without a line limit it wraps instead of shrinking
pub struct MinimumScaleText {
    text: SharedString,
    minimum_scale_factor: f32,
    line_limit: Option<usize>,
    truncation: TruncationMode,
    line_spacing: f32,
    color_runs: Vec<(Range<usize>, Hsla)>,
}

/// splits the inherited run at the colored byte ranges so a swept label keeps colors at every scale
/// ranges must be sorted, disjoint and on char boundaries
pub fn colored_runs(base: TextRun, len: usize, color_runs: &[(Range<usize>, Hsla)]) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(color_runs.len() * 2 + 1);
    let mut cursor = 0;
    let push = |runs: &mut Vec<TextRun>, run_len: usize, color: Option<Hsla>| {
        if run_len == 0 {
            return;
        }
        let mut run = base.clone();
        run.len = run_len;
        if let Some(color) = color {
            run.color = color;
        }
        runs.push(run);
    };
    for (range, color) in color_runs {
        let start = range.start.clamp(cursor, len);
        let end = range.end.clamp(start, len);
        push(&mut runs, start - cursor, None);
        push(&mut runs, end - start, Some(*color));
        cursor = end;
    }
    push(&mut runs, len - cursor, None);
    runs
}

impl MinimumScaleText {
    pub fn new(text: impl Into<SharedString>, minimum_scale_factor: f32) -> Self {
        Self {
            text: text.into(),
            minimum_scale_factor: clamped_minimum_scale_factor(minimum_scale_factor),
            line_limit: None,
            truncation: TruncationMode::Tail,
            line_spacing: 0.0,
            color_runs: Vec::new(),
        }
    }

    /// per range colors over the inherited text color
    pub fn color_runs(mut self, color_runs: Vec<(Range<usize>, Hsla)>) -> Self {
        self.color_runs = color_runs;
        self
    }

    /// line limit, 0 means no limit
    pub fn line_limit(mut self, line_limit: impl Into<Option<usize>>) -> Self {
        self.line_limit = line_limit.into().filter(|lines| *lines > 0);
        self
    }

    /// truncation mode for what is left at the smallest scale
    pub fn truncation(mut self, truncation: TruncationMode) -> Self {
        self.truncation = truncation;
        self
    }

    /// part of the inherited line height that is line spacing, kept unscaled
    pub fn line_spacing(mut self, line_spacing: f32) -> Self {
        self.line_spacing = line_spacing.max(0.0);
        self
    }
}

impl IntoElement for MinimumScaleText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// what the measure pass settled on, read back when painting
pub struct MinimumScaleTextLayout {
    lines: Vec<WrappedLine>,
    line_height: Pixels,
}

struct FitInput {
    text: SharedString,
    runs: Vec<TextRun>,
    font_size: Pixels,
    minimum_scale_factor: f32,
    line_limit: Option<usize>,
    truncation: TruncationMode,
}

struct Fit {
    lines: Vec<WrappedLine>,
    scale: f32,
}

impl FitInput {
    fn wrap_width(&self, width: Pixels) -> Option<Pixels> {
        match self.line_limit {
            Some(1) => None,
            _ => Some(width),
        }
    }

    fn shape(
        &self,
        text: SharedString,
        runs: &[TextRun],
        scale: f32,
        wrap_width: Option<Pixels>,
        window: &mut Window,
    ) -> Vec<WrappedLine> {
        window
            .text_system()
            .shape_text(text, self.font_size * scale, runs, wrap_width, self.line_limit)
            .map(|lines| lines.into_iter().collect())
            .unwrap_or_default()
    }

    fn fits(&self, lines: &[WrappedLine], width: Pixels) -> bool {
        match self.line_limit {
            Some(1) => lines
                .iter()
                .all(|line| line.unwrapped_layout.width <= width),
            Some(limit) => visual_line_count(lines) <= limit && !wraps_inside_word(lines),
            None => true,
        }
    }

    fn unclamped_fits(&self, scale: f32, width: Pixels, window: &mut Window) -> bool {
        let wrap = self.wrap_width(width);
        let unclamped = window
            .text_system()
            .shape_text(self.text.clone(), self.font_size * scale, &self.runs, wrap, None)
            .map(|lines| lines.into_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        self.fits(&unclamped, width)
    }

    fn fit(&self, width: Option<Pixels>, window: &mut Window, cx: &mut App) -> Fit {
        let Some(width) = width else {
            let lines = self.shape(self.text.clone(), &self.runs, 1.0, None, window);
            return Fit { lines, scale: 1.0 };
        };
        let wrap = self.wrap_width(width);

        let fits_full = self.unclamped_fits(1.0, width, window);
        if fits_full || self.line_limit.is_none() {
            let lines = self.shape(self.text.clone(), &self.runs, 1.0, wrap, window);
            return Fit { lines, scale: 1.0 };
        }

        let scale = match self.line_limit {
            Some(1) => self.single_line_fit_scale(width, window),
            _ => self.wrapped_fit_scale(width, window),
        };

        let fits = self.unclamped_fits(scale, width, window);
        if fits {
            let lines = self.shape(self.text.clone(), &self.runs, scale, wrap, window);
            return Fit { lines, scale };
        }

        let lines = self.truncated(scale, width, window, cx);
        Fit { lines, scale }
    }

    fn single_line_fit_scale(&self, width: Pixels, window: &mut Window) -> f32 {
        let natural = self
            .shape(self.text.clone(), &self.runs, 1.0, None, window)
            .iter()
            .map(|line| f32::from(line.unwrapped_layout.width))
            .fold(0.0_f32, f32::max);
        let mut scale = single_line_scale(natural, f32::from(width), self.minimum_scale_factor);
        for _ in 0..SINGLE_LINE_SCALE_ATTEMPTS {
            let fits = self.unclamped_fits(scale, width, window);
            if fits || scale <= self.minimum_scale_factor {
                break;
            }
            scale = (scale * SINGLE_LINE_SCALE_STEP).max(self.minimum_scale_factor);
        }
        scale
    }

    fn wrapped_fit_scale(&self, width: Pixels, window: &mut Window) -> f32 {
        let mut low = self.minimum_scale_factor;
        let mut high = 1.0_f32;
        let fits_low = self.unclamped_fits(low, width, window);
        if !fits_low {
            return low;
        }
        for _ in 0..WRAPPED_SCALE_SEARCH_STEPS {
            let middle = (low + high) / 2.0;
            let fits = self.unclamped_fits(middle, width, window);
            if fits {
                low = middle;
            } else {
                high = middle;
            }
        }
        low
    }

    fn truncated(&self, scale: f32, width: Pixels, window: &mut Window, cx: &mut App) -> Vec<WrappedLine> {
        let Some(font) = self.runs.first().map(|run| run.font.clone()) else {
            return Vec::new();
        };
        let mut line_wrapper = cx.text_system().line_wrapper(font, self.font_size * scale);
        let from = truncate_from(self.truncation);
        let (text, runs) = match self.line_limit {
            Some(limit) if limit > 1 => line_wrapper.truncate_wrapped_line(
                self.text.clone(),
                width,
                limit,
                TRUNCATION_AFFIX,
                &self.runs,
                from,
            ),
            _ => line_wrapper.truncate_line(self.text.clone(), width, TRUNCATION_AFFIX, &self.runs, from),
        };
        let runs = runs.into_owned();
        self.shape(text, &runs, scale, self.wrap_width(width), window)
    }
}

impl Element for MinimumScaleText {
    type RequestLayoutState = Rc<RefCell<Option<MinimumScaleTextLayout>>>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text_style = window.text_style();
        let rem_size = window.rem_size();
        let font_size = text_style.font_size.to_pixels(rem_size);
        let full_line_height = f32::from(text_style.line_height.to_pixels(font_size.into(), rem_size));
        let input = FitInput {
            runs: colored_runs(text_style.to_run(self.text.len()), self.text.len(), &self.color_runs),
            text: self.text.clone(),
            font_size,
            minimum_scale_factor: self.minimum_scale_factor,
            line_limit: self.line_limit,
            truncation: self.truncation,
        };
        let line_spacing = self.line_spacing;
        let state: Self::RequestLayoutState = Rc::new(RefCell::new(None));
        let measured = state.clone();
        let layout_id = window.request_measured_layout(Style::default(), move |known, available, window, cx| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
            });
            let fit = input.fit(width, window, cx);
            let line_height = window.pixel_snap(px(scaled_line_height(full_line_height, line_spacing, fit.scale)));
            let mut size: Size<Pixels> = Size::default();
            for line in &fit.lines {
                let line_size = line.size(line_height);
                size.height += line_size.height;
                size.width = size.width.max(line_size.width).ceil();
            }
            if let Some(width) = width {
                size.width = size.width.min(width);
            }
            *measured.borrow_mut() = Some(MinimumScaleTextLayout {
                lines: fit.lines,
                line_height,
            });
            size
        });
        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = layout.borrow();
        let Some(layout) = layout.as_ref() else {
            return;
        };
        let align = window.text_style().text_align;
        let mut origin = bounds.origin;
        for line in &layout.lines {
            let _ = line.paint_background(origin, layout.line_height, align, Some(bounds), window, cx);
            let _ = line.paint(origin, layout.line_height, align, Some(bounds), window, cx);
            origin.y += line.size(layout.line_height).height;
        }
    }
}
