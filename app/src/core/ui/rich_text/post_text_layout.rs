use std::any::Any;
use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Bounds, Font as GpuiFont, FontId, Hsla, Pixels, Point, RenderGlyphParams, RenderImage, Rgba, SUBPIXEL_VARIANTS_X,
    SUBPIXEL_VARIANTS_Y, ShapedLine, SharedString, Size, TextAlign, TextRun, TextSystem, Window, WindowTextSystem,
    bounds, fill, point, px, size,
};
use image::{Frame, RgbaImage};
use rise_core::foundation::foundation_text::character_prefix;
use rise_core::foundation::ns_range::{self, NSRange};
use rise_ui::swiftui::font::Font;
use rise_ui::swiftui::rich_text::{AttributeContainer, RichFont};

use crate::core::ui::rich_text::post_rich_text_render::{
    PostRichTextAttribute, PostRichTextAttributedString, PostRichTextCalibration, PostRichTextStyleRun,
};
use crate::core::ui::rich_text::post_text_contracts_hooks::{PostTextLayouting, PostTextToken};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;

/// one uniformly sized stretch of a line, gpui shapes one font size per line
/// so a line with inline code or kerned separators is a row of these
#[derive(Clone, Debug)]
pub struct PostTextLinePiece {
    shaped: ShapedLine,
    colors: Vec<(usize, Hsla)>,
    x: f32,
    byte_start: usize,
    utf16_start: usize,
    utf16_length: usize,
    kern: f32,
    is_truncation: bool,
}

impl PostTextLinePiece {
    fn kern_before(&self, byte: usize) -> f32 {
        let text: &str = self.shaped.text.as_ref();
        let byte = byte.min(text.len());
        if self.kern == 0.0 || !text.is_char_boundary(byte) {
            return 0.0;
        }
        self.kern * ns_range::length_utf16(&text[..byte]) as f32
    }

    fn color_at(&self, byte: usize) -> Hsla {
        let mut end = 0;
        for (length, color) in self.colors.iter() {
            end += length;
            if byte < end {
                return *color;
            }
        }
        self.colors.last().map_or_else(gpui::black, |(_, color)| *color)
    }
}

/// what a line was shaped from and its typographic bounds
#[derive(Clone, Debug, Default)]
pub struct PostTextCtLine {
    pieces: Vec<PostTextLinePiece>,
    width: f32,
    trailing_whitespace_width: f32,
    ascent: f32,
    descent: f32,
    leading: f32,
}

impl PostTextCtLine {
    /// offset for a string index, index is in utf-16 units
    fn offset_for_string_index(&self, index: usize) -> f32 {
        for piece in self.pieces.iter() {
            if index <= piece.utf16_start {
                return piece.x;
            }
            if index < piece.utf16_start + piece.utf16_length {
                let text: &str = piece.shaped.text.as_ref();
                let mut units = index - piece.utf16_start;
                let byte = loop {
                    match ns_range::utf16_to_byte_offset(text, units) {
                        Some(byte) => break byte,
                        None => units -= 1,
                    }
                };
                return piece.x + f32::from(piece.shaped.x_for_index(byte)) + piece.kern_before(byte);
            }
        }
        self.width
    }
}

/// one laid out line, origin top left and y goes down
#[derive(Clone, Debug)]
pub struct PostTextLayoutLine {
    pub line: PostTextCtLine,
    pub visible_range: NSRange,
    pub line_index_offset: usize,
    pub baseline_origin: Point<Pixels>,
    pub top: f32,
    pub height: f32,
    pub used_width: f32,
    pub is_truncated: bool,
}

/// background of a run, like inline code
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostTextLayoutFill {
    pub rect: Bounds<Pixels>,
    pub color: Hsla,
}

/// strikethrough bar
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostTextLayoutStroke {
    pub rect: Bounds<Pixels>,
    pub color: Hsla,
}

/// finished layout of one post text block, made off the main thread
pub struct PostTextLayout {
    pub size: Size<Pixels>,
    pub plain_text: String,
    pub tokens: Vec<PostTextToken>,
    pub lines: Vec<PostTextLayoutLine>,
    pub backgrounds: Vec<PostTextLayoutFill>,
    pub strikethroughs: Vec<PostTextLayoutStroke>,
    pub is_truncated: bool,
    pub display_scale: f32,
}

impl PostTextLayout {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        size: Size<Pixels>,
        plain_text: String,
        tokens: Vec<PostTextToken>,
        lines: Vec<PostTextLayoutLine>,
        backgrounds: Vec<PostTextLayoutFill>,
        strikethroughs: Vec<PostTextLayoutStroke>,
        is_truncated: bool,
        display_scale: f32,
    ) -> Self {
        Self {
            size,
            plain_text,
            tokens,
            lines,
            backgrounds,
            strikethroughs,
            is_truncated,
            display_scale,
        }
    }

    /// draws the layout into a transparent bitmap off the main thread
    /// none if over the pixel budget, empty or cancelled
    pub fn make_bitmap(
        &self,
        scale: Option<f32>,
        pixel_budget: i64,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<PostTextBitmap> {
        let resolved_scale = scale.filter(|value| value.is_finite() && *value > 0.0).unwrap_or(self.display_scale);
        let width = f32::from(self.size.width);
        let height = f32::from(self.size.height);
        if self.lines.is_empty() || width <= 0.0 || height <= 0.0 {
            return None;
        }
        let maximum_dimension = PostRichTextCalibration::BITMAP_MAXIMUM_DIMENSION as f32;
        let scaled_width = (width * resolved_scale).ceil();
        let scaled_height = (height * resolved_scale).ceil();
        if !scaled_width.is_finite()
            || !scaled_height.is_finite()
            || scaled_width > maximum_dimension
            || scaled_height > maximum_dimension
        {
            return None;
        }
        let pixel_width = scaled_width as i64;
        let pixel_height = scaled_height as i64;
        let pixel_count = pixel_width.checked_mul(pixel_height)?;
        if pixel_width <= 0 || pixel_height <= 0 || pixel_count > pixel_budget || is_cancelled() {
            return None;
        }

        let text_system = PostTextLayoutBuilder::text_system()?;
        let mut canvas = PostTextBitmapCanvas::new(pixel_width as u32, pixel_height as u32);
        for background in self.backgrounds.iter() {
            canvas.fill(background.rect, background.color, resolved_scale);
        }
        for line in self.lines.iter() {
            if is_cancelled() {
                return None;
            }
            Self::rasterize_line(&mut canvas, &text_system, line, resolved_scale);
        }
        for stroke in self.strikethroughs.iter() {
            canvas.fill(stroke.rect, stroke.color, resolved_scale);
        }
        if is_cancelled() {
            return None;
        }
        let image = RgbaImage::from_raw(pixel_width as u32, pixel_height as u32, canvas.pixels)?;
        Some(PostTextBitmap {
            image: Arc::new(RenderImage::new([Frame::new(image)])),
            size: self.size,
            scale: resolved_scale,
        })
    }

    fn rasterize_line(
        canvas: &mut PostTextBitmapCanvas,
        text_system: &Arc<TextSystem>,
        line: &PostTextLayoutLine,
        scale: f32,
    ) {
        let baseline_x = f32::from(line.baseline_origin.x);
        let baseline_y = f32::from(line.baseline_origin.y);
        for piece in line.line.pieces.iter() {
            for run in piece.shaped.runs.iter() {
                for glyph in run.glyphs.iter() {
                    let x = baseline_x + piece.x + f32::from(glyph.position.x) + piece.kern_before(glyph.index);
                    let (origin_x, variant_x) = Self::quantized(x * scale, SUBPIXEL_VARIANTS_X);
                    let (origin_y, variant_y) = Self::quantized(baseline_y * scale, SUBPIXEL_VARIANTS_Y);
                    let params = RenderGlyphParams {
                        font_id: run.font_id,
                        glyph_id: glyph.id,
                        font_size: piece.shaped.font_size,
                        subpixel_variant: point(variant_x, variant_y),
                        scale_factor: scale,
                        is_emoji: glyph.is_emoji,
                        subpixel_rendering: false,
                        dilation: 0,
                    };
                    let Ok(raster_bounds) = text_system.raster_bounds(&params) else {
                        continue;
                    };
                    if raster_bounds.size.width.0 <= 0 || raster_bounds.size.height.0 <= 0 {
                        continue;
                    }
                    let Ok((raster_size, bytes)) = text_system.rasterize_glyph(&params) else {
                        continue;
                    };
                    let left = origin_x + raster_bounds.origin.x.0;
                    let top = origin_y + raster_bounds.origin.y.0;
                    let raster_width = raster_size.width.0;
                    let raster_height = raster_size.height.0;
                    if glyph.is_emoji {
                        canvas.blit_bgra(left, top, raster_width, raster_height, &bytes);
                    } else {
                        canvas.blit_coverage(left, top, raster_width, raster_height, &bytes, piece.color_at(glyph.index));
                    }
                }
            }
        }
    }

    /// whole device pixel and subpixel variant gpui picks for a glyph origin
    fn quantized(value: f32, variants: u8) -> (i32, u8) {
        let count = f32::from(variants);
        let snapped = (value * count).round() / count;
        (snapped.trunc() as i32, (snapped.fract().abs() * count) as u8)
    }
}

impl PostTextLayouting for PostTextLayout {
    fn size(&self) -> Size<Pixels> {
        self.size
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn plain_text(&self) -> &str {
        &self.plain_text
    }

    fn tokens(&self) -> &[PostTextToken] {
        &self.tokens
    }

    fn draw(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
        if self.lines.is_empty() {
            return;
        }
        let offset = |rect: Bounds<Pixels>| bounds(rect.origin + origin, rect.size);
        for background in self.backgrounds.iter() {
            window.paint_quad(fill(offset(background.rect), background.color));
        }
        for line in self.lines.iter() {
            for piece in line.line.pieces.iter() {
                let shaped = &piece.shaped;
                let line_height = shaped.ascent + shaped.descent;
                let piece_origin = point(
                    origin.x + line.baseline_origin.x + px(piece.x),
                    origin.y + line.baseline_origin.y - shaped.ascent,
                );
                let _ = shaped.paint(piece_origin, line_height, TextAlign::Left, None, window, cx);
            }
        }
        for stroke in self.strikethroughs.iter() {
            window.paint_quad(fill(offset(stroke.rect), stroke.color));
        }
    }

    fn as_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// pre-rendered layout, image is drawn at size with scale device pixels per point
#[derive(Clone)]
pub struct PostTextBitmap {
    pub image: Arc<RenderImage>,
    pub size: Size<Pixels>,
    pub scale: f32,
}

/// bgra pixels with straight alpha, same byte order RenderImage draws
struct PostTextBitmapCanvas {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
}

impl PostTextBitmapCanvas {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width: width as i32,
            height: height as i32,
            pixels: vec![0; width as usize * height as usize * 4],
        }
    }

    fn fill(&mut self, rect: Bounds<Pixels>, color: Hsla, scale: f32) {
        let left = (f32::from(rect.origin.x) * scale).round() as i32;
        let top = (f32::from(rect.origin.y) * scale).round() as i32;
        let right = ((f32::from(rect.origin.x) + f32::from(rect.size.width)) * scale).round() as i32;
        let bottom = ((f32::from(rect.origin.y) + f32::from(rect.size.height)) * scale).round() as i32;
        let rgba = Rgba::from(color);
        for y in top.max(0)..bottom.min(self.height) {
            for x in left.max(0)..right.min(self.width) {
                self.source_over(x, y, rgba.b, rgba.g, rgba.r, rgba.a);
            }
        }
    }

    fn blit_coverage(&mut self, left: i32, top: i32, width: i32, height: i32, bytes: &[u8], color: Hsla) {
        let rgba = Rgba::from(color);
        for row in 0..height {
            for column in 0..width {
                let Some(coverage) = bytes.get((row * width + column) as usize) else {
                    return;
                };
                let alpha = rgba.a * f32::from(*coverage) / 255.0;
                self.source_over(left + column, top + row, rgba.b, rgba.g, rgba.r, alpha);
            }
        }
    }

    fn blit_bgra(&mut self, left: i32, top: i32, width: i32, height: i32, bytes: &[u8]) {
        for row in 0..height {
            for column in 0..width {
                let start = ((row * width + column) * 4) as usize;
                let Some(pixel) = bytes.get(start..start + 4) else {
                    return;
                };
                let channel = |index: usize| f32::from(pixel[index]) / 255.0;
                self.source_over(left + column, top + row, channel(0), channel(1), channel(2), channel(3));
            }
        }
    }

    fn source_over(&mut self, x: i32, y: i32, blue: f32, green: f32, red: f32, alpha: f32) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height || alpha <= 0.0 {
            return;
        }
        let start = (y as usize * self.width as usize + x as usize) * 4;
        let Some(pixel) = self.pixels.get_mut(start..start + 4) else {
            return;
        };
        let under_alpha = f32::from(pixel[3]) / 255.0;
        let out_alpha = alpha + under_alpha * (1.0 - alpha);
        if out_alpha <= 0.0 {
            return;
        }
        for (index, channel) in [blue, green, red].into_iter().enumerate() {
            let under = f32::from(pixel[index]) / 255.0;
            let out = (channel * alpha + under * under_alpha * (1.0 - alpha)) / out_alpha;
            pixel[index] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        pixel[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

struct PostTextTypesetter<'a> {
    shaper: &'a WindowTextSystem,
    text_system: &'a Arc<TextSystem>,
    attributed: &'a PostRichTextAttributedString,
    paragraph_end: usize,
    paragraph_width: f32,
    glyph_offsets: Vec<(usize, f32)>,
}

impl<'a> PostTextTypesetter<'a> {
    fn new(
        shaper: &'a WindowTextSystem,
        text_system: &'a Arc<TextSystem>,
        attributed: &'a PostRichTextAttributedString,
    ) -> Self {
        Self {
            shaper,
            text_system,
            attributed,
            paragraph_end: 0,
            paragraph_width: 0.0,
            glyph_offsets: Vec::new(),
        }
    }

    fn prepare_paragraph(&mut self, start: usize) {
        if start < self.paragraph_end {
            return;
        }
        let text = self.attributed.string();
        let end = PostTextLayoutBuilder::paragraph_end(text, start);
        let line = PostTextLayoutBuilder::create_line(self.shaper, self.text_system, self.attributed, start..end);
        let mut offsets = Vec::new();
        for piece in line.pieces.iter() {
            for run in piece.shaped.runs.iter() {
                for glyph in run.glyphs.iter() {
                    offsets.push((
                        piece.byte_start + glyph.index,
                        piece.x + f32::from(glyph.position.x) + piece.kern_before(glyph.index),
                    ));
                }
            }
        }
        offsets.sort_by_key(|(byte, _)| *byte);
        self.paragraph_end = end;
        self.paragraph_width = line.width;
        self.glyph_offsets = offsets;
    }

    /// x of the first glyph at or after this byte
    fn offset(&self, byte: usize) -> f32 {
        let index = self.glyph_offsets.partition_point(|(glyph_byte, _)| *glyph_byte < byte);
        self.glyph_offsets.get(index).map_or(self.paragraph_width, |(_, x)| *x)
    }

    fn suggest_line_break(&mut self, start: usize, width: f32) -> usize {
        self.prepare_paragraph(start);
        let text = self.attributed.string();
        let Some(paragraph) = text.get(start..self.paragraph_end) else {
            return 0;
        };
        let origin = self.offset(start);
        let mut last_break = 0;
        let mut characters = paragraph.char_indices().peekable();
        while let Some((index, character)) = characters.next() {
            let end = start + index + character.len_utf8();
            if !character.is_whitespace() && self.offset(end) - origin > width {
                return last_break;
            }
            let next = characters.peek().map(|(_, next)| *next);
            if next.is_some_and(|next| PostTextLayoutBuilder::is_break_opportunity(character, next)) {
                last_break = end - start;
            }
        }
        paragraph.len()
    }

    fn suggest_cluster_break(&mut self, start: usize, width: f32) -> usize {
        self.prepare_paragraph(start);
        let text = self.attributed.string();
        let Some(paragraph) = text.get(start..self.paragraph_end) else {
            return 0;
        };
        let origin = self.offset(start);
        let mut count = 0;
        while count < paragraph.len() {
            let cluster = character_prefix(&paragraph[count..], 1).len();
            if self.offset(start + count + cluster) - origin > width {
                break;
            }
            count += cluster;
        }
        count
    }
}

struct PostTextLayoutBuilderPlacedLine {
    line: PostTextCtLine,
    visible_range: NSRange,
    line_index_offset: usize,
    is_truncated: bool,
}

struct PostTextLayoutBuilderSegment<'a> {
    line: &'a PostTextLayoutLine,
    min_x: f32,
    max_x: f32,
}

/// builds post text layouts, any thread is fine
pub enum PostTextLayoutBuilder {}

impl PostTextLayoutBuilder {
    const UNBOUNDED_WIDTH: f32 = 1_000_000.0;
    const ELLIPSIS: &'static str = "\u{2026}";

    /// pass spec.display_scale, a bad value falls back to the fallback display scale
    pub fn layout(
        attributed: &PostRichTextAttributedString,
        width: f32,
        maximum_lines: Option<i64>,
        display_scale: f32,
    ) -> PostTextLayout {
        let scale = if display_scale.is_finite() && display_scale > 0.0 {
            display_scale
        } else {
            PostRichTextCalibration::FALLBACK_DISPLAY_SCALE
        };
        let text = attributed.string();
        let length = attributed.length();
        let empty = || {
            PostTextLayout::new(
                size(px(0.0), px(0.0)),
                text.to_owned(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                false,
                scale,
            )
        };
        if length == 0 {
            return empty();
        }
        let Some(text_system) = Self::text_system() else {
            return empty();
        };

        let line_limit = maximum_lines.filter(|limit| *limit > 0);
        let break_width = if width.is_finite() { width.max(1.0) } else { Self::UNBOUNDED_WIDTH };
        let shaper = WindowTextSystem::new(text_system.clone());
        let mut typesetter = PostTextTypesetter::new(&shaper, &text_system, attributed);

        let mut lines: Vec<PostTextLayoutLine> = Vec::new();
        let mut start = 0_usize;
        let mut next_top = 0.0_f32;
        let mut used_width = 0.0_f32;
        let mut is_truncated = false;

        while start < text.len() {
            let mut count = typesetter.suggest_line_break(start, break_width);
            if count == 0 {
                count = typesetter.suggest_cluster_break(start, break_width);
            }
            if count == 0 {
                count = character_prefix(&text[start..], 1).len();
            }
            count = Self::fitted_count(&shaper, &text_system, attributed, start, count, break_width);

            let is_last_allowed_line = line_limit.is_some_and(|limit| lines.len() as i64 == limit - 1);
            let cuts_text = is_last_allowed_line && start + count < text.len();
            let visible_bytes = start..start + count;
            let placed = if cuts_text {
                Self::truncated_line(&shaper, &text_system, attributed, visible_bytes.clone(), break_width)
            } else {
                None
            };
            let placed = placed.unwrap_or_else(|| PostTextLayoutBuilderPlacedLine {
                line: Self::create_line(&shaper, &text_system, attributed, visible_bytes.clone()),
                visible_range: ns_range::byte_to_utf16_range(text, visible_bytes.clone()),
                line_index_offset: 0,
                is_truncated: false,
            });

            if let Some(previous) = lines.last() {
                next_top += Self::spacing(previous.visible_range, attributed);
            }

            let line_width = (placed.line.width - placed.line.trailing_whitespace_width).max(0.0).min(break_width);
            let line_height = (placed.line.ascent
                + placed.line.descent
                + placed.line.leading.max(0.0)
                + PostRichTextCalibration::EXTRA_LINE_HEIGHT)
                .max(0.0);
            let top = next_top;
            let mut baseline = top + placed.line.ascent;
            if PostRichTextCalibration::SNAPS_BASELINES_TO_PIXELS {
                baseline = (baseline * scale).round() / scale;
            }

            lines.push(PostTextLayoutLine {
                line: placed.line,
                visible_range: placed.visible_range,
                line_index_offset: placed.line_index_offset,
                baseline_origin: point(px(0.0), px(baseline)),
                top,
                height: line_height,
                used_width: line_width,
                is_truncated: placed.is_truncated,
            });
            next_top = top + line_height;
            used_width = used_width.max(line_width);

            if cuts_text {
                is_truncated = true;
                break;
            }
            start += count;
        }

        if PostRichTextCalibration::APPLIES_LINE_SPACING_AFTER_LAST_LINE
            && let Some(last) = lines.last()
        {
            next_top += Self::spacing(last.visible_range, attributed);
        }

        let tokens = Self::tokens(attributed, &lines);
        let backgrounds = Self::backgrounds(attributed, &lines);
        let strikethroughs = Self::strikethroughs(attributed, &lines, scale, &text_system);
        PostTextLayout::new(
            size(px(Self::pixel_ceil(used_width, scale)), px(Self::pixel_ceil(next_top, scale))),
            text.to_owned(),
            tokens,
            lines,
            backgrounds,
            strikethroughs,
            is_truncated,
            scale,
        )
    }

    /// text system bound at launch, the bake already owns it for off frame measuring
    pub fn text_system() -> Option<Arc<TextSystem>> {
        EditorImageBake::text_system()
    }

    pub(crate) fn text_width(text: &str, font: &RichFont) -> f32 {
        let Some(text_system) = Self::text_system() else {
            return 0.0;
        };
        let shaper = WindowTextSystem::new(text_system);
        let run = TextRun {
            len: text.len(),
            font: Self::gpui_font(font),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(shaper.shape_line(SharedString::from(text.to_owned()), px(font.size), &[run], None).width())
    }

    fn gpui_font(font: &RichFont) -> GpuiFont {
        let face = Font {
            size: font.size,
            weight: font.weight,
            design: font.design,
            italic: font.italic,
            monospaced_digit: false,
        };
        GpuiFont {
            family: face.family(),
            features: face.features(),
            fallbacks: None,
            weight: face.weight,
            style: face.style(),
        }
    }

    fn font_id(text_system: &Arc<TextSystem>, font: &RichFont) -> FontId {
        text_system.resolve_font(&Self::gpui_font(font))
    }

    /// makes a line from the bytes of source, one piece per font size and kern
    /// line breaks take no room and arent shaped
    fn create_line(
        shaper: &WindowTextSystem,
        text_system: &Arc<TextSystem>,
        source: &PostRichTextAttributedString,
        bytes: Range<usize>,
    ) -> PostTextCtLine {
        let text = source.string();
        let Some(whole) = text.get(bytes.clone()) else {
            return PostTextCtLine::default();
        };
        let shaped_end = bytes.start + whole.trim_end_matches(Self::is_line_break).len();
        let runs = source.style_runs(bytes.clone());

        let mut line = PostTextCtLine::default();
        for run in runs.iter() {
            let font_id = Self::font_id(text_system, &run.font);
            line.ascent = line.ascent.max(f32::from(text_system.ascent(font_id, px(run.font.size))));
            line.descent = line.descent.max(f32::from(text_system.descent(font_id, px(run.font.size))).abs());
        }

        let mut group: Vec<PostRichTextStyleRun> = Vec::new();
        for run in runs {
            let end = run.bytes.end.min(shaped_end);
            if run.bytes.start >= end {
                continue;
            }
            let run = PostRichTextStyleRun {
                bytes: run.bytes.start..end,
                ..run
            };
            let joins = group.last().is_some_and(|last| {
                last.font.size.to_bits() == run.font.size.to_bits()
                    && last.kern.to_bits() == run.kern.to_bits()
                    && last.is_truncation == run.is_truncation
            });
            if !joins {
                Self::flush_piece(&mut line, shaper, text, &mut group);
            }
            group.push(run);
        }
        Self::flush_piece(&mut line, shaper, text, &mut group);

        line.trailing_whitespace_width = Self::trailing_whitespace_width(&line);
        line
    }

    fn flush_piece(
        line: &mut PostTextCtLine,
        shaper: &WindowTextSystem,
        text: &str,
        group: &mut Vec<PostRichTextStyleRun>,
    ) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            return;
        };
        let bytes = first.bytes.start..last.bytes.end;
        let Some(piece_text) = text.get(bytes.clone()) else {
            group.clear();
            return;
        };
        let runs: Vec<TextRun> = group
            .iter()
            .map(|run| TextRun {
                len: run.bytes.len(),
                font: Self::gpui_font(&run.font),
                color: run.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            })
            .collect();
        let shaped = shaper.shape_line(SharedString::from(piece_text.to_owned()), px(first.font.size), &runs, None);
        let utf16_length = ns_range::length_utf16(piece_text);
        let x = line.width;
        let kern = first.kern;
        line.width += f32::from(shaped.width) + kern * utf16_length as f32;
        line.ascent = line.ascent.max(f32::from(shaped.ascent));
        line.descent = line.descent.max(f32::from(shaped.descent).abs());
        line.pieces.push(PostTextLinePiece {
            colors: group.iter().map(|run| (run.bytes.len(), run.color)).collect(),
            x,
            byte_start: bytes.start,
            utf16_start: ns_range::byte_to_utf16_offset(text, bytes.start),
            utf16_length,
            kern,
            is_truncation: first.is_truncation,
            shaped,
        });
        group.clear();
    }

    fn trailing_whitespace_width(line: &PostTextCtLine) -> f32 {
        for piece in line.pieces.iter().rev() {
            let text: &str = piece.shaped.text.as_ref();
            let kept = text.trim_end().len();
            if kept > 0 {
                let end = piece.x + f32::from(piece.shaped.x_for_index(kept)) + piece.kern_before(kept);
                return (line.width - end).max(0.0);
            }
        }
        line.width
    }

    /// end of the paragraph that holds start, terminator included
    fn paragraph_end(text: &str, start: usize) -> usize {
        let Some(rest) = text.get(start..) else {
            return text.len();
        };
        for (index, character) in rest.char_indices() {
            if Self::is_line_break(character) {
                let end = start + index + character.len_utf8();
                if character == '\r' && text[end..].starts_with('\n') {
                    return end + 1;
                }
                return end;
            }
        }
        text.len()
    }

    fn fitted_count(
        shaper: &WindowTextSystem,
        text_system: &Arc<TextSystem>,
        attributed: &PostRichTextAttributedString,
        start: usize,
        count: usize,
        width: f32,
    ) -> usize {
        let text = attributed.string();
        let mut count = count;
        loop {
            let Some(line_text) = text.get(start..start + count) else {
                return count;
            };
            let first = character_prefix(line_text, 1).len();
            if first == 0 || count <= first {
                return count;
            }
            let line = Self::create_line(shaper, text_system, attributed, start..start + count);
            if line.width - line.trailing_whitespace_width <= width {
                return count;
            }
            let x_at = |byte: usize| line.offset_for_string_index(ns_range::byte_to_utf16_offset(text, start + byte));
            let mut word_break = None;
            let mut cluster_break = None;
            let mut position = 0;
            while position < count {
                let cluster_length = character_prefix(&line_text[position..], 1).len();
                if cluster_length == 0 {
                    break;
                }
                let end = position + cluster_length;
                if end >= count {
                    break;
                }
                let cluster = &line_text[position..end];
                if !cluster.chars().all(char::is_whitespace) && x_at(end) > width {
                    break;
                }
                cluster_break = Some(end);
                if let (Some(previous), Some(next)) = (cluster.chars().last(), line_text[end..].chars().next())
                    && Self::is_break_opportunity(previous, next)
                {
                    word_break = Some(end);
                }
                position = end;
            }
            count = word_break.or(cluster_break).unwrap_or(first);
        }
    }

    /// greedy guess at where a line can break: after whitespace, hyphens and slashes or between ideographs
    fn is_break_opportunity(previous: char, next: char) -> bool {
        if next.is_whitespace() {
            return false;
        }
        if previous.is_whitespace() || previous == '\u{200B}' {
            return true;
        }
        if matches!(previous, '-' | '/' | '\u{2010}' | '\u{2013}' | '\u{2014}') {
            return next.is_alphanumeric();
        }
        Self::is_ideograph(previous) || Self::is_ideograph(next)
    }

    fn is_ideograph(character: char) -> bool {
        matches!(
            character as u32,
            0x2E80..=0x303F | 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF
                | 0x20000..=0x2FFFF
        )
    }

    fn truncated_line(
        shaper: &WindowTextSystem,
        text_system: &Arc<TextSystem>,
        drawable: &PostRichTextAttributedString,
        bytes: Range<usize>,
        width: f32,
    ) -> Option<PostTextLayoutBuilderPlacedLine> {
        let text = drawable.string();
        let length = drawable.length();
        let start = ns_range::byte_to_utf16_offset(text, bytes.start);
        let count = ns_range::byte_to_utf16_offset(text, bytes.end) - start;
        let paragraph_end = ns_range::byte_to_utf16_offset(text, Self::paragraph_end(text, bytes.start));
        let mut end = length.min(paragraph_end).min(start + count * 2 + 64);
        if end <= start {
            return None;
        }
        while ns_range::utf16_to_byte_offset(text, end).is_none() {
            end -= 1;
        }
        if end <= start {
            return None;
        }
        let has_more_text = end < length;

        let source = drawable.attributed_substring(NSRange::new(start, end - start))?;
        let visible_length = ns_range::length_utf16(source.string().trim_end_matches(Self::is_line_break));
        if visible_length == 0 {
            return None;
        }
        let mut source = source.attributed_substring(NSRange::new(0, visible_length))?;

        let last_attributes = source.attributes_at(visible_length - 1).cloned().unwrap_or_default();
        let ellipsis_attributes = AttributeContainer {
            background_color: None,
            strikethrough_style: None,
            ..last_attributes
        };
        let mut ellipsis_string =
            PostRichTextAttributedString::new(Self::ELLIPSIS, ellipsis_attributes, source.paragraph_style());
        ellipsis_string.add_truncation(NSRange::new(0, ellipsis_string.length()));
        let ellipsis_line = Self::whole_line(shaper, text_system, &ellipsis_string);
        let visible = source.clone();
        if has_more_text {
            source.append(ellipsis_string.clone());
        }

        let full_line = Self::whole_line(shaper, text_system, &source);
        let full_width = full_line.width - full_line.trailing_whitespace_width;
        let line = if full_width <= width {
            full_line
        } else {
            let kept = Self::truncated_length(&full_line, &visible, width - ellipsis_line.width)?;
            let mut cut = visible.attributed_substring(NSRange::new(0, kept))?;
            cut.append(ellipsis_string);
            Self::whole_line(shaper, text_system, &cut)
        };

        let visible_end = line
            .pieces
            .iter()
            .filter(|piece| !piece.is_truncation)
            .map(|piece| piece.utf16_start + piece.utf16_length)
            .max()
            .unwrap_or(0);
        Some(PostTextLayoutBuilderPlacedLine {
            line,
            visible_range: NSRange::new(start, visible_end.min(visible_length)),
            line_index_offset: start,
            is_truncated: true,
        })
    }

    fn whole_line(
        shaper: &WindowTextSystem,
        text_system: &Arc<TextSystem>,
        source: &PostRichTextAttributedString,
    ) -> PostTextCtLine {
        Self::create_line(shaper, text_system, source, 0..source.string().len())
    }

    fn truncated_length(line: &PostTextCtLine, visible: &PostRichTextAttributedString, available: f32) -> Option<usize> {
        if available < 0.0 {
            return None;
        }
        let text = visible.string();
        let mut kept = 0_usize;
        let mut byte = 0_usize;
        while byte < text.len() {
            let cluster = character_prefix(&text[byte..], 1);
            let next = kept + ns_range::length_utf16(cluster);
            if line.offset_for_string_index(next) > available {
                break;
            }
            kept = next;
            byte += cluster.len();
        }
        Some(kept)
    }

    /// line spacing from the paragraph style, plus paragraph spacing if the previous line ended a paragraph
    fn spacing(range: NSRange, attributed: &PostRichTextAttributedString) -> f32 {
        if range.length == 0 || range.location >= attributed.length() {
            return 0.0;
        }
        let style = attributed.paragraph_style();
        let mut value = style.line_spacing.max(0.0);
        let ends_paragraph = ns_range::substring(attributed.string(), range)
            .and_then(|line| line.chars().next_back())
            .is_some_and(Self::is_paragraph_break);
        if ends_paragraph {
            value += style.paragraph_spacing.max(0.0);
        }
        value
    }

    fn is_line_break(character: char) -> bool {
        matches!(character, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
    }

    fn is_paragraph_break(character: char) -> bool {
        matches!(character, '\n' | '\r' | '\u{0085}' | '\u{2029}')
    }

    fn pixel_ceil(value: f32, scale: f32) -> f32 {
        if !value.is_finite() || value <= 0.0 {
            return 0.0;
        }
        (value * scale - 0.001).ceil().max(0.0) / scale
    }

    /// horizontal extent of range on every line it touches
    fn segments(range: NSRange, lines: &[PostTextLayoutLine]) -> Vec<PostTextLayoutBuilderSegment<'_>> {
        let mut result = Vec::new();
        let range_end = range.upper_bound();
        for line in lines {
            if line.visible_range.location >= range_end {
                break;
            }
            let Some(overlap) = range.intersection(line.visible_range) else {
                continue;
            };
            let start_x = line.line.offset_for_string_index(overlap.location - line.line_index_offset);
            let end_x = line.line.offset_for_string_index(overlap.upper_bound() - line.line_index_offset);
            let baseline_x = f32::from(line.baseline_origin.x);
            let min_x = start_x.min(end_x) + baseline_x;
            let max_x = start_x.max(end_x) + baseline_x;
            if max_x - min_x <= 0.01 {
                continue;
            }
            result.push(PostTextLayoutBuilderSegment { line, min_x, max_x });
        }
        result
    }

    fn line_box(segment: &PostTextLayoutBuilderSegment<'_>) -> Bounds<Pixels> {
        bounds(
            point(px(segment.min_x), px(segment.line.top)),
            size(px(segment.max_x - segment.min_x), px(segment.line.height)),
        )
    }

    fn tokens(attributed: &PostRichTextAttributedString, lines: &[PostTextLayoutLine]) -> Vec<PostTextToken> {
        let mut result = Vec::new();
        for key in PostRichTextAttribute::TOKEN_KEYS {
            for (token_range, value) in attributed.token_spans(key) {
                if token_range.length == 0 {
                    continue;
                }
                let rects: Vec<Bounds<Pixels>> = Self::segments(token_range, lines).iter().map(Self::line_box).collect();
                if rects.is_empty() {
                    continue;
                }
                result.push(PostTextToken {
                    kind: value.kind.clone(),
                    utf16_range: token_range.lower_bound()..token_range.upper_bound(),
                    rects,
                });
            }
        }
        result
    }

    /// background color runs as line box rects
    fn backgrounds(attributed: &PostRichTextAttributedString, lines: &[PostTextLayoutLine]) -> Vec<PostTextLayoutFill> {
        let mut result = Vec::new();
        for (fill_range, color) in attributed.background_runs() {
            for segment in Self::segments(fill_range, lines).iter() {
                result.push(PostTextLayoutFill {
                    rect: Self::line_box(segment),
                    color,
                });
            }
        }
        result
    }

    /// strikethrough runs as bars at half x-height, same color as the glyphs
    fn strikethroughs(
        attributed: &PostRichTextAttributedString,
        lines: &[PostTextLayoutLine],
        scale: f32,
        text_system: &Arc<TextSystem>,
    ) -> Vec<PostTextLayoutStroke> {
        let mut result = Vec::new();
        for (piece_range, font, color) in attributed.strikethrough_runs() {
            let point_size = font.map_or(PostRichTextCalibration::PARAGRAPH_FONT_SIZE, |font| font.size);
            let thickness = (1.0 / scale)
                .max((point_size * PostRichTextCalibration::STRIKETHROUGH_THICKNESS_EM * scale).round() / scale);
            let x_height = font.map_or(point_size * 0.5, |font| {
                f32::from(text_system.x_height(Self::font_id(text_system, &font), px(font.size)))
            });
            let offset = x_height * PostRichTextCalibration::STRIKETHROUGH_X_HEIGHT_OFFSET;
            let bar_color = color.unwrap_or_else(gpui::black);
            for segment in Self::segments(piece_range, lines).iter() {
                let center_y = f32::from(segment.line.baseline_origin.y) - offset;
                let y = ((center_y - thickness / 2.0) * scale).round() / scale;
                result.push(PostTextLayoutStroke {
                    rect: bounds(
                        point(px(segment.min_x), px(y)),
                        size(px(segment.max_x - segment.min_x), px(thickness)),
                    ),
                    color: bar_color,
                });
            }
        }
        result
    }
}
