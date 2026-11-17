use std::ffi::c_void;

use objc2::encode::{Encoding, RefEncode};
use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_foundation::{NSDictionary, NSPoint, NSSize, NSString};

use super::emoji_raster::*;

/// opaque cgcontext ref, encoded as the graphics context constructor declares it
#[repr(C)]
struct CGContext {
    _private: [u8; 0],
}

unsafe impl RefEncode for CGContext {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGContext", &[]));
}

const RGBA_PREMULTIPLIED: u32 = 1;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSFontAttributeName: &'static NSString;
    static NSFontWeightSemibold: f64;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: *const c_void;
    fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        bitmap_info: u32,
    ) -> *mut CGContext;
    fn CGContextRelease(context: *mut CGContext);
}

pub(super) struct NativeEmojiRaster;

impl EmojiRasterSource for NativeEmojiRaster {
    fn render(&self, emoji: &str, point_size: f32) -> Result<EmojiRaster, EmojiRasterError> {
        autoreleasepool(|_| draw(emoji, f64::from(point_size), TextWeight::Regular))
    }

    fn render_weighted(
        &self,
        text: &str,
        point_size: f32,
        weight: TextWeight,
    ) -> Result<EmojiRaster, EmojiRasterError> {
        autoreleasepool(|_| draw(text, f64::from(point_size), weight))
    }
}

fn draw(string: &str, point_size: f64, weight: TextWeight) -> Result<EmojiRaster, EmojiRasterError> {
    let text = NSString::from_str(string);
    let font_class = AnyClass::get(c"NSFont").ok_or_else(|| failed("NSFont is not loaded"))?;
    let font: Option<Retained<AnyObject>> = unsafe {
        match weight {
            TextWeight::Regular => msg_send![font_class, systemFontOfSize: point_size],
            TextWeight::Semibold => {
                msg_send![font_class, systemFontOfSize: point_size, weight: NSFontWeightSemibold]
            }
        }
    };
    let font = font.ok_or_else(|| failed("the system font of this size is not available"))?;
    let font_key: &NSString = unsafe { NSFontAttributeName };
    let attributes = NSDictionary::<NSString, AnyObject>::from_slices(&[font_key], &[&*font]);
    let measured: NSSize = unsafe { msg_send![&*text, sizeWithAttributes: &*attributes] };
    let (width, height) = pixel_box(measured.width, measured.height).ok_or_else(|| {
        failed(format!(
            "the measured size {}x{} gives no bitmap",
            measured.width, measured.height
        ))
    })?;
    let bytes_per_row = width as usize * 4;
    let mut rgba = vec![0u8; bytes_per_row * height as usize];

    unsafe {
        let space = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        if space.is_null() {
            return Err(failed("the sRGB colour space is not available"));
        }
        let context = CGBitmapContextCreate(
            rgba.as_mut_ptr().cast(),
            width as usize,
            height as usize,
            8,
            bytes_per_row,
            space,
            RGBA_PREMULTIPLIED,
        );
        CGColorSpaceRelease(space);
        if context.is_null() {
            return Err(failed(format!("no {width}x{height} bitmap context")));
        }
        let drawn = autoreleasepool(|_| draw_into(&text, &attributes, context));
        CGContextRelease(context);
        drawn?;
    }
    unpremultiply_rgba(&mut rgba);
    Ok(EmojiRaster { width, height, rgba })
}

unsafe fn draw_into(
    text: &NSString,
    attributes: &NSDictionary<NSString, AnyObject>,
    context: *mut CGContext,
) -> Result<(), EmojiRasterError> {
    let graphics_class =
        AnyClass::get(c"NSGraphicsContext").ok_or_else(|| failed("NSGraphicsContext is not loaded"))?;
    let graphics: Option<Retained<AnyObject>> = unsafe {
        msg_send![graphics_class, graphicsContextWithCGContext: context, flipped: Bool::NO]
    };
    let graphics = graphics.ok_or_else(|| failed("no graphics context over the bitmap"))?;
    unsafe {
        let _: () = msg_send![graphics_class, saveGraphicsState];
        let _: () = msg_send![graphics_class, setCurrentContext: &*graphics];
        let _: () = msg_send![text, drawAtPoint: NSPoint::new(0.0, 0.0), withAttributes: attributes];
        let _: () = msg_send![&*graphics, flushGraphics];
        let _: () = msg_send![graphics_class, restoreGraphicsState];
    }
    Ok(())
}

fn failed(reason: impl Into<String>) -> EmojiRasterError {
    EmojiRasterError::Failed { reason: reason.into() }
}
