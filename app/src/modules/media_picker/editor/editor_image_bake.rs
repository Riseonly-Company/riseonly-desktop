use std::borrow::Cow;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};

use gpui::{
    App, AssetSource, Bounds, FontWeight, Hsla, LineFragment, Point, RenderImage, Rgba,
    SMOOTH_SVG_SCALE_FACTOR, SharedString, Size, SvgRenderer, TextSystem, hsla, px,
};
use image::codecs::png::{CompressionType, FilterType as PngFilterType, PngEncoder};
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, Frame, ImageEncoder, RgbaImage};
use rise_core::foundation::foundation_data::base64_encoded_string;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT};

use crate::core::ui::clever_image_disk_cache::CleverImageBitmapEncoder;
use crate::modules::media_picker::editor::editor_adjustments_renderer::EditorAdjustmentsRenderer;
use crate::modules::media_picker::editor::editor_crop_auto_fit::EditorCropAutoFitPlanner;
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::media_picker::editor::editor_link_entity::{EditorLinkEntity, EditorLinkRenderer};
use crate::modules::media_picker::editor::editor_state::{
    EditorDrawStroke, EditorDrawStrokeTool, EditorImageState, EditorOverlayCoordinateSpace,
    EditorTextEntity, EditorTextEntityStyleMode, EditorTextEntityTextAlignment,
};
use crate::modules::media_picker::editor::editor_sticker_entity::{EditorStickerEntity, EditorStickerRenderer};
use crate::modules::media_picker::editor::story_canvas_renderer::StoryCanvasRenderer;

/// text plate insets and corner as fractions of the font size
/// shared by the live label and the bake so the sent photo keeps the plate the editor showed
pub enum EditorImageBakeTextPlate {}

impl EditorImageBakeTextPlate {
    pub const PLATE_INSET_X: f32 = 0.74;
    pub const PLATE_INSET_Y: f32 = 0.46;
    pub const OUTLINE_INSET_X: f32 = 0.46;
    pub const OUTLINE_INSET_Y: f32 = 0.37;
    pub const CORNER_RADIUS: f32 = 0.37;
}

/// text system and svg rasterizer the marked renders draw with, bound once at launch
/// so the bake runs on any thread
#[derive(Clone)]
pub struct EditorImageBakeCanvas {
    text_system: Arc<TextSystem>,
    svg_renderer: SvgRenderer,
}

static BAKE_CANVAS: OnceLock<EditorImageBakeCanvas> = OnceLock::new();

struct EditorImageBakeFonts {
    assets: Arc<dyn AssetSource>,
}

impl EditorImageBakeFonts {
    const SANS_SLOT: &'static str = "fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf";
    const SEMIBOLD_FACE: &'static str = "fonts/Inter-SemiBold.ttf";
}

impl AssetSource for EditorImageBakeFonts {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let source = if path == Self::SANS_SLOT { Self::SEMIBOLD_FACE } else { path };
        self.assets.load(source)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        self.assets.list(path)
    }
}

/// one wrapped text line with its shaped width
struct EditorBakeTextLine {
    text: String,
    width: f64,
}

pub enum EditorImageBake {}

impl EditorImageBake {
    pub const BRUSH_BLUR_PREVIEW_MAX_PIXEL_SIDE: f32 = 1024.0;
    pub const BRUSH_BLUR_EXPORT_MAX_PIXEL_SIDE: f32 = 2048.0;
    pub const THUMBNAIL_PREVIEW_MAX_PIXEL_SIDE: f32 = 480.0;
    const BRUSH_BLUR_SIGMA_SHARE: f64 = 0.025;
    const MARKER_ALPHA: f32 = 0.45;
    const NEON_SHADOW_STD_DEVIATION: f64 = 4.0;
    const OVERLAY_KEEP_MARGIN: f64 = 0.15;
    const ARROW_HEAD_SHORT_SIDE_SHARE: f64 = 0.04;
    const MINIMUM_FONT_SIZE: f64 = 8.0;
    const OUTLINE_STROKE_SHARE: f64 = 0.02;
    const OUTLINE_STROKE_ALPHA: f32 = 0.4;
    const TRANSLUCENT_PLATE_ALPHA: f32 = 0.5;
    const DARK_FOREGROUND_WHITE: f32 = 0.08;
    const FOREGROUND_LUMINANCE_THRESHOLD: f32 = 0.55;

    /// the text system bound at launch, for callers that measure text off the frame
    pub(crate) fn text_system() -> Option<Arc<TextSystem>> {
        BAKE_CANVAS.get().map(|canvas| Arc::clone(&canvas.text_system))
    }

    pub fn install(cx: &App) {
        let canvas = EditorImageBakeCanvas {
            text_system: Arc::clone(cx.text_system()),
            svg_renderer: SvgRenderer::new(Arc::new(EditorImageBakeFonts {
                assets: Arc::clone(cx.asset_source()),
            })),
        };
        BAKE_CANVAS.set(canvas).ok();
    }

    pub fn apply_orientation(
        image: &Arc<RenderImage>,
        rotation_steps: i64,
        mirrored: bool,
        free_rotation_degrees: f64,
    ) -> Arc<RenderImage> {
        let mut result = Self::apply_rotation(image, rotation_steps);
        if mirrored && let Some(mut bitmap) = Self::bitmap(&result) {
            imageops::flip_horizontal_in_place(&mut bitmap);
            result = Self::render_image(bitmap);
        }
        if free_rotation_degrees.abs() > EditorImageLayout::GEOMETRY_EPSILON_DEGREES
            && let Some(bitmap) = Self::bitmap(&result)
        {
            result = Self::render_image(Self::rotated(&bitmap, free_rotation_degrees * PI / 180.0));
        }
        result
    }

    pub fn apply_crop(image: &Arc<RenderImage>, relative: Option<Bounds<f32>>) -> Arc<RenderImage> {
        let Some(relative) = relative else {
            return Arc::clone(image);
        };
        if EditorCropAutoFitPlanner::is_effectively_full(Some(relative)) {
            return Arc::clone(image);
        }
        let Some(bitmap) = Self::bitmap(image) else {
            return Arc::clone(image);
        };
        let width = f64::from(bitmap.width());
        let height = f64::from(bitmap.height());
        let min_x = (f64::from(relative.origin.x) * width).floor().max(0.0);
        let min_y = (f64::from(relative.origin.y) * height).floor().max(0.0);
        let max_x = (f64::from(relative.origin.x + relative.size.width) * width).ceil().min(width);
        let max_y = (f64::from(relative.origin.y + relative.size.height) * height).ceil().min(height);
        if max_x - min_x <= 0.0 || max_y - min_y <= 0.0 {
            return Arc::clone(image);
        }
        let cropped = imageops::crop_imm(
            &bitmap,
            min_x as u32,
            min_y as u32,
            (max_x - min_x) as u32,
            (max_y - min_y) as u32,
        )
        .to_image();
        Self::render_image(cropped)
    }

    pub fn commit_canvas(
        source: &Arc<RenderImage>,
        state: &EditorImageState,
    ) -> Option<(Arc<RenderImage>, EditorImageState)> {
        let crop_rel = state.crop_rect_relative.unwrap_or_else(Self::full_rect);
        let has_crop = !EditorCropAutoFitPlanner::is_effectively_full(state.crop_rect_relative);
        let has_orient = state.rotation_steps % 4 != 0
            || state.is_mirrored
            || state.free_rotation_degrees.abs() > EditorImageLayout::GEOMETRY_EPSILON_DEGREES;
        if !has_crop && !has_orient {
            return None;
        }

        let mut image = Self::apply_orientation(
            source,
            state.rotation_steps,
            state.is_mirrored,
            state.free_rotation_degrees,
        );
        if has_crop {
            image = Self::apply_crop(&image, Some(crop_rel));
        }

        let mut new_state = Self::remap_overlay_coordinates(
            state,
            EditorOverlayCoordinateSpace {
                crop_rect_relative: crop_rel,
                rotation_steps: state.rotation_steps,
                free_rotation_degrees: state.free_rotation_degrees,
                is_mirrored: state.is_mirrored,
            },
        );
        new_state.crop_baked_into_canvas = true;
        new_state.was_edited = true;
        Some((image, new_state))
    }

    pub fn remap_overlay_coordinates(
        state: &EditorImageState,
        destination: EditorOverlayCoordinateSpace,
    ) -> EditorImageState {
        let mut s = state.clone();
        let source = state
            .overlay_coordinate_space
            .unwrap_or_else(|| Self::implicit_overlay_space(state));
        let source_crop = source.crop_rect_relative;
        let destination_crop = destination.crop_rect_relative;
        if source_crop.size.width <= 0.0
            || source_crop.size.height <= 0.0
            || destination_crop.size.width <= 0.0
            || destination_crop.size.height <= 0.0
        {
            return s;
        }
        let source_canvas_short_side = Self::overlay_canvas_short_side(state.source_pixel_size, &source);
        let destination_canvas_short_side =
            Self::overlay_canvas_short_side(state.source_pixel_size, &destination);
        let overlay_size_scale = if destination_canvas_short_side > 0.0 {
            source_canvas_short_side / destination_canvas_short_side
        } else {
            1.0
        };

        let remap = |p: Point<f32>| -> (f64, f64) {
            let old_oriented = Point {
                x: source_crop.origin.x + p.x * source_crop.size.width,
                y: source_crop.origin.y + p.y * source_crop.size.height,
            };
            let source_point = Self::map_from_oriented_unit(old_oriented, state.source_pixel_size, &source);
            let new_oriented = Self::map_to_oriented_unit(source_point, state.source_pixel_size, &destination);
            (
                f64::from(new_oriented.x - destination_crop.origin.x) / f64::from(destination_crop.size.width),
                f64::from(new_oriented.y - destination_crop.origin.y) / f64::from(destination_crop.size.height),
            )
        };
        let keeps = |(x, y): (f64, f64)| {
            let low = -Self::OVERLAY_KEEP_MARGIN;
            let high = 1.0 + Self::OVERLAY_KEEP_MARGIN;
            x >= low && x <= high && y >= low && y <= high
        };
        let clamped = |(x, y): (f64, f64)| Point {
            x: x.clamp(0.0, 1.0) as f32,
            y: y.clamp(0.0, 1.0) as f32,
        };

        s.strokes = s
            .strokes
            .iter()
            .filter_map(|stroke| {
                let points: Vec<(f64, f64)> = stroke.points.iter().map(|point| remap(*point)).collect();
                if points.len() < 2 {
                    return None;
                }
                let min_x = points.iter().map(|point| point.0).fold(f64::INFINITY, f64::min);
                let max_x = points.iter().map(|point| point.0).fold(f64::NEG_INFINITY, f64::max);
                let min_y = points.iter().map(|point| point.1).fold(f64::INFINITY, f64::min);
                let max_y = points.iter().map(|point| point.1).fold(f64::NEG_INFINITY, f64::max);
                let visible = max_x >= 0.0 && min_x <= 1.0 && max_y >= 0.0 && min_y <= 1.0;
                if !visible {
                    return None;
                }
                let mut st = stroke.clone();
                st.points = points
                    .into_iter()
                    .map(|(x, y)| Point { x: x as f32, y: y as f32 })
                    .collect();
                st.width = (f64::from(st.width) * overlay_size_scale) as f32;
                Some(st)
            })
            .collect();

        s.texts = s
            .texts
            .iter()
            .filter_map(|entity| {
                let center = remap(entity.center);
                if !keeps(center) {
                    return None;
                }
                let mut e = entity.clone();
                e.center = clamped(center);
                e.font_size = (f64::from(e.font_size) * overlay_size_scale) as f32;
                Some(e)
            })
            .collect();
        if let Some(stickers) = s.stickers.take() {
            s.stickers = Some(
                stickers
                    .into_iter()
                    .filter_map(|entity| {
                        let center = remap(entity.center);
                        if !keeps(center) {
                            return None;
                        }
                        let mut e = entity;
                        e.center = clamped(center);
                        e.scale = (f64::from(e.scale) * overlay_size_scale) as f32;
                        Some(e)
                    })
                    .collect(),
            );
        }
        if let Some(tags) = s.person_tags.take() {
            s.person_tags = Some(
                tags.into_iter()
                    .filter_map(|entity| {
                        let center = remap(entity.center);
                        if !keeps(center) {
                            return None;
                        }
                        let mut e = entity;
                        e.center = clamped(center);
                        Some(e)
                    })
                    .collect(),
            );
        }
        if let Some(plate) = s.music_plate.take() {
            let center = remap(plate.center);
            let mut p = plate;
            p.center = clamped(center);
            p.scale = (f64::from(p.scale) * overlay_size_scale) as f32;
            s.music_plate = Some(p);
        }
        s.overlay_coordinate_space = Some(destination);
        s
    }

    pub fn map_to_oriented_unit(
        source_unit_point: Point<f32>,
        source_size: Size<f32>,
        space: &EditorOverlayCoordinateSpace,
    ) -> Point<f32> {
        let safe_size = Self::safe_size(source_size);
        let mut point = (
            f64::from(source_unit_point.x) * safe_size.0,
            f64::from(source_unit_point.y) * safe_size.1,
        );
        let mut size = safe_size;
        (point, size) = Self::rotate(point, size, Self::quarter_radians(space.rotation_steps));
        if space.is_mirrored {
            point.0 = size.0 - point.0;
        }
        (point, size) = Self::rotate(point, size, space.free_rotation_degrees * PI / 180.0);
        Point {
            x: (point.0 / size.0.max(1.0)) as f32,
            y: (point.1 / size.1.max(1.0)) as f32,
        }
    }

    pub fn map_from_oriented_unit(
        oriented_unit_point: Point<f32>,
        source_size: Size<f32>,
        space: &EditorOverlayCoordinateSpace,
    ) -> Point<f32> {
        let safe_size = Self::safe_size(source_size);
        let free_radians = space.free_rotation_degrees * PI / 180.0;
        let step_size = Self::rotated_size(safe_size, Self::quarter_radians(space.rotation_steps));
        let oriented_size = Self::rotated_size(step_size, free_radians);
        let mut point = (
            f64::from(oriented_unit_point.x) * oriented_size.0,
            f64::from(oriented_unit_point.y) * oriented_size.1,
        );
        point = Self::inverse_rotate(point, step_size, oriented_size, free_radians);
        if space.is_mirrored {
            point.0 = step_size.0 - point.0;
        }
        point = Self::inverse_rotate(
            point,
            safe_size,
            step_size,
            Self::quarter_radians(space.rotation_steps),
        );
        Point {
            x: (point.0 / safe_size.0) as f32,
            y: (point.1 / safe_size.1) as f32,
        }
    }

    fn quarter_radians(steps: i64) -> f64 {
        (((steps % 4) + 4) % 4) as f64 * PI / 2.0
    }

    fn overlay_canvas_short_side(source_size: Size<f32>, space: &EditorOverlayCoordinateSpace) -> f64 {
        let safe_size = Self::safe_size(source_size);
        let step_size = Self::rotated_size(safe_size, Self::quarter_radians(space.rotation_steps));
        let oriented_size = Self::rotated_size(step_size, space.free_rotation_degrees * PI / 180.0);
        let crop = space.crop_rect_relative;
        (oriented_size.0 * f64::from(crop.size.width)).min(oriented_size.1 * f64::from(crop.size.height))
    }

    fn safe_size(size: Size<f32>) -> (f64, f64) {
        (f64::from(size.width).max(1.0), f64::from(size.height).max(1.0))
    }

    fn rotated_size(size: (f64, f64), radians: f64) -> (f64, f64) {
        let c = radians.cos().abs();
        let s = radians.sin().abs();
        (size.0 * c + size.1 * s, size.0 * s + size.1 * c)
    }

    fn rotate(point: (f64, f64), source_size: (f64, f64), radians: f64) -> ((f64, f64), (f64, f64)) {
        let destination_size = Self::rotated_size(source_size, radians);
        let dx = point.0 - source_size.0 / 2.0;
        let dy = point.1 - source_size.1 / 2.0;
        let c = radians.cos();
        let s = radians.sin();
        (
            (
                destination_size.0 / 2.0 + dx * c - dy * s,
                destination_size.1 / 2.0 + dx * s + dy * c,
            ),
            destination_size,
        )
    }

    fn inverse_rotate(
        point: (f64, f64),
        source_size: (f64, f64),
        destination_size: (f64, f64),
        radians: f64,
    ) -> (f64, f64) {
        let dx = point.0 - destination_size.0 / 2.0;
        let dy = point.1 - destination_size.1 / 2.0;
        let c = radians.cos();
        let s = radians.sin();
        (
            source_size.0 / 2.0 + dx * c + dy * s,
            source_size.1 / 2.0 - dx * s + dy * c,
        )
    }

    fn apply_rotation(image: &Arc<RenderImage>, steps: i64) -> Arc<RenderImage> {
        let normalized = ((steps % 4) + 4) % 4;
        if normalized == 0 {
            return Arc::clone(image);
        }
        let Some(bitmap) = Self::bitmap(image) else {
            return Arc::clone(image);
        };
        let turned = match normalized {
            1 => imageops::rotate90(&bitmap),
            2 => imageops::rotate180(&bitmap),
            _ => imageops::rotate270(&bitmap),
        };
        Self::render_image(turned)
    }

    /// free turn drawn into a canvas grown to the turned bounds, corners left clear
    fn rotated(source: &RgbaImage, radians: f64) -> RgbaImage {
        let source_size = (f64::from(source.width()), f64::from(source.height()));
        let size = Self::rotated_size(source_size, radians);
        let width = size.0.ceil().max(1.0) as u32;
        let height = size.1.ceil().max(1.0) as u32;
        let c = radians.cos();
        let s = radians.sin();
        let mut output = RgbaImage::new(width, height);
        for (x, y, pixel) in output.enumerate_pixels_mut() {
            let dx = f64::from(x) + 0.5 - size.0 / 2.0;
            let dy = f64::from(y) + 0.5 - size.1 / 2.0;
            let sx = source_size.0 / 2.0 + dx * c + dy * s;
            let sy = source_size.1 / 2.0 - dx * s + dy * c;
            pixel.0 = Self::bilinear(source, sx - 0.5, sy - 0.5);
        }
        output
    }

    fn bilinear(source: &RgbaImage, x: f64, y: f64) -> [u8; 4] {
        let x0 = x.floor();
        let y0 = y.floor();
        let fx = x - x0;
        let fy = y - y0;
        let mut accumulated = [0.0_f64; 4];
        for (ox, oy, weight) in [
            (0.0, 0.0, (1.0 - fx) * (1.0 - fy)),
            (1.0, 0.0, fx * (1.0 - fy)),
            (0.0, 1.0, (1.0 - fx) * fy),
            (1.0, 1.0, fx * fy),
        ] {
            let sx = x0 + ox;
            let sy = y0 + oy;
            if weight <= 0.0
                || sx < 0.0
                || sy < 0.0
                || sx >= f64::from(source.width())
                || sy >= f64::from(source.height())
            {
                continue;
            }
            let texel = source.get_pixel(sx as u32, sy as u32).0;
            let alpha = f64::from(texel[3]) / 255.0;
            for (sum, value) in accumulated.iter_mut().zip(&texel[..3]) {
                *sum += f64::from(*value) * alpha * weight;
            }
            accumulated[3] += alpha * weight;
        }
        let alpha = accumulated[3];
        if alpha <= 0.0 {
            return [0; 4];
        }
        [
            (accumulated[0] / alpha).round().clamp(0.0, 255.0) as u8,
            (accumulated[1] / alpha).round().clamp(0.0, 255.0) as u8,
            (accumulated[2] / alpha).round().clamp(0.0, 255.0) as u8,
            (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
        ]
    }

    pub fn render_edited_image(base_image: &Arc<RenderImage>, state: &EditorImageState) -> Arc<RenderImage> {
        if let Some(canvas) = &state.story_canvas {
            return StoryCanvasRenderer::render_default(state, canvas);
        }
        let adjusted = EditorAdjustmentsRenderer::shared().render(base_image, &state.adjustments);
        let oriented = Self::apply_orientation(
            &adjusted,
            state.rotation_steps,
            state.is_mirrored,
            state.free_rotation_degrees,
        );
        let cropped = Self::apply_crop(&oriented, state.crop_rect_relative);
        let stickers = state.stickers.clone().unwrap_or_default();
        let links = state.links.clone().unwrap_or_default();
        if state.strokes.is_empty() && state.texts.is_empty() && stickers.is_empty() && links.is_empty() {
            return cropped;
        }
        let Some(mut ctx) = CleverImageBitmapEncoder::rgba(&cropped) else {
            return cropped;
        };
        let size = Size { width: ctx.width() as f32, height: ctx.height() as f32 };
        Self::draw_strokes(&state.strokes, size, &mut ctx, &cropped);
        Self::draw_stacked(&state.texts, &stickers, &links, state.entity_order.as_deref(), size, &mut ctx);
        Self::render_image_from_rgba(ctx)
    }

    /// texts and stickers and links in the author's stacking order
    /// those in order go on top in that order and the rest under them in type order
    pub fn draw_stacked(
        texts: &[EditorTextEntity],
        stickers: &[EditorStickerEntity],
        links: &[EditorLinkEntity],
        order: Option<&[Uuid]>,
        size: Size<f32>,
        ctx: &mut RgbaImage,
    ) {
        let Some(order) = order.filter(|order| !order.is_empty()) else {
            Self::draw_texts(texts, size, ctx);
            EditorStickerRenderer::draw_still(stickers, size, ctx);
            EditorLinkRenderer::draw(links, size, ctx);
            return;
        };
        enum Item<'a> {
            Text(&'a EditorTextEntity),
            Sticker(&'a EditorStickerEntity),
            Link(&'a EditorLinkEntity),
        }
        let mut rank: HashMap<Uuid, i64> = HashMap::new();
        for (index, id) in order.iter().enumerate() {
            rank.insert(*id, index as i64);
        }
        let rank_of = |id: &Uuid| rank.get(id).copied().unwrap_or(-1);
        let mut items: Vec<(i64, usize, Item<'_>)> = Vec::new();
        for text in texts {
            items.push((rank_of(&text.id), items.len(), Item::Text(text)));
        }
        for sticker in stickers {
            items.push((rank_of(&sticker.id), items.len(), Item::Sticker(sticker)));
        }
        for link in links {
            items.push((rank_of(&link.id), items.len(), Item::Link(link)));
        }
        items.sort_by_key(|entry| (entry.0, entry.1));
        let mut pending_texts: Vec<EditorTextEntity> = Vec::new();
        for (_, _, item) in items {
            match item {
                Item::Text(text) => pending_texts.push(text.clone()),
                Item::Sticker(sticker) => {
                    Self::draw_texts(&std::mem::take(&mut pending_texts), size, ctx);
                    EditorStickerRenderer::draw_still(std::slice::from_ref(sticker), size, ctx);
                }
                Item::Link(link) => {
                    Self::draw_texts(&std::mem::take(&mut pending_texts), size, ctx);
                    EditorLinkRenderer::draw(std::slice::from_ref(link), size, ctx);
                }
            }
        }
        Self::draw_texts(&pending_texts, size, ctx);
    }

    /// strokes in the order drawn
    /// blur brush shows blur_base blurred through its path
    /// eraser masks everything drawn before it like the clear blend mode
    pub fn draw_strokes(strokes: &[EditorDrawStroke], size: Size<f32>, ctx: &mut RgbaImage, blur_base: &Arc<RenderImage>) {
        if strokes.is_empty() {
            return;
        }
        let Some(canvas) = BAKE_CANVAS.get() else {
            return;
        };
        let width = ctx.width();
        let height = ctx.height();
        let short_side = f64::from(size.width.min(size.height));

        let mut defs = String::new();
        let Some(mut body) = Self::image_element(ctx, width, height, None) else {
            return;
        };
        let mut brush_blur: Option<Option<RgbaImage>> = None;
        for (index, stroke) in strokes.iter().enumerate() {
            let line_width = f64::from(stroke.width) * short_side;
            let path_data = Self::path_data(&Self::stroke_path(stroke, size, line_width as f32));
            match stroke.tool {
                EditorDrawStrokeTool::Pen | EditorDrawStrokeTool::Pencil | EditorDrawStrokeTool::Arrow => {
                    body.push_str(&Self::stroke_element(&path_data, line_width, stroke.color, None));
                }
                EditorDrawStrokeTool::Marker => {
                    let color = Hsla { a: Self::MARKER_ALPHA, ..stroke.color };
                    body.push_str(&Self::stroke_element(&path_data, line_width, color, None));
                }
                EditorDrawStrokeTool::Neon => {
                    let filter_id = format!("neon{index}");
                    let (hex, opacity) = Self::svg_color(stroke.color);
                    let _ = write!(
                        defs,
                        r#"<filter id="{filter_id}" filterUnits="userSpaceOnUse" x="0" y="0" width="{width}" height="{height}"><feDropShadow dx="0" dy="0" stdDeviation="{}" flood-color="{hex}" flood-opacity="{opacity}"/></filter>"#,
                        Self::NEON_SHADOW_STD_DEVIATION
                    );
                    body.push_str(&Self::stroke_element(&path_data, line_width, stroke.color, Some(&filter_id)));
                }
                EditorDrawStrokeTool::Blur => {
                    let blurred = brush_blur.get_or_insert_with(|| {
                        Self::brush_blur_image(blur_base, Self::BRUSH_BLUR_EXPORT_MAX_PIXEL_SIDE)
                            .and_then(|image| CleverImageBitmapEncoder::rgba(&image))
                    });
                    let Some(blurred) = blurred.as_ref() else {
                        continue;
                    };
                    let mask_id = format!("blur{index}");
                    let _ = write!(
                        defs,
                        r#"<mask id="{mask_id}" maskUnits="userSpaceOnUse" x="0" y="0" width="{width}" height="{height}">{}</mask>"#,
                        Self::stroke_element(&path_data, line_width, hsla(0.0, 0.0, 1.0, 1.0), None)
                    );
                    let Some(element) = Self::image_element(blurred, width, height, Some(&mask_id)) else {
                        continue;
                    };
                    body.push_str(&element);
                }
                EditorDrawStrokeTool::Eraser => {
                    let mask_id = format!("erase{index}");
                    let _ = write!(
                        defs,
                        r##"<mask id="{mask_id}" maskUnits="userSpaceOnUse" x="0" y="0" width="{width}" height="{height}"><rect x="0" y="0" width="{width}" height="{height}" fill="#FFFFFF"/>{}</mask>"##,
                        Self::stroke_element(&path_data, line_width, hsla(0.0, 0.0, 0.0, 1.0), None)
                    );
                    body = format!(r#"<g mask="url(#{mask_id})">{body}</g>"#);
                }
            }
        }
        if let Some(rendered) = Self::rasterize(canvas, &defs, &body, width, height) {
            *ctx = rendered;
        }
    }

    pub fn draw_texts(texts: &[EditorTextEntity], size: Size<f32>, ctx: &mut RgbaImage) {
        if texts.iter().all(|entity| entity.text.is_empty()) {
            return;
        }
        let Some(canvas) = BAKE_CANVAS.get() else {
            return;
        };
        let width = ctx.width();
        let height = ctx.height();
        let Some(mut body) = Self::image_element(ctx, width, height, None) else {
            return;
        };
        for entity in texts {
            Self::draw_text_entity(canvas, entity, size, &mut body);
        }
        if let Some(rendered) = Self::rasterize(canvas, "", &body, width, height) {
            *ctx = rendered;
        }
    }

    /// crop stage needs its own full size render only when marks must be drawn into it
    pub fn crop_stage_needs_marks(state: &EditorImageState) -> bool {
        !state.strokes.is_empty()
            || !state.texts.is_empty()
            || state.stickers.as_ref().is_some_and(|stickers| !stickers.is_empty())
    }

    /// crop mode stage, the adjusted source with strokes and texts drawn in source space
    /// so marks turn and mirror and crop with the photo while the live overlay is hidden
    /// full size render, call it off the main thread
    pub fn crop_stage_image(source: &Arc<RenderImage>, state: &EditorImageState) -> Arc<RenderImage> {
        if !Self::crop_stage_needs_marks(state) {
            return EditorAdjustmentsRenderer::shared().render(source, &state.adjustments);
        }
        let from = state
            .overlay_coordinate_space
            .unwrap_or_else(|| Self::implicit_overlay_space(state));
        let mut marks = Self::remap_overlay_coordinates(
            state,
            EditorOverlayCoordinateSpace {
                crop_rect_relative: Self::full_rect(),
                rotation_steps: 0,
                free_rotation_degrees: 0.0,
                is_mirrored: false,
            },
        );
        let free_radians = from.free_rotation_degrees * PI / 180.0;
        let quarter = Self::quarter_radians(from.rotation_steps);
        let unturn = |rotation: f32| -> f32 {
            let unturned = f64::from(rotation) - free_radians;
            ((if from.is_mirrored { -unturned } else { unturned }) - quarter) as f32
        };
        for entity in &mut marks.texts {
            entity.rotation = unturn(entity.rotation);
        }
        if let Some(stickers) = marks.stickers.as_mut() {
            for entity in stickers {
                entity.rotation = unturn(entity.rotation);
            }
        }
        marks.person_tags = None;
        marks.music_plate = None;
        marks.rotation_steps = 0;
        marks.free_rotation_degrees = 0.0;
        marks.is_mirrored = false;
        marks.crop_rect_relative = None;
        Self::render_edited_image(source, &marks)
    }

    /// normalized stroke points laid out in size as sub paths
    /// the stroke itself and for the arrow tool its head at the last point
    pub fn stroke_path(stroke: &EditorDrawStroke, size: Size<f32>, line_width: f32) -> Vec<Vec<Point<f32>>> {
        let width = f64::from(size.width);
        let height = f64::from(size.height);
        let points: Vec<(f64, f64)> = stroke
            .points
            .iter()
            .map(|point| (f64::from(point.x) * width, f64::from(point.y) * height))
            .collect();
        let as_point = |(x, y): (f64, f64)| Point { x: x as f32, y: y as f32 };
        let (Some(&first), Some(&tip)) = (points.first(), points.last()) else {
            return Vec::new();
        };
        let mut path = vec![points.iter().copied().map(as_point).collect::<Vec<_>>()];
        if stroke.tool != EditorDrawStrokeTool::Arrow {
            return path;
        }
        let head_length =
            (f64::from(line_width) * 3.0).max(width.min(height) * Self::ARROW_HEAD_SHORT_SIDE_SHARE);
        let tail = points
            .iter()
            .rev()
            .find(|point| (tip.0 - point.0).hypot(tip.1 - point.1) >= head_length * 0.5)
            .copied()
            .unwrap_or(first);
        let dx = tip.0 - tail.0;
        let dy = tip.1 - tail.1;
        if dx.hypot(dy) <= 1.0 {
            return path;
        }
        let angle = dy.atan2(dx);
        let spread = PI / 6.0;
        path.push(vec![
            as_point((
                tip.0 - head_length * (angle - spread).cos(),
                tip.1 - head_length * (angle - spread).sin(),
            )),
            as_point(tip),
            as_point((
                tip.0 - head_length * (angle + spread).cos(),
                tip.1 - head_length * (angle + spread).sin(),
            )),
        ]);
        path
    }

    /// what the blur brush paints through its stroke
    /// radius is a share of the short side so editor preview and export blur alike at any resolution
    pub fn brush_blur_image(image: &Arc<RenderImage>, max_pixel_side: f32) -> Option<Arc<RenderImage>> {
        let bitmap = Self::bitmap(image)?;
        let longest_pixel_side = f64::from(bitmap.width().max(bitmap.height()));
        if longest_pixel_side <= 0.0 {
            return None;
        }
        let factor = (f64::from(max_pixel_side) / longest_pixel_side).min(1.0);
        let upright = if factor < 1.0 {
            let width = (f64::from(bitmap.width()) * factor).ceil().max(1.0) as u32;
            let height = (f64::from(bitmap.height()) * factor).ceil().max(1.0) as u32;
            imageops::resize(&bitmap, width, height, FilterType::Triangle)
        } else {
            bitmap
        };
        let sigma = f64::from(upright.width().min(upright.height())) * Self::BRUSH_BLUR_SIGMA_SHARE;
        Some(Self::render_image(Self::gaussian_blur_clamped(&upright, sigma)))
    }

    pub fn thumbnail_preview(
        base_image: &Arc<RenderImage>,
        state: &EditorImageState,
        max_pixel_side: f32,
    ) -> Arc<RenderImage> {
        let full = Self::render_edited_image(base_image, state);
        let size = full.size(0);
        let w = f64::from(size.width.0);
        let h = f64::from(size.height.0);
        let side = w.max(h);
        let max_pixel_side = f64::from(max_pixel_side);
        if side <= max_pixel_side || side <= 1.0 {
            return full;
        }
        let scale = max_pixel_side / side;
        let target_width = (w * scale).floor().max(1.0) as u32;
        let target_height = (h * scale).floor().max(1.0) as u32;
        let Some(bitmap) = Self::bitmap(&full) else {
            return full;
        };
        Self::render_image(imageops::resize(&bitmap, target_width, target_height, FilterType::Triangle))
    }

    fn rasterize(canvas: &EditorImageBakeCanvas, defs: &str, body: &str, width: u32, height: u32) -> Option<RgbaImage> {
        let document = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><defs>{defs}</defs>{body}</svg>"#
        );
        let rendered = canvas
            .svg_renderer
            .render_single_frame(document.as_bytes(), 1.0 / SMOOTH_SVG_SCALE_FACTOR)
            .ok()?;
        let bitmap = CleverImageBitmapEncoder::rgba(&rendered)?;
        (bitmap.width() == width && bitmap.height() == height).then_some(bitmap)
    }

    fn draw_text_entity(
        canvas: &EditorImageBakeCanvas,
        entity: &EditorTextEntity,
        size: Size<f32>,
        document: &mut String,
    ) {
        if entity.text.is_empty() {
            return;
        }
        let width = f64::from(size.width);
        let height = f64::from(size.height);
        let short_side = width.min(height);
        let font_size = (f64::from(entity.font_size) * short_side).max(Self::MINIMUM_FONT_SIZE);
        let font = Self::semibold_font();

        let (text_color, bg_color) = match entity.style_mode {
            EditorTextEntityStyleMode::Background => {
                (Self::best_foreground_color(entity.color), Some(entity.color))
            }
            EditorTextEntityStyleMode::TranslucentBg => (
                entity.color,
                Some(hsla(0.0, 0.0, 1.0, Self::TRANSLUCENT_PLATE_ALPHA)),
            ),
            EditorTextEntityStyleMode::TextOnly => (entity.color, None),
        };
        let is_outline = entity.style_mode == EditorTextEntityStyleMode::TextOnly;

        let lines = Self::wrapped_lines(&canvas.text_system, &font, &entity.text, font_size, width);
        let line_height = font_size * f64::from(NATURAL_LINE_HEIGHT);
        let font_id = canvas.text_system.resolve_font(&font);
        let ascent = f64::from(f32::from(canvas.text_system.ascent(font_id, px(font_size as f32))));
        let measured_width = lines.iter().map(|line| line.width).fold(0.0, f64::max);
        let measured_height = line_height * lines.len() as f64;

        let (inset_x, inset_y) = if is_outline {
            (EditorImageBakeTextPlate::OUTLINE_INSET_X, EditorImageBakeTextPlate::OUTLINE_INSET_Y)
        } else {
            (EditorImageBakeTextPlate::PLATE_INSET_X, EditorImageBakeTextPlate::PLATE_INSET_Y)
        };
        let pad_x = font_size * f64::from(inset_x);
        let pad_y = font_size * f64::from(inset_y);
        let box_w = measured_width.ceil() + pad_x * 2.0;
        let box_h = measured_height.ceil() + pad_y * 2.0;
        let box_x = -box_w / 2.0;
        let box_y = -box_h / 2.0;

        let center_x = f64::from(entity.center.x) * width;
        let center_y = f64::from(entity.center.y) * height;
        let _ = write!(
            document,
            r#"<g transform="translate({center_x} {center_y}) rotate({}) scale({})">"#,
            f64::from(entity.rotation).to_degrees(),
            entity.scale
        );
        if let Some(bg_color) = bg_color {
            let corner_radius = font_size * f64::from(EditorImageBakeTextPlate::CORNER_RADIUS);
            let (hex, opacity) = Self::svg_color(bg_color);
            let _ = write!(
                document,
                r#"<rect x="{box_x}" y="{box_y}" width="{box_w}" height="{box_h}" rx="{corner_radius}" ry="{corner_radius}" fill="{hex}" fill-opacity="{opacity}"/>"#
            );
        }
        let draw_min_x = box_x + pad_x;
        let draw_width = box_w - pad_x * 2.0;
        let (anchor, x) = match entity.alignment {
            EditorTextEntityTextAlignment::Leading => ("start", draw_min_x),
            EditorTextEntityTextAlignment::Center => ("middle", draw_min_x + draw_width / 2.0),
            EditorTextEntityTextAlignment::Trailing => ("end", draw_min_x + draw_width),
        };
        let (fill_hex, fill_opacity) = Self::svg_color(text_color);
        let outline = if is_outline {
            let (stroke_hex, _) = Self::svg_color(hsla(0.0, 0.0, 0.0, 1.0));
            format!(
                r#" stroke="{stroke_hex}" stroke-opacity="{}" stroke-width="{}" stroke-linejoin="round""#,
                Self::OUTLINE_STROKE_ALPHA,
                font_size * Self::OUTLINE_STROKE_SHARE
            )
        } else {
            String::new()
        };
        let family = Self::escaped(&font.family);
        for (index, line) in lines.iter().enumerate() {
            let baseline = box_y + pad_y + line_height * index as f64 + ascent;
            let _ = write!(
                document,
                r#"<text x="{x}" y="{baseline}" text-anchor="{anchor}" xml:space="preserve" font-family="{family}" font-weight="{}" font-size="{font_size}" fill="{fill_hex}" fill-opacity="{fill_opacity}"{outline}>{}</text>"#,
                FontWeight::SEMIBOLD.0,
                Self::escaped(&line.text)
            );
        }
        document.push_str("</g>");
    }

    fn best_foreground_color(color: Hsla) -> Hsla {
        let rgba: Rgba = color.into();
        let luminance = 0.2126 * rgba.r + 0.7152 * rgba.g + 0.0722 * rgba.b;
        if luminance > Self::FOREGROUND_LUMINANCE_THRESHOLD {
            hsla(0.0, 0.0, Self::DARK_FOREGROUND_WHITE, 1.0)
        } else {
            hsla(0.0, 0.0, 1.0, 1.0)
        }
    }

    fn wrapped_lines(
        text_system: &Arc<TextSystem>,
        font: &gpui::Font,
        text: &str,
        font_size: f64,
        width: f64,
    ) -> Vec<EditorBakeTextLine> {
        let font_id = text_system.resolve_font(font);
        let font_px = px(font_size as f32);
        let mut wrapper = text_system.line_wrapper(font.clone(), font_px);
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let fragments = [LineFragment::text(paragraph)];
            let boundaries: Vec<usize> =
                wrapper.wrap_line(&fragments, px(width as f32)).map(|boundary| boundary.ix).collect();
            let mut start = 0;
            for end in boundaries.into_iter().chain([paragraph.len()]) {
                if let Some(line) = paragraph.get(start..end) {
                    let line = line.trim_end();
                    let line_width = line
                        .chars()
                        .map(|ch| f64::from(f32::from(text_system.layout_width(font_id, font_px, ch))))
                        .sum();
                    lines.push(EditorBakeTextLine { text: line.to_owned(), width: line_width });
                }
                start = end;
            }
        }
        lines
    }

    fn semibold_font() -> gpui::Font {
        let spec = Font::system_weight(0.0, FontWeight::SEMIBOLD);
        gpui::Font {
            family: spec.family(),
            features: spec.features(),
            fallbacks: None,
            weight: FontWeight::SEMIBOLD,
            style: spec.style(),
        }
    }

    fn stroke_element(path_data: &str, line_width: f64, color: Hsla, filter_id: Option<&str>) -> String {
        let (hex, opacity) = Self::svg_color(color);
        let filter = filter_id.map(|id| format!(r#" filter="url(#{id})""#)).unwrap_or_default();
        format!(
            r#"<path d="{path_data}" fill="none" stroke="{hex}" stroke-opacity="{opacity}" stroke-width="{line_width}" stroke-linecap="round" stroke-linejoin="round"{filter}/>"#
        )
    }

    fn path_data(path: &[Vec<Point<f32>>]) -> String {
        let mut data = String::new();
        for sub_path in path {
            for (index, point) in sub_path.iter().enumerate() {
                let command = if index == 0 { 'M' } else { 'L' };
                let _ = write!(data, "{command}{} {} ", point.x, point.y);
            }
        }
        data
    }

    /// bitmap as an embedded png drawn over the whole canvas, optionally through a mask
    fn image_element(bitmap: &RgbaImage, width: u32, height: u32, mask_id: Option<&str>) -> Option<String> {
        let png = Self::png_data(bitmap)?;
        let mask = mask_id.map(|id| format!(r#" mask="url(#{id})""#)).unwrap_or_default();
        Some(format!(
            r#"<image x="0" y="0" width="{width}" height="{height}" preserveAspectRatio="none"{mask} xlink:href="data:image/png;base64,{}"/>"#,
            base64_encoded_string(&png)
        ))
    }

    /// fast compression, the png only carries pixels into the rasterizer
    fn png_data(bitmap: &RgbaImage) -> Option<Vec<u8>> {
        let mut data = Vec::new();
        PngEncoder::new_with_quality(&mut data, CompressionType::Fast, PngFilterType::NoFilter)
            .write_image(bitmap.as_raw(), bitmap.width(), bitmap.height(), ExtendedColorType::Rgba8)
            .ok()?;
        Some(data)
    }

    fn svg_color(color: Hsla) -> (String, f32) {
        let rgba: Rgba = color.into();
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        (
            format!("#{:02X}{:02X}{:02X}", channel(rgba.r), channel(rgba.g), channel(rgba.b)),
            rgba.a.clamp(0.0, 1.0),
        )
    }

    fn escaped(text: &str) -> String {
        let mut escaped = String::with_capacity(text.len());
        for character in text.chars() {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&apos;"),
                other => escaped.push(other),
            }
        }
        escaped
    }

    fn gaussian_blur_clamped(source: &RgbaImage, sigma: f64) -> RgbaImage {
        let width = source.width() as usize;
        let height = source.height() as usize;
        if sigma <= 0.0 || width == 0 || height == 0 {
            return source.clone();
        }
        let radius = (sigma * 3.0).ceil() as isize;
        let mut kernel: Vec<f64> = (-radius..=radius)
            .map(|offset| (-((offset * offset) as f64) / (2.0 * sigma * sigma)).exp())
            .collect();
        let total: f64 = kernel.iter().sum();
        for weight in &mut kernel {
            *weight /= total;
        }
        let premultiplied: Vec<[f64; 4]> = source
            .pixels()
            .map(|pixel| {
                let alpha = f64::from(pixel.0[3]) / 255.0;
                [
                    f64::from(pixel.0[0]) * alpha,
                    f64::from(pixel.0[1]) * alpha,
                    f64::from(pixel.0[2]) * alpha,
                    alpha,
                ]
            })
            .collect();
        let pass = |input: &[[f64; 4]], horizontal: bool| -> Vec<[f64; 4]> {
            let mut output = vec![[0.0; 4]; input.len()];
            for y in 0..height {
                for x in 0..width {
                    let mut sum = [0.0; 4];
                    for (tap, weight) in kernel.iter().enumerate() {
                        let offset = tap as isize - radius;
                        let (sx, sy) = if horizontal {
                            ((x as isize + offset).clamp(0, width as isize - 1) as usize, y)
                        } else {
                            (x, (y as isize + offset).clamp(0, height as isize - 1) as usize)
                        };
                        let texel = input[sy * width + sx];
                        for (total, value) in sum.iter_mut().zip(texel) {
                            *total += value * weight;
                        }
                    }
                    output[y * width + x] = sum;
                }
            }
            output
        };
        let blurred = pass(&pass(&premultiplied, true), false);
        let mut output = RgbaImage::new(source.width(), source.height());
        for (pixel, value) in output.pixels_mut().zip(blurred) {
            let alpha = value[3];
            pixel.0 = if alpha <= 0.0 {
                [0; 4]
            } else {
                [
                    (value[0] / alpha).round().clamp(0.0, 255.0) as u8,
                    (value[1] / alpha).round().clamp(0.0, 255.0) as u8,
                    (value[2] / alpha).round().clamp(0.0, 255.0) as u8,
                    (alpha * 255.0).round().clamp(0.0, 255.0) as u8,
                ]
            };
        }
        output
    }

    fn implicit_overlay_space(state: &EditorImageState) -> EditorOverlayCoordinateSpace {
        let full = Self::full_rect();
        EditorOverlayCoordinateSpace {
            crop_rect_relative: if state.crop_baked_into_canvas {
                state.crop_rect_relative.unwrap_or(full)
            } else {
                full
            },
            rotation_steps: if state.crop_baked_into_canvas { state.rotation_steps } else { 0 },
            free_rotation_degrees: if state.crop_baked_into_canvas {
                state.free_rotation_degrees
            } else {
                0.0
            },
            is_mirrored: state.crop_baked_into_canvas && state.is_mirrored,
        }
    }

    fn full_rect() -> Bounds<f32> {
        Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size { width: 1.0, height: 1.0 },
        }
    }

    pub(crate) fn rasterize_svg_rgba(document: &str) -> Option<RgbaImage> {
        let canvas = BAKE_CANVAS.get()?;
        let rendered = canvas
            .svg_renderer
            .render_single_frame(document.as_bytes(), 1.0 / SMOOTH_SVG_SCALE_FACTOR)
            .ok()?;
        let mut bitmap = Self::bitmap(&rendered)?;
        for pixel in bitmap.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Some(bitmap)
    }

    fn bitmap(image: &RenderImage) -> Option<RgbaImage> {
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        RgbaImage::from_raw(width, height, image.as_bytes(0)?.to_vec())
    }

    fn render_image(bitmap: RgbaImage) -> Arc<RenderImage> {
        Arc::new(RenderImage::new([Frame::new(bitmap)]))
    }

    fn render_image_from_rgba(mut rgba: RgbaImage) -> Arc<RenderImage> {
        for pixel in rgba.pixels_mut() {
            pixel.0.swap(0, 2);
        }
        Self::render_image(rgba)
    }
}
