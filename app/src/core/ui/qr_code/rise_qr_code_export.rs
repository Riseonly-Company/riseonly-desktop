use std::borrow::Cow;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    AssetSource, AsyncApp, FontWeight, Hsla, LineFragment, Pixels, RenderImage, Rgba, SMOOTH_SVG_SCALE_FACTOR,
    SharedString, Size, SvgRenderer, TextSystem, TruncateFrom, px, size,
};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageEncoder, ImageFormat, RgbImage, RgbaImage};
use rise_core::foundation::foundation_data::base64_encoded_string;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use rise_theme::RiseThemeAppearance;
use rise_ui::qr_code::rise_qr_code::{
    RiseQRCodeBitmap, RiseQRCodeGenerator, RiseQRCodeMatrix, RiseQRCodeRenderer,
};
use rise_ui::swiftui::font::{Font, NATURAL_LINE_HEIGHT};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::qr_code::rise_qr_code_card::{
    RiseQRCodeAvatar, RiseQRCodeCardFace, RiseQRCodeCardMetrics, RiseQRCodeShareCanvas, RiseQRCodeSubject,
    RiseQRCodeTheme,
};

const APP_LOGO_ASSET: &str = "images/AppLogo.png";
const TRUNCATION_AFFIX: &str = "\u{2026}";
const CAPTION_SCALE_SEARCH_STEPS: usize = 8;

/// the code card built into a photo file
/// some apps take an image object as an image in general and rebuild it their own way
/// a jpg file goes where a camera roll shot would, so it hands out a file url and not a bitmap
pub enum RiseQRCodeExport {}

/// what the background step hands back to the main thread
struct RiseQRCodeExportPrepared {
    code: RiseQRCodeBitmap,
    code_png: Vec<u8>,
    avatar: Option<CleverImage>,
    avatar_png: Option<Vec<u8>>,
}

impl RiseQRCodeExport {
    const SCALE: f32 = 2.0;
    const JPEG_QUALITY: f32 = 0.92;
    const DIRECTORY_NAME: &'static str = "qr-share";

    /// 9:16 portrait, the aspect the card is seen at in someone elses feed
    pub fn canvas() -> Size<Pixels> {
        size(px(540.0), px(960.0))
    }

    pub async fn jpeg_file(
        subject: RiseQRCodeSubject,
        theme: RiseQRCodeTheme,
        appearance: RiseThemeAppearance,
        pattern_intensity: f64,
        matrix: Option<RiseQRCodeMatrix>,
        cx: &mut AsyncApp,
    ) -> Option<PathBuf> {
        let canvas = Self::canvas();
        let card_width = f32::from(canvas.width) * RiseQRCodeCardMetrics::CANVAS_CARD_RATIO;
        let side = RiseQRCodeCardMetrics::code_side(card_width);
        let link = subject.link.absolute_string().to_owned();
        let avatar_source = subject.avatar_url.clone();
        let style = theme.style();
        let render_scale = Self::SCALE;
        let assets = cx.update(|cx| Arc::clone(cx.asset_source()));

        let prepared = RiseAccountRuntime::current()
            .spawn_blocking(move || -> Option<RiseQRCodeExportPrepared> {
                let matrix = matrix.or_else(|| RiseQRCodeGenerator::matrix(&link))?;
                let logo = Self::app_logo(assets.as_ref());
                let code = RiseQRCodeRenderer::image(&matrix, side, render_scale, &style, logo.as_ref())?;
                let code_png = Self::png_data(RgbaImage::from_raw(code.width, code.height, code.rgba.clone())?)?;
                let avatar = avatar_source.and_then(|source| {
                    let trimmed = trimming_whitespaces_and_newlines(&source);
                    if trimmed.is_empty() {
                        return None;
                    }
                    CleverImageBitmapLoader::image_from_memory_or_disk(
                        trimmed,
                        true,
                        SaiAccountCacheScope::active_user_id().as_deref(),
                        SaiAccountCacheScope::active_epoch().as_deref(),
                        CleverImageCachePolicy::Standard,
                        None,
                    )
                });
                let avatar_png = avatar.as_ref().and_then(|image| Self::png_data(Self::rgba(image)?));
                Some(RiseQRCodeExportPrepared { code, code_png, avatar, avatar_png })
            })
            .await
            .ok()
            .flatten()?;

        let (document, wallpaper, renderer) = cx.update(|cx| {
            let share_canvas = RiseQRCodeShareCanvas::new(
                RiseQRCodeCardFace::code_image(&prepared.code),
                prepared.avatar.clone(),
                subject.clone(),
                theme,
                appearance,
                pattern_intensity,
                canvas,
            );
            let wallpaper = share_canvas.wallpaper().render_image(Self::SCALE, cx);
            let document = Self::share_canvas_document(
                &share_canvas,
                &prepared.code_png,
                prepared.avatar_png.as_deref(),
                cx.text_system(),
            );
            let renderer = SvgRenderer::new(Arc::new(RiseQRCodeExportFonts { assets: Arc::clone(cx.asset_source()) }));
            (document, wallpaper, renderer)
        });

        let name = Self::file_name(&subject);
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> Option<PathBuf> {
                let card = Self::rasterize(&renderer, &document, wallpaper.as_deref(), appearance)?;
                let data = Self::jpeg_data(&card)?;
                let manager = FileManager;
                let directory = temporary_directory().join(Self::DIRECTORY_NAME);
                manager.remove_item(&directory).ok();
                manager.create_directory(&directory, true).ok()?;
                let url = directory.join(name);
                Self::write_atomically(&url, &data).ok()?;
                Some(url)
            })
            .await
            .ok()
            .flatten()
    }

    /// recipient sees the file name so it carries the tag and not a uuid
    fn file_name(subject: &RiseQRCodeSubject) -> String {
        let source = subject.tag.as_deref().unwrap_or(&subject.title);
        let stem: String = source
            .chars()
            .filter(|&scalar| scalar.is_alphanumeric() || scalar == '_' || scalar == '-')
            .take(40)
            .collect();
        if stem.is_empty() { "Riseonly-QR.jpg".to_owned() } else { format!("Riseonly-{stem}.jpg") }
    }

    fn app_logo(assets: &dyn AssetSource) -> Option<RiseQRCodeBitmap> {
        let bytes = assets.load(APP_LOGO_ASSET).ok().flatten()?;
        let logo = image::load_from_memory_with_format(&bytes, ImageFormat::Png).ok()?.to_rgba8();
        let (width, height) = logo.dimensions();
        RiseQRCodeBitmap::new(width, height, 1, logo.into_raw())
    }

    fn share_canvas_document(
        canvas: &RiseQRCodeShareCanvas,
        code_png: &[u8],
        avatar_png: Option<&[u8]>,
        text_system: &Arc<TextSystem>,
    ) -> String {
        let width = f32::from(canvas.size.width);
        let height = f32::from(canvas.size.height);
        let card_width = canvas.card_width();
        let avatar_size = canvas.avatar_size();
        let padding = RiseQRCodeCardMetrics::padding(card_width);
        let side = RiseQRCodeCardMetrics::code_side(card_width);
        let overlap = RiseQRCodeCardMetrics::avatar_overlap(card_width, avatar_size);

        let caption_font = Self::system_font(FontWeight::BOLD);
        let (caption_size, caption_lines) = Self::caption_lines(
            text_system,
            &caption_font,
            &canvas.subject.caption(),
            RiseQRCodeCardFace::caption_font_size(card_width),
            side,
        );
        let caption_line_height = caption_size * NATURAL_LINE_HEIGHT;
        let caption_height = caption_line_height * caption_lines.len() as f32;
        let face_height = padding * 2.0 + side + RiseQRCodeCardFace::CODE_CAPTION_SPACING + caption_height;
        let group_top = (height - (overlap + face_height)) / 2.0;
        let card_left = (width - card_width) / 2.0;
        let card_top = group_top + overlap;
        let code_left = card_left + padding;
        let code_top = card_top + padding;
        let center_x = width / 2.0;

        let mut document = String::new();
        let _ = write!(
            document,
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}" viewBox="0 0 {width} {height}">"#
        );
        let radius = card_width * RiseQRCodeCardMetrics::CORNER_RADIUS_RATIO;
        let _ = write!(
            document,
            r#"<rect x="{card_left}" y="{card_top}" width="{card_width}" height="{face_height}" rx="{radius}" ry="{radius}" {}/>"#,
            Self::fill(canvas.theme.style().background)
        );
        let _ = write!(
            document,
            r#"<image x="{code_left}" y="{code_top}" width="{side}" height="{side}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/>"#,
            base64_encoded_string(code_png)
        );

        let caption_ascent = Self::ascent(text_system, &caption_font, caption_size);
        let caption_top = code_top + side + RiseQRCodeCardFace::CODE_CAPTION_SPACING;
        for (index, line) in caption_lines.iter().enumerate() {
            let baseline = caption_top + caption_line_height * index as f32 + caption_ascent;
            Self::push_text(
                &mut document,
                line,
                center_x,
                baseline,
                caption_size,
                FontWeight::BOLD,
                canvas.theme.accent(),
            );
        }

        let avatar = RiseQRCodeAvatar::new(
            canvas.avatar.clone(),
            canvas.subject.title.clone(),
            canvas.subject.identity_seed.clone(),
            avatar_size,
        );
        let avatar_radius = avatar_size / 2.0;
        let avatar_center_y = group_top + avatar_radius;
        match avatar_png {
            Some(png) => {
                let avatar_left = center_x - avatar_radius;
                let _ = write!(
                    document,
                    r#"<defs><clipPath id="avatar"><circle cx="{center_x}" cy="{avatar_center_y}" r="{avatar_radius}"/></clipPath></defs><image x="{avatar_left}" y="{group_top}" width="{avatar_size}" height="{avatar_size}" preserveAspectRatio="xMidYMid slice" clip-path="url(#avatar)" xlink:href="data:image/png;base64,{}"/>"#,
                    base64_encoded_string(png)
                );
            }
            None => {
                let _ = write!(
                    document,
                    r#"<circle cx="{center_x}" cy="{avatar_center_y}" r="{avatar_radius}" {}/>"#,
                    Self::fill(avatar.fill())
                );
                let letter_font = Self::system_font(FontWeight::SEMIBOLD);
                let letter_size = RiseQRCodeAvatar::letter_font_size(avatar_size);
                let letter_line_height = letter_size * NATURAL_LINE_HEIGHT;
                let baseline = avatar_center_y - letter_line_height / 2.0
                    + Self::ascent(text_system, &letter_font, letter_size);
                Self::push_text(
                    &mut document,
                    &avatar.letter(),
                    center_x,
                    baseline,
                    letter_size,
                    FontWeight::SEMIBOLD,
                    rise_theme::AvatarPalette::letter_color_on(avatar.fill()),
                );
            }
        }
        document.push_str("</svg>");
        document
    }

    fn system_font(weight: FontWeight) -> gpui::Font {
        let spec = Font::system_weight(0.0, weight);
        gpui::Font { family: spec.family(), features: spec.features(), fallbacks: None, weight, style: spec.style() }
    }

    fn ascent(text_system: &Arc<TextSystem>, font: &gpui::Font, font_size: f32) -> f32 {
        let font_id = text_system.resolve_font(font);
        f32::from(text_system.ascent(font_id, px(font_size)))
    }

    /// font shrinks until the caption fits two lines of the code width
    /// what still doesnt fit at half size is cut with an ellipsis
    fn caption_lines(
        text_system: &Arc<TextSystem>,
        font: &gpui::Font,
        caption: &str,
        font_size: f32,
        width: f32,
    ) -> (f32, Vec<String>) {
        let limit = RiseQRCodeCardFace::CAPTION_LINE_LIMIT;
        let lines_at = |scale: f32| Self::wrapped_lines(text_system, font, caption, font_size * scale, width);

        let full = lines_at(1.0);
        if full.len() <= limit {
            return (font_size, full);
        }
        let minimum = RiseQRCodeCardFace::CAPTION_MINIMUM_SCALE_FACTOR;
        if lines_at(minimum).len() > limit {
            let smallest = font_size * minimum;
            let mut wrapper = text_system.line_wrapper(font.clone(), px(smallest));
            let (truncated, _) = wrapper.truncate_wrapped_line(
                SharedString::from(caption.to_owned()),
                px(width),
                limit,
                TRUNCATION_AFFIX,
                &[],
                TruncateFrom::End,
            );
            drop(wrapper);
            let mut lines = Self::wrapped_lines(text_system, font, &truncated, smallest, width);
            lines.truncate(limit);
            return (smallest, lines);
        }
        let (mut fitting, mut overflowing) = (minimum, 1.0_f32);
        for _ in 0..CAPTION_SCALE_SEARCH_STEPS {
            let middle = (fitting + overflowing) / 2.0;
            if lines_at(middle).len() <= limit {
                fitting = middle;
            } else {
                overflowing = middle;
            }
        }
        (font_size * fitting, lines_at(fitting))
    }

    fn wrapped_lines(
        text_system: &Arc<TextSystem>,
        font: &gpui::Font,
        text: &str,
        font_size: f32,
        width: f32,
    ) -> Vec<String> {
        let mut wrapper = text_system.line_wrapper(font.clone(), px(font_size));
        let fragments = [LineFragment::text(text)];
        let boundaries: Vec<usize> = wrapper.wrap_line(&fragments, px(width)).map(|boundary| boundary.ix).collect();
        let mut lines = Vec::with_capacity(boundaries.len() + 1);
        let mut start = 0;
        for end in boundaries.into_iter().chain([text.len()]) {
            if let Some(line) = text.get(start..end) {
                lines.push(trimming_whitespaces_and_newlines(line).to_owned());
            }
            start = end;
        }
        lines
    }

    fn push_text(
        document: &mut String,
        text: &str,
        center_x: f32,
        baseline: f32,
        font_size: f32,
        weight: FontWeight,
        color: Hsla,
    ) {
        let family = Font::system_weight(font_size, weight).family();
        let _ = write!(
            document,
            r#"<text x="{center_x}" y="{baseline}" text-anchor="middle" font-family="{}" font-weight="{}" font-size="{font_size}" {}>{}</text>"#,
            Self::escaped(&family),
            weight.0,
            Self::fill(color),
            Self::escaped(text)
        );
    }

    fn fill(color: Hsla) -> String {
        let rgba: Rgba = color.into();
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!(
            r##"fill="#{:02X}{:02X}{:02X}" fill-opacity="{}""##,
            channel(rgba.r),
            channel(rgba.g),
            channel(rgba.b),
            rgba.a.clamp(0.0, 1.0)
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

    fn rasterize(
        renderer: &SvgRenderer,
        document: &str,
        wallpaper: Option<&RenderImage>,
        appearance: RiseThemeAppearance,
    ) -> Option<RgbImage> {
        let layer = renderer
            .render_single_frame(document.as_bytes(), Self::SCALE / SMOOTH_SVG_SCALE_FACTOR)
            .ok()?;
        let layer_size = layer.size(0);
        let width = u32::try_from(layer_size.width.0).ok()?;
        let height = u32::try_from(layer_size.height.0).ok()?;
        let layer_bytes = layer.as_bytes(0)?;

        let backdrop = wallpaper.and_then(Self::rgba).map(|backdrop| {
            if backdrop.dimensions() == (width, height) {
                backdrop
            } else {
                image::imageops::resize(&backdrop, width, height, FilterType::Triangle)
            }
        });
        let base = if appearance == RiseThemeAppearance::Light { [255u8; 3] } else { [0u8; 3] };

        let mut card = RgbImage::new(width, height);
        for (index, (pixel, source)) in card.pixels_mut().zip(layer_bytes.chunks_exact(4)).enumerate() {
            let under = backdrop
                .as_ref()
                .and_then(|backdrop| backdrop.as_raw().get(index * 4..index * 4 + 3))
                .map_or(base, |rgb| [rgb[0], rgb[1], rgb[2]]);
            let alpha = f32::from(source[3]) / 255.0;
            let over = [source[2], source[1], source[0]];
            pixel.0 = std::array::from_fn(|channel| {
                let blended = f32::from(over[channel]) * alpha + f32::from(under[channel]) * (1.0 - alpha);
                blended.round().clamp(0.0, 255.0) as u8
            });
        }
        Some(card)
    }

    fn jpeg_data(card: &RgbImage) -> Option<Vec<u8>> {
        let quality = (Self::JPEG_QUALITY * 100.0).round() as u8;
        let mut data = Vec::new();
        JpegEncoder::new_with_quality(&mut data, quality).encode_image(card).ok()?;
        Some(data)
    }

    fn png_data(rgba: RgbaImage) -> Option<Vec<u8>> {
        let mut data = Vec::new();
        PngEncoder::new(&mut data)
            .write_image(rgba.as_raw(), rgba.width(), rgba.height(), ExtendedColorType::Rgba8)
            .ok()?;
        Some(data)
    }

    /// gpui images hold bgra and the encoders take rgba
    fn rgba(image: &RenderImage) -> Option<RgbaImage> {
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        let mut bytes = image.as_bytes(0)?.to_vec();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        RgbaImage::from_raw(width, height, bytes)
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = url.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, url).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }
}

struct RiseQRCodeExportFonts {
    assets: Arc<dyn AssetSource>,
}

impl RiseQRCodeExportFonts {
    const SANS_SLOT: &'static str = "fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf";
    const MONO_SLOT: &'static str = "fonts/lilex/Lilex-Regular.ttf";
    const CAPTION_FACE: &'static str = "fonts/Inter-Bold.ttf";
    const LETTER_FACE: &'static str = "fonts/Inter-SemiBold.ttf";
}

impl AssetSource for RiseQRCodeExportFonts {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let source = match path {
            Self::SANS_SLOT => Self::CAPTION_FACE,
            Self::MONO_SLOT => Self::LETTER_FACE,
            other => other,
        };
        self.assets.load(source)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        self.assets.list(path)
    }
}
