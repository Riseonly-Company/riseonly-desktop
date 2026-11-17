use std::sync::{Arc, LazyLock, Mutex};

use gpui::RenderImage;
use image::{Frame, RgbaImage};

use crate::modules::media_picker::editor::editor_state::EditorImageAdjustments;

const LUMA_WEIGHTS: [f32; 3] = [0.2125, 0.7154, 0.0721];

/// working image
/// straight alpha rgba floats in 0 to 1 and row major
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EditorWorkingImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Vec<[f32; 4]>,
}

impl EditorWorkingImage {
    pub(crate) fn from_render_image(image: &RenderImage) -> Option<Self> {
        let bytes = image.as_bytes(0)?;
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        let count = (width as usize).checked_mul(height as usize)?;
        if count == 0 || bytes.len() < count * 4 {
            return None;
        }
        let pixels = bytes
            .chunks_exact(4)
            .take(count)
            .map(|bgra| {
                [
                    f32::from(bgra[2]) / 255.0,
                    f32::from(bgra[1]) / 255.0,
                    f32::from(bgra[0]) / 255.0,
                    f32::from(bgra[3]) / 255.0,
                ]
            })
            .collect();
        Some(Self { width, height, pixels })
    }

    /// renders the working image into a gpui render image
    pub(crate) fn render_image(&self) -> Option<Arc<RenderImage>> {
        let mut bytes = Vec::with_capacity(self.pixels.len() * 4);
        for pixel in &self.pixels {
            bytes.extend_from_slice(&[
                Self::quantize(pixel[2]),
                Self::quantize(pixel[1]),
                Self::quantize(pixel[0]),
                Self::quantize(pixel[3]),
            ]);
        }
        let buffer = RgbaImage::from_raw(self.width, self.height, bytes)?;
        Some(Arc::new(RenderImage::new([Frame::new(buffer)])))
    }

    fn quantize(value: f32) -> u8 {
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    fn luma(rgb: &[f32; 4]) -> f32 {
        rgb[0] * LUMA_WEIGHTS[0] + rgb[1] * LUMA_WEIGHTS[1] + rgb[2] * LUMA_WEIGHTS[2]
    }

    fn map_rgb(&mut self, f: impl Fn(&mut [f32; 4])) {
        for pixel in &mut self.pixels {
            f(pixel);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum EditorAutoAdjustment {
    ToneCurve { low: f32, high: f32 },
    Vibrance { amount: f32 },
}

impl EditorAutoAdjustment {
    const TONE_CLIP: f32 = 0.005;
    const TONE_MIN_SPAN: f32 = 0.05;
    const VIBRANCE_TARGET: f32 = 0.25;

    fn filters(source: &EditorWorkingImage) -> Vec<Self> {
        let mut histogram = [0usize; 256];
        let mut chroma_sum = 0.0f32;
        for pixel in &source.pixels {
            let bin = (EditorWorkingImage::luma(pixel).clamp(0.0, 1.0) * 255.0).round() as usize;
            histogram[bin.min(255)] += 1;
            let max = pixel[0].max(pixel[1]).max(pixel[2]);
            let min = pixel[0].min(pixel[1]).min(pixel[2]);
            chroma_sum += max - min;
        }
        let total = source.pixels.len();
        let mut filters = Vec::new();
        if total == 0 {
            return filters;
        }
        let clip = (total as f32 * Self::TONE_CLIP) as usize;
        let percentile = |from_top: bool| -> f32 {
            let mut seen = 0usize;
            let bins: Box<dyn Iterator<Item = usize>> =
                if from_top { Box::new((0..256).rev()) } else { Box::new(0..256) };
            for bin in bins {
                seen += histogram[bin];
                if seen > clip {
                    return bin as f32 / 255.0;
                }
            }
            if from_top { 1.0 } else { 0.0 }
        };
        let low = percentile(false);
        let high = percentile(true);
        if high - low >= Self::TONE_MIN_SPAN && (low > 0.0 || high < 1.0) {
            filters.push(Self::ToneCurve { low, high });
        }
        let mean_chroma = chroma_sum / total as f32;
        if mean_chroma < Self::VIBRANCE_TARGET {
            filters.push(Self::Vibrance { amount: (Self::VIBRANCE_TARGET - mean_chroma) * 2.0 });
        }
        filters
    }

    fn output_image(self, input: &EditorWorkingImage) -> EditorWorkingImage {
        let mut out = input.clone();
        match self {
            Self::ToneCurve { low, high } => {
                let span = high - low;
                out.map_rgb(|p| {
                    for c in p.iter_mut().take(3) {
                        *c = ((*c - low) / span).clamp(0.0, 1.0);
                    }
                });
            }
            Self::Vibrance { amount } => {
                out.map_rgb(|p| {
                    let max = p[0].max(p[1]).max(p[2]);
                    let min = p[0].min(p[1]).min(p[2]);
                    let factor = 1.0 + amount * (1.0 - (max - min));
                    let l = EditorWorkingImage::luma(p);
                    for c in p.iter_mut().take(3) {
                        *c = (l + (*c - l) * factor).clamp(0.0, 1.0);
                    }
                });
            }
        }
        out
    }
}

#[derive(Default)]
struct EditorAdjustmentsRendererState {
    cached_source: Option<Arc<RenderImage>>,
    cached_source_ci: Option<Arc<EditorWorkingImage>>,
    last_adjustments: Option<EditorImageAdjustments>,
    last_result: Option<Arc<RenderImage>>,
}

#[derive(Default)]
pub(crate) struct EditorAdjustmentsRenderer {
    state_lock: Mutex<EditorAdjustmentsRendererState>,
}

static SHARED: LazyLock<EditorAdjustmentsRenderer> = LazyLock::new(EditorAdjustmentsRenderer::default);

impl EditorAdjustmentsRenderer {
    pub(crate) fn shared() -> &'static EditorAdjustmentsRenderer {
        &SHARED
    }

    pub(crate) fn render(&self, source: &Arc<RenderImage>, adjustments: &EditorImageAdjustments) -> Arc<RenderImage> {
        let mut state = self.state_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if adjustments.is_identity() {
            return source.clone();
        }
        let same_source = state.cached_source.as_ref().is_some_and(|cached| Arc::ptr_eq(cached, source));
        if same_source
            && state.last_adjustments.as_ref() == Some(adjustments)
            && let Some(last_result) = &state.last_result
        {
            return last_result.clone();
        }

        let ci_source = match (&state.cached_source_ci, same_source) {
            (Some(cached), true) => cached.clone(),
            _ => match EditorWorkingImage::from_render_image(source) {
                Some(working) => Arc::new(working),
                None => return source.clone(),
            },
        };

        let image = Self::apply(&ci_source, adjustments);

        if let Some(result) = image.render_image() {
            state.cached_source = Some(source.clone());
            state.cached_source_ci = Some(ci_source);
            state.last_adjustments = Some(*adjustments);
            state.last_result = Some(result.clone());
            return result;
        }
        source.clone()
    }

    pub(crate) fn reset_account_state(&self) {
        let mut state = self.state_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        state.cached_source = None;
        state.cached_source_ci = None;
        state.last_adjustments = None;
        state.last_result = None;
    }

    pub(crate) fn apply(source: &EditorWorkingImage, adjustments: &EditorImageAdjustments) -> EditorWorkingImage {
        let mut image = source.clone();
        if adjustments.is_identity() {
            return image;
        }
        if adjustments.enhance > 0.0 {
            for filter in EditorAutoAdjustment::filters(source) {
                let out = filter.output_image(&image);
                image = Self::blend(&image, &out, adjustments.enhance as f32);
            }
        }
        if adjustments.brightness != 0.0 || adjustments.contrast != 0.0 || adjustments.saturation != 0.0 {
            Self::color_controls(
                &mut image,
                (adjustments.brightness * 0.5) as f32,
                (1.0 + adjustments.contrast * 0.6) as f32,
                (1.0 + adjustments.saturation * 0.7) as f32,
            );
        }
        if adjustments.temperature != 0.0 {
            let target = 6500.0 - adjustments.temperature * 2200.0;
            Self::temperature_and_tint(&mut image, 6500.0, target);
        }
        if adjustments.highlights != 0.0 || adjustments.shadows != 0.0 {
            Self::highlight_shadow_adjust(
                &mut image,
                (1.0 + adjustments.highlights * 0.6) as f32,
                (adjustments.shadows * 0.6) as f32,
            );
        }
        if adjustments.vignette > 0.0 {
            Self::vignette(&mut image, (adjustments.vignette * 1.5) as f32, 1.5);
        }
        image
    }

    /// dissolve between the base and enhanced image at the given fraction
    fn blend(base: &EditorWorkingImage, enhanced: &EditorWorkingImage, fraction: f32) -> EditorWorkingImage {
        let t = fraction.clamp(0.0, 1.0);
        let mut out = base.clone();
        for (pixel, target) in out.pixels.iter_mut().zip(&enhanced.pixels) {
            for (c, e) in pixel.iter_mut().zip(target) {
                *c += (*e - *c) * t;
            }
        }
        out
    }

    /// color controls
    /// saturation around luma then brightness offset then contrast about 0.5
    fn color_controls(image: &mut EditorWorkingImage, brightness: f32, contrast: f32, saturation: f32) {
        image.map_rgb(|p| {
            let l = EditorWorkingImage::luma(p);
            for c in p.iter_mut().take(3) {
                let saturated = l + (*c - l) * saturation;
                *c = (((saturated + brightness) - 0.5) * contrast + 0.5).clamp(0.0, 1.0);
            }
        });
    }

    fn temperature_and_tint(image: &mut EditorWorkingImage, neutral: f64, target_neutral: f64) {
        let from = Self::white_point_rgb(neutral);
        let to = Self::white_point_rgb(target_neutral);
        let gains = [(to[0] / from[0]) as f32, (to[1] / from[1]) as f32, (to[2] / from[2]) as f32];
        image.map_rgb(|p| {
            for (c, gain) in p.iter_mut().zip(gains) {
                *c = (*c * gain).clamp(0.0, 1.0);
            }
        });
    }

    /// linear rgb of the planckian white at kelvin with Y = 1
    /// good for 1667 to 25000 K
    fn white_point_rgb(kelvin: f64) -> [f64; 3] {
        let t = kelvin.clamp(1667.0, 25000.0);
        let x = if t <= 4000.0 {
            -0.266_123_9e9 / t.powi(3) - 0.234_358_9e6 / t.powi(2) + 0.877_695_6e3 / t + 0.179_910
        } else {
            -3.025_846_9e9 / t.powi(3) + 2.107_037_9e6 / t.powi(2) + 0.222_634_7e3 / t + 0.240_390
        };
        let y = if t <= 2222.0 {
            -1.106_381_4 * x.powi(3) - 1.348_110_20 * x.powi(2) + 2.185_558_32 * x - 0.202_196_83
        } else if t <= 4000.0 {
            -0.954_947_6 * x.powi(3) - 1.374_185_93 * x.powi(2) + 2.091_370_15 * x - 0.167_488_67
        } else {
            3.081_758_0 * x.powi(3) - 5.873_386_70 * x.powi(2) + 3.751_129_97 * x - 0.370_014_83
        };
        let big_x = x / y;
        let big_z = (1.0 - x - y) / y;
        [
            (3.240_454_2 * big_x - 1.537_138_5 - 0.498_531_4 * big_z).max(1e-6),
            (-0.969_266_0 * big_x + 1.876_010_8 + 0.041_556_0 * big_z).max(1e-6),
            (0.055_643_4 * big_x - 0.204_025_9 + 1.057_225_2 * big_z).max(1e-6),
        ]
    }

    fn highlight_shadow_adjust(image: &mut EditorWorkingImage, highlight_amount: f32, shadow_amount: f32) {
        image.map_rgb(|p| {
            let l = EditorWorkingImage::luma(p);
            let shadow_mask = 1.0 - Self::smoothstep(0.0, 0.6, l);
            let highlight_mask = Self::smoothstep(0.4, 1.0, l);
            let lifted = l + shadow_amount * shadow_mask * (1.0 - l) * 0.5;
            let target = (lifted + (highlight_amount - 1.0) * highlight_mask * lifted * 0.5).clamp(0.0, 1.0);
            if l > 1e-4 {
                let scale = target / l;
                for c in p.iter_mut().take(3) {
                    *c = (*c * scale).clamp(0.0, 1.0);
                }
            } else {
                for c in p.iter_mut().take(3) {
                    *c = (*c + target - l).clamp(0.0, 1.0);
                }
            }
        });
    }

    /// vignette
    /// darkens toward the edges by squared distance from the center
    /// in units of radius times the half diagonal
    fn vignette(image: &mut EditorWorkingImage, intensity: f32, radius: f32) {
        let width = image.width as usize;
        let cx = image.width as f32 / 2.0;
        let cy = image.height as f32 / 2.0;
        let reach = (cx * cx + cy * cy).sqrt() * radius;
        if reach <= 0.0 || width == 0 {
            return;
        }
        for (index, pixel) in image.pixels.iter_mut().enumerate() {
            let x = (index % width) as f32 + 0.5 - cx;
            let y = (index / width) as f32 + 0.5 - cy;
            let d = (x * x + y * y).sqrt() / reach;
            let multiplier = (1.0 - intensity * d * d).clamp(0.0, 1.0);
            for c in pixel.iter_mut().take(3) {
                *c *= multiplier;
            }
        }
    }

    fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
        let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}
