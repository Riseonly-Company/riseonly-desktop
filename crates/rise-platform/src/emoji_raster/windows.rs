use ::windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use ::windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Factory,
};
use ::windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_METRICS, DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory,
    IDWriteFactory, IDWriteFontCollection,
};
use ::windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use ::windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapCacheOnLoad, WICRect,
};
use ::windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use ::windows::core::w;

use super::emoji_raster::*;

const MEASURE_LIMIT: f32 = 1.0e6;

const PIXELS_PER_INCH: f32 = 96.0;

pub(super) struct NativeEmojiRaster;

impl EmojiRasterSource for NativeEmojiRaster {
    fn render(&self, emoji: &str, point_size: f32) -> Result<EmojiRaster, EmojiRasterError> {
        self.render_weighted(emoji, point_size, TextWeight::Regular)
    }

    fn render_weighted(
        &self,
        text: &str,
        point_size: f32,
        weight: TextWeight,
    ) -> Result<EmojiRaster, EmojiRasterError> {
        let _apartment = ComApartment::join();
        draw(text, point_size, face_weight(weight))
    }
}

/// directwrite weight of a seam weight
/// segoe ui has a semibold face and create text format picks it from the family name and weight
fn face_weight(weight: TextWeight) -> DWRITE_FONT_WEIGHT {
    match weight {
        TextWeight::Regular => DWRITE_FONT_WEIGHT_NORMAL,
        TextWeight::Semibold => DWRITE_FONT_WEIGHT_SEMI_BOLD,
    }
}

struct ComApartment {
    joined: bool,
}

impl ComApartment {
    fn join() -> Self {
        let joined = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        Self { joined }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.joined {
            unsafe { CoUninitialize() };
        }
    }
}

fn draw(emoji: &str, point_size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<EmojiRaster, EmojiRasterError> {
    let text: Vec<u16> = emoji.encode_utf16().collect();
    unsafe {
        let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(|e| com("DWriteCreateFactory", e))?;
        let format = write
            .CreateTextFormat(
                w!("Segoe UI"),
                None::<&IDWriteFontCollection>,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                point_size,
                w!("en-us"),
            )
            .map_err(|e| com("CreateTextFormat", e))?;
        format
            .SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)
            .map_err(|e| com("SetWordWrapping", e))?;
        let layout = write
            .CreateTextLayout(&text, &format, MEASURE_LIMIT, MEASURE_LIMIT)
            .map_err(|e| com("CreateTextLayout", e))?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut metrics).map_err(|e| com("GetMetrics", e))?;
        let measured_width = f64::from(metrics.widthIncludingTrailingWhitespace);
        let measured_height = f64::from(metrics.height);
        let (width, height) = pixel_box(measured_width, measured_height).ok_or_else(|| {
            failed(format!("the measured size {measured_width}x{measured_height} gives no bitmap"))
        })?;

        let imaging: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| com("CoCreateInstance(WICImagingFactory)", e))?;
        let bitmap = imaging
            .CreateBitmap(width, height, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)
            .map_err(|e| com("CreateBitmap", e))?;
        let d2d: ID2D1Factory =
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).map_err(|e| com("D2D1CreateFactory", e))?;
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: PIXELS_PER_INCH,
            dpiY: PIXELS_PER_INCH,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let target = d2d
            .CreateWicBitmapRenderTarget(&bitmap, &properties)
            .map_err(|e| com("CreateWicBitmapRenderTarget", e))?;
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let black = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
        let brush = target
            .CreateSolidColorBrush(&black, None)
            .map_err(|e| com("CreateSolidColorBrush", e))?;
        let clear = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
        let rect = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: width as f32,
            bottom: height as f32,
        };
        target.BeginDraw();
        target.Clear(Some(&raw const clear));
        target.DrawText(
            &text,
            &format,
            &rect,
            &brush,
            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            DWRITE_MEASURING_MODE_NATURAL,
        );
        target.EndDraw(None, None).map_err(|e| com("EndDraw", e))?;

        let stride = width * 4;
        let mut rgba = vec![0u8; stride as usize * height as usize];
        let whole = WICRect {
            X: 0,
            Y: 0,
            Width: width as i32,
            Height: height as i32,
        };
        bitmap
            .CopyPixels(&whole, stride, &mut rgba)
            .map_err(|e| com("CopyPixels", e))?;
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        unpremultiply_rgba(&mut rgba);
        Ok(EmojiRaster { width, height, rgba })
    }
}

fn com(call: &str, error: ::windows::core::Error) -> EmojiRasterError {
    failed(format!("{call} failed: {error}"))
}

fn failed(reason: impl Into<String>) -> EmojiRasterError {
    EmojiRasterError::Failed { reason: reason.into() }
}
