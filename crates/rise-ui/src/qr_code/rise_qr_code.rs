use gpui::{Hsla, Rgba};
use qrcode::{EcLevel, QrCode};
use tiny_skia::{
    ColorU8, FillRule, FilterQuality, GradientStop, IntSize, LinearGradient, Mask, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point, Rect, Shader, SpreadMode, Transform,
};

/// code matrix, one boolean module per cell with the quiet zone already trimmed
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseQRCodeMatrix {
    pub size: usize,
    modules: Vec<bool>,
}

impl RiseQRCodeMatrix {
    pub fn new(size: usize, modules: Vec<bool>) -> Option<Self> {
        if size == 0 || modules.len() != size * size {
            return None;
        }
        Some(Self { size, modules })
    }

    pub fn is_dark(&self, x: i64, y: i64) -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.size || y >= self.size {
            return false;
        }
        self.modules[y * self.size + x]
    }
}

pub enum RiseQRCodeGenerator {}

impl RiseQRCodeGenerator {
    const CORRECTION_LEVEL: EcLevel = EcLevel::H;

    pub fn matrix(text: &str) -> Option<RiseQRCodeMatrix> {
        let code = QrCode::with_error_correction_level(text.as_bytes(), Self::CORRECTION_LEVEL).ok()?;
        let width = code.width();
        if width < 1 {
            return None;
        }
        let dark: Vec<bool> = code
            .to_colors()
            .into_iter()
            .map(|color| color == qrcode::Color::Dark)
            .collect();
        Self::matrix_from(width, width, &dark)
    }

    fn matrix_from(width: usize, height: usize, pixels: &[bool]) -> Option<RiseQRCodeMatrix> {
        if width == 0 || height == 0 || pixels.len() != width * height {
            return None;
        }
        let is_dark = |x: usize, y: usize| pixels[y * width + x];

        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for y in 0..height {
            for x in (0..width).filter(|&x| is_dark(x, y)) {
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((min_x, min_y, max_x, max_y)) => {
                        (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                    }
                });
            }
        }
        let (min_x, min_y, max_x, max_y) = bounds?;

        let side = (max_x - min_x + 1).max(max_y - min_y + 1);
        let mut modules = vec![false; side * side];
        for row in 0..side {
            for column in 0..side {
                let x = min_x + column;
                let y = min_y + row;
                if x > max_x || y > max_y {
                    continue;
                }
                modules[row * side + column] = is_dark(x, y);
            }
        }
        RiseQRCodeMatrix::new(side, modules)
    }
}

/// code coloring, module gradient plus the card backdrop color
#[derive(Clone, Debug, PartialEq)]
pub struct RiseQRCodeStyle {
    pub gradient: Vec<Hsla>,
    pub background: Hsla,
}

impl RiseQRCodeStyle {
    pub fn fallback() -> Self {
        Self {
            gradient: vec![
                Rgba { r: 0.11, g: 0.24, b: 0.78, a: 1.0 }.into(),
                Rgba { r: 0.38, g: 0.13, b: 0.66, a: 1.0 }.into(),
            ],
            background: gpui::white(),
        }
    }
}

/// straight (not premultiplied) rgba raster, row major top row first
/// the qr leaves the kit as pixels because the same raster is shown on screen and written to a jpeg
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseQRCodeBitmap {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub rgba: Vec<u8>,
}

impl RiseQRCodeBitmap {
    pub fn new(width: u32, height: u32, scale: u32, rgba: Vec<u8>) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if width == 0 || height == 0 || rgba.len() != expected {
            return None;
        }
        Some(Self { width, height, scale: scale.max(1), rgba })
    }

    fn from_pixmap(pixmap: &Pixmap, scale: u32) -> Option<Self> {
        let mut rgba = Vec::with_capacity(pixmap.pixels().len() * 4);
        for pixel in pixmap.pixels() {
            let color = pixel.demultiply();
            rgba.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
        }
        Self::new(pixmap.width(), pixmap.height(), scale, rgba)
    }

    fn to_pixmap(&self) -> Option<Pixmap> {
        let mut data = Vec::with_capacity(self.rgba.len());
        for chunk in self.rgba.chunks_exact(4) {
            let color = ColorU8::from_rgba(chunk[0], chunk[1], chunk[2], chunk[3]).premultiply();
            data.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
        }
        Pixmap::from_vec(data, IntSize::from_wh(self.width, self.height)?)
    }
}

pub enum RiseQRCodeRenderer {}

impl RiseQRCodeRenderer {
    const LOGO_DIAMETER_RATIO: f32 = 0.24;

    pub fn image(
        matrix: &RiseQRCodeMatrix,
        side: f32,
        scale: f32,
        style: &RiseQRCodeStyle,
        logo: Option<&RiseQRCodeBitmap>,
    ) -> Option<RiseQRCodeBitmap> {
        if side.is_nan() || side <= 0.0 || matrix.size < 21 {
            return None;
        }
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let pixels = (side * scale).ceil();
        if !(1.0..=u32::MAX as f32).contains(&pixels) {
            return None;
        }
        let mut pixmap = Pixmap::new(pixels as u32, pixels as u32)?;
        let transform = Transform::from_scale(scale, scale);

        let count = matrix.size;
        let module = side / count as f32;
        let bounds = Rect::from_xywh(0.0, 0.0, side, side)?;
        let logo_radius = side * Self::LOGO_DIAMETER_RATIO / 2.0;
        let center = (side / 2.0, side / 2.0);

        let cell = |x: usize, y: usize| Rect::from_xywh(x as f32 * module, y as f32 * module, module, module);

        let is_finder = |x: usize, y: usize| {
            let low = 0..7;
            let high = count.saturating_sub(7)..count;
            (low.contains(&x) && low.contains(&y))
                || (high.contains(&x) && low.contains(&y))
                || (low.contains(&x) && high.contains(&y))
        };

        let is_under_logo = |x: usize, y: usize| {
            if logo_radius.is_nan() || logo_radius <= 0.0 {
                return false;
            }
            let min_x = x as f32 * module - module / 2.0;
            let min_y = y as f32 * module - module / 2.0;
            let max_x = min_x + module * 2.0;
            let max_y = min_y + module * 2.0;
            let nearest_x = center.0.max(min_x).min(max_x);
            let nearest_y = center.1.max(min_y).min(max_y);
            (nearest_x - center.0).hypot(nearest_y - center.1) <= logo_radius
        };

        let paints = |x: usize, y: usize| {
            matrix.is_dark(x as i64, y as i64) && !is_finder(x, y) && !is_under_logo(x, y)
        };

        let mut dots = PathBuilder::new();
        for y in 0..count {
            for x in (0..count).filter(|&x| paints(x, y)) {
                let Some(rect) = cell(x, y) else { continue };
                dots.push_oval(rect);
                let mid_x = rect.left() + module / 2.0;
                let mid_y = rect.top() + module / 2.0;
                if x + 1 < count && paints(x + 1, y) {
                    if let Some(bridge) = Rect::from_xywh(mid_x, rect.top(), module, module) {
                        dots.push_rect(bridge);
                    }
                }
                if y + 1 < count && paints(x, y + 1) {
                    if let Some(bridge) = Rect::from_xywh(rect.left(), mid_y, module, module) {
                        dots.push_rect(bridge);
                    }
                }
            }
        }
        if let Some(path) = dots.finish() {
            Self::fill(&mut pixmap, &path, FillRule::Winding, bounds, style, transform);
        }

        for origin in [(0, 0), (count - 7, 0), (0, count - 7)] {
            let Some(frame) = Rect::from_xywh(
                origin.0 as f32 * module,
                origin.1 as f32 * module,
                module * 7.0,
                module * 7.0,
            ) else {
                continue;
            };
            let mut ring = PathBuilder::new();
            Self::push_rounded(&mut ring, frame, module * 2.2);
            if let Some(inner) = Self::inset(frame, module) {
                Self::push_rounded(&mut ring, inner, module * 1.5);
            }
            if let Some(path) = ring.finish() {
                Self::fill(&mut pixmap, &path, FillRule::EvenOdd, bounds, style, transform);
            }

            if let Some(inner) = Self::inset(frame, module * 2.0) {
                let mut pupil = PathBuilder::new();
                Self::push_rounded(&mut pupil, inner, module * 0.9);
                if let Some(path) = pupil.finish() {
                    Self::fill(&mut pixmap, &path, FillRule::Winding, bounds, style, transform);
                }
            }
        }

        if let Some(logo) = logo {
            Self::draw_logo(&mut pixmap, logo, center, logo_radius, transform);
        }

        RiseQRCodeBitmap::from_pixmap(&pixmap, scale.round().max(1.0) as u32)
    }

    fn draw_logo(
        pixmap: &mut Pixmap,
        logo: &RiseQRCodeBitmap,
        center: (f32, f32),
        logo_radius: f32,
        transform: Transform,
    ) {
        let Some(frame) = Rect::from_xywh(
            center.0 - logo_radius,
            center.1 - logo_radius,
            logo_radius * 2.0,
            logo_radius * 2.0,
        ) else {
            return;
        };
        let (Some(source), Some(clip)) = (logo.to_pixmap(), PathBuilder::from_oval(frame)) else {
            return;
        };
        let Some(mut mask) = Mask::new(pixmap.width(), pixmap.height()) else {
            return;
        };
        mask.fill_path(&clip, FillRule::Winding, true, transform);

        let placement = transform
            .pre_translate(frame.left(), frame.top())
            .pre_scale(
                frame.width() / source.width() as f32,
                frame.height() / source.height() as f32,
            );
        let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
        pixmap.draw_pixmap(0, 0, source.as_ref(), &paint, placement, Some(&mask));
    }

    fn inset(rect: Rect, by: f32) -> Option<Rect> {
        Rect::from_ltrb(rect.left() + by, rect.top() + by, rect.right() - by, rect.bottom() - by)
    }

    /// rounded rect with the radius clamped to half the side, each corner one cubic quarter ellipse
    fn push_rounded(builder: &mut PathBuilder, rect: Rect, radius: f32) {
        const KAPPA: f32 = 0.552_284_8;
        let rx = radius.min(rect.width() / 2.0).max(0.0);
        let ry = radius.min(rect.height() / 2.0).max(0.0);
        let (l, t, r, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);

        builder.move_to(l + rx, t);
        builder.line_to(r - rx, t);
        builder.cubic_to(r - rx + kx, t, r, t + ry - ky, r, t + ry);
        builder.line_to(r, b - ry);
        builder.cubic_to(r, b - ry + ky, r - rx + kx, b, r - rx, b);
        builder.line_to(l + rx, b);
        builder.cubic_to(l + rx - kx, b, l, b - ry + ky, l, b - ry);
        builder.line_to(l, t + ry);
        builder.cubic_to(l, t + ry - ky, l + rx - kx, t, l + rx, t);
        builder.close();
    }

    fn fill(
        pixmap: &mut Pixmap,
        path: &Path,
        rule: FillRule,
        bounds: Rect,
        style: &RiseQRCodeStyle,
        transform: Transform,
    ) {
        let colors = if style.gradient.is_empty() {
            RiseQRCodeStyle::fallback().gradient
        } else {
            style.gradient.clone()
        };
        let skia: Vec<tiny_skia::Color> = colors.iter().filter_map(|&color| Self::skia_color(color)).collect();
        let shader = match skia.as_slice() {
            [] => return,
            [single] => Shader::SolidColor(*single),
            many => {
                let last = (many.len() - 1) as f32;
                let stops = many
                    .iter()
                    .enumerate()
                    .map(|(index, &color)| GradientStop::new(index as f32 / last, color))
                    .collect();
                let Some(shader) = LinearGradient::new(
                    Point::from_xy(bounds.left(), bounds.top()),
                    Point::from_xy(bounds.right(), bounds.bottom()),
                    stops,
                    SpreadMode::Pad,
                    Transform::identity(),
                ) else {
                    return;
                };
                shader
            }
        };
        let paint = Paint { shader, anti_alias: true, ..Paint::default() };
        pixmap.fill_path(path, &paint, rule, transform, None);
    }

    fn skia_color(color: Hsla) -> Option<tiny_skia::Color> {
        let rgba: Rgba = color.into();
        tiny_skia::Color::from_rgba(
            rgba.r.clamp(0.0, 1.0),
            rgba.g.clamp(0.0, 1.0),
            rgba.b.clamp(0.0, 1.0),
            rgba.a.clamp(0.0, 1.0),
        )
    }
}
