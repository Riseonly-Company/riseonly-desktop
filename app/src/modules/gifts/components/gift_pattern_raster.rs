use std::f64::consts::PI;
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use gpui::{AsyncApp, Hsla, Rgba, Size};
use image::RgbaImage;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapDecoder, CleverImageBitmapEncoder, CleverImageBitmapLoader,
    CleverImageLoadResult, CleverImageRenderMemoryCache, CleverImageSharedLoadRegistry, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;

pub enum GiftPatternRaster {}

#[derive(Clone, Debug)]
pub struct GiftPatternRasterRequest {
    pub symbol_url: String,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub alpha: f64,
    pub strength: f64,
    pub pixel_width: i64,
    pub pixel_height: i64,
    pub scale: f64,
    pub cache_key: String,
}

impl PartialEq for GiftPatternRasterRequest {
    fn eq(&self, other: &Self) -> bool {
        self.symbol_url == other.symbol_url
            && self.red.to_bits() == other.red.to_bits()
            && self.green.to_bits() == other.green.to_bits()
            && self.blue.to_bits() == other.blue.to_bits()
            && self.alpha.to_bits() == other.alpha.to_bits()
            && self.strength.to_bits() == other.strength.to_bits()
            && self.pixel_width == other.pixel_width
            && self.pixel_height == other.pixel_height
            && self.scale.to_bits() == other.scale.to_bits()
            && self.cache_key == other.cache_key
    }
}

impl Eq for GiftPatternRasterRequest {}

impl Hash for GiftPatternRasterRequest {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.symbol_url.hash(state);
        self.red.to_bits().hash(state);
        self.green.to_bits().hash(state);
        self.blue.to_bits().hash(state);
        self.alpha.to_bits().hash(state);
        self.strength.to_bits().hash(state);
        self.pixel_width.hash(state);
        self.pixel_height.hash(state);
        self.scale.to_bits().hash(state);
        self.cache_key.hash(state);
    }
}

impl GiftPatternRasterRequest {
    pub fn new(symbol_url: &str, color: Hsla, strength: f64, size: Size<f32>, scale: f32) -> Option<Self> {
        let pixel_width = (f64::from(size.width) * f64::from(scale)).round() as i64;
        let pixel_height = (f64::from(size.height) * f64::from(scale)).round() as i64;
        if symbol_url.is_empty() || pixel_width <= 0 || pixel_height <= 0 || scale <= 0.0 {
            return None;
        }
        let rgba = Rgba::from(color);
        let (red, green, blue, alpha) = (
            f64::from(rgba.r),
            f64::from(rgba.g),
            f64::from(rgba.b),
            f64::from(rgba.a),
        );
        let tint: Vec<String> = [red, green, blue, alpha]
            .iter()
            .map(|channel| ((channel * 1_000.0).round() as i64).to_string())
            .collect();
        let cache_key = format!(
            "gift-pattern|{symbol_url}|{}|{}|{pixel_width}x{pixel_height}",
            tint.join(","),
            (strength * 1_000.0).round() as i64
        );
        Some(Self {
            symbol_url: symbol_url.to_owned(),
            red,
            green,
            blue,
            alpha,
            strength,
            pixel_width,
            pixel_height,
            scale: f64::from(scale),
            cache_key,
        })
    }

    pub fn maximum_pixel_dimension(&self) -> i64 {
        self.pixel_width.max(self.pixel_height)
    }
}

struct Spot {
    x: f64,
    y: f64,
    size: f64,
    opacity: f64,
    angle: f64,
}

struct RingSpec {
    count: usize,
    radius: f64,
    size: f64,
    opacity: f64,
    turn: f64,
}

static SPOTS: LazyLock<Vec<Spot>> = LazyLock::new(|| {
    let rings = [
        RingSpec { count: 6, radius: 0.31, size: 0.085, opacity: 0.9, turn: 0.0 },
        RingSpec { count: 9, radius: 0.42, size: 0.072, opacity: 0.7, turn: 0.5 },
        RingSpec { count: 12, radius: 0.53, size: 0.062, opacity: 0.5, turn: 0.25 },
    ];
    let mut spots = Vec::new();
    for (ring, spec) in rings.iter().enumerate() {
        for index in 0..spec.count {
            let step = (index as f64 + spec.turn) / spec.count as f64;
            let angle = step * 2.0 * PI - PI / 2.0;
            let dx = spec.radius * angle.cos();
            let dy = spec.radius * angle.sin() * 0.96;
            let tilt = ((index * 37 + ring * 11) % 30) as f64 - 15.0;
            spots.push(Spot {
                x: 0.5 + dx,
                y: 0.46 + dy,
                size: spec.size,
                opacity: spec.opacity,
                angle: tilt,
            });
        }
    }
    spots
});

static IN_FLIGHT: LazyLock<CleverImageSharedLoadRegistry<GiftPatternRasterRequest, CleverImageLoadResult>> =
    LazyLock::new(CleverImageSharedLoadRegistry::new);

/// the symbol shape in the tint, its own alpha with the tint colour
struct TintedSymbol {
    width: usize,
    height: usize,
    alpha: Vec<f32>,
}

impl TintedSymbol {
    fn alpha_at(&self, x: i64, y: i64) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return 0.0;
        }
        self.alpha[y as usize * self.width + x as usize]
    }

    fn sample(&self, x: f64, y: f64) -> f32 {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = (fx - x0) as f32;
        let ty = (fy - y0) as f32;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = self.alpha_at(x0, y0) * (1.0 - tx) + self.alpha_at(x0 + 1, y0) * tx;
        let bottom = self.alpha_at(x0, y0 + 1) * (1.0 - tx) + self.alpha_at(x0 + 1, y0 + 1) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

impl GiftPatternRaster {
    const SYMBOL_PIXELS: i64 = 256;

    pub fn cached(request: &GiftPatternRasterRequest) -> Option<CleverImage> {
        CleverImageRenderMemoryCache::image(
            &request.cache_key,
            request.maximum_pixel_dimension(),
            SaiAccountCacheScope::active_user_id().as_deref(),
            SaiAccountCacheScope::active_epoch().as_deref(),
        )
    }

    /// the bitmap from the cache or drawn off the main thread within the image decode budget
    /// cards asking for the same one at once share one drawing
    pub async fn image(request: GiftPatternRasterRequest, cx: &AsyncApp) -> Option<CleverImage> {
        if let Some(hit) = Self::cached(&request) {
            return Some(hit);
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let operation_request = request.clone();
        let result = IN_FLIGHT
            .value(
                request,
                async move |cx: &mut AsyncApp| {
                    CleverImageLoadResult::new(
                        Self::render(operation_request, account_user_id, account_epoch, cx).await,
                    )
                },
                cx,
            )
            .await;
        result.image
    }

    async fn render(
        request: GiftPatternRasterRequest,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut AsyncApp,
    ) -> Option<CleverImage> {
        let symbol = CleverImageBitmapLoader::load(
            &request.symbol_url,
            true,
            true,
            account_user_id.clone(),
            account_epoch.clone(),
            Some(Self::SYMBOL_PIXELS),
            CleverImageCachePolicy::Standard,
            CleverImageWorkLane::Visible,
            cx,
        )
        .await?;
        let draw_request = request.clone();
        let image = CleverImageBitmapLoader::prepare_bitmap(
            move || {
                let symbol = CleverImageBitmapEncoder::rgba(&symbol)?;
                Self::draw(&symbol, &draw_request)
            },
            cx,
        )
        .await?;
        if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
            return None;
        }
        CleverImageRenderMemoryCache::store(
            image.clone(),
            &request.cache_key,
            request.maximum_pixel_dimension(),
            account_user_id.as_deref(),
            account_epoch.as_deref(),
        );
        Some(image)
    }

    /// each spot as the live layer placed it, the symbol fitted into a square of the spot share
    /// of the card shorter side
    /// turned by its tilt about the centre at ring opacity times strength and the tint alpha
    fn draw(symbol: &RgbaImage, request: &GiftPatternRasterRequest) -> Option<CleverImage> {
        let tint = Self::tinted(symbol)?;
        let pixel_width = usize::try_from(request.pixel_width).ok()?;
        let pixel_height = usize::try_from(request.pixel_height).ok()?;
        let scale = request.scale;
        let width = pixel_width as f64 / scale;
        let height = pixel_height as f64 / scale;
        let side = width.min(height);
        let symbol_width = tint.width as f64;
        let symbol_height = tint.height as f64;
        let mut coverage = vec![0.0_f32; pixel_width * pixel_height];
        for spot in SPOTS.iter() {
            let spot_box = side * spot.size;
            let fit = (spot_box / symbol_width).min(spot_box / symbol_height);
            if fit <= 0.0 {
                continue;
            }
            let drawn_width = symbol_width * fit;
            let drawn_height = symbol_height * fit;
            let center_x = width * spot.x;
            let center_y = height * spot.y;
            let radians = spot.angle * PI / 180.0;
            let (sin, cos) = radians.sin_cos();
            let opacity = (spot.opacity * request.strength * request.alpha) as f32;
            let reach = 0.5 * (drawn_width * drawn_width + drawn_height * drawn_height).sqrt();
            let left = (((center_x - reach) * scale).floor().max(0.0)) as usize;
            let top = (((center_y - reach) * scale).floor().max(0.0)) as usize;
            let right = ((((center_x + reach) * scale).ceil()) as usize).min(pixel_width);
            let bottom = ((((center_y + reach) * scale).ceil()) as usize).min(pixel_height);
            for y in top..bottom {
                for x in left..right {
                    let dx = (x as f64 + 0.5) / scale - center_x;
                    let dy = (y as f64 + 0.5) / scale - center_y;
                    let u = cos * dx + sin * dy;
                    let v = -sin * dx + cos * dy;
                    let source_x = (u + drawn_width / 2.0) / fit;
                    let source_y = (v + drawn_height / 2.0) / fit;
                    if source_x < -1.0 || source_y < -1.0 || source_x > symbol_width + 1.0 || source_y > symbol_height + 1.0 {
                        continue;
                    }
                    let alpha = tint.sample(source_x, source_y) * opacity;
                    if alpha <= 0.0 {
                        continue;
                    }
                    let destination = &mut coverage[y * pixel_width + x];
                    *destination = alpha + *destination * (1.0 - alpha);
                }
            }
        }
        let red = (request.red.clamp(0.0, 1.0) * 255.0).round() as u8;
        let green = (request.green.clamp(0.0, 1.0) * 255.0).round() as u8;
        let blue = (request.blue.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut bitmap = RgbaImage::new(u32::try_from(pixel_width).ok()?, u32::try_from(pixel_height).ok()?);
        for (index, pixel) in bitmap.pixels_mut().enumerate() {
            let alpha = (coverage[index].clamp(0.0, 1.0) * 255.0).round() as u8;
            pixel.0 = [red, green, blue, alpha];
        }
        Some(CleverImageBitmapDecoder::render_image(bitmap))
    }

    /// the symbol shape filled with the tint at full strength like a template image
    fn tinted(symbol: &RgbaImage) -> Option<TintedSymbol> {
        let width = usize::try_from(symbol.width()).ok()?;
        let height = usize::try_from(symbol.height()).ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        let alpha = symbol.pixels().map(|pixel| f32::from(pixel.0[3]) / 255.0).collect();
        Some(TintedSymbol { width, height, alpha })
    }
}
