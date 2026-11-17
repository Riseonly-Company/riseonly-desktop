use thiserror::Error;

use crate::host_os::HostOs;

pub const MAX_RASTER_SIDE: u32 = 8192;

/// a drawn emoji, straight (not premultiplied) rgba 8 bits per channel
/// rows top to bottom with no padding so len is width * height * 4
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EmojiRaster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// why no raster came back, the caller draws its picture without the emoji
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum EmojiRasterError {
    #[error("emoji rasterisation is not written for {host:?}")]
    BranchNotWritten { host: HostOs },
    #[error("emoji rasterisation failed: {reason}")]
    Failed { reason: String },
}

/// the weight of the system font, only the weights this seam is asked for
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextWeight {
    Regular,
    Semibold,
}

/// what an os layer implements
/// the default is the honest answer of a host whose branch isnt written yet
pub trait EmojiRasterSource {
    fn render(&self, emoji: &str, point_size: f32) -> Result<EmojiRaster, EmojiRasterError> {
        let _ = (emoji, point_size);
        Err(EmojiRasterError::BranchNotWritten {
            host: HostOs::current(),
        })
    }

    fn render_weighted(
        &self,
        text: &str,
        point_size: f32,
        weight: TextWeight,
    ) -> Result<EmojiRaster, EmojiRasterError> {
        match weight {
            TextWeight::Regular => self.render(text, point_size),
            TextWeight::Semibold => Err(EmojiRasterError::BranchNotWritten {
                host: HostOs::current(),
            }),
        }
    }
}

/// draws emoji with the system font of point_size through the os layer of this host
pub fn render_emoji(emoji: &str, point_size: f32) -> Result<EmojiRaster, EmojiRasterError> {
    render_with(&super::os::NativeEmojiRaster, emoji, point_size)
}

/// draws text with the system font of point_size and weight through the os layer of this host
pub fn render_text(text: &str, point_size: f32, weight: TextWeight) -> Result<EmojiRaster, EmojiRasterError> {
    render_text_with(&super::os::NativeEmojiRaster, text, point_size, weight)
}

/// render_emoji through a given source, checks the input and the shape of the answer
pub fn render_with(
    source: &dyn EmojiRasterSource,
    emoji: &str,
    point_size: f32,
) -> Result<EmojiRaster, EmojiRasterError> {
    render_text_with(source, emoji, point_size, TextWeight::Regular)
}

/// render_text through a given source, checks the input and the shape of the answer
/// an os layer without the weight still draws the string in the regular face so a letter
/// avatar keeps its letter
pub fn render_text_with(
    source: &dyn EmojiRasterSource,
    text: &str,
    point_size: f32,
    weight: TextWeight,
) -> Result<EmojiRaster, EmojiRasterError> {
    if text.is_empty() {
        return Err(failed("the string is empty"));
    }
    if !point_size.is_finite() || point_size <= 0.0 {
        return Err(failed(format!("the point size {point_size} is not a positive number")));
    }
    let raster = match source.render_weighted(text, point_size, weight) {
        Err(EmojiRasterError::BranchNotWritten { .. }) if weight != TextWeight::Regular => {
            source.render(text, point_size)?
        }
        answer => answer?,
    };
    let expected = u64::from(raster.width) * u64::from(raster.height) * 4;
    if raster.width == 0 || raster.height == 0 || raster.rgba.len() as u64 != expected {
        return Err(failed(format!(
            "the raster is {}x{} with {} bytes",
            raster.width,
            raster.height,
            raster.rgba.len()
        )));
    }
    Ok(raster)
}

/// pixel box of a measured string size, each side rounded up so the bitmap holds the whole string
/// none for a size thats not finite positive or a side over the max raster side
pub fn pixel_box(width: f64, height: f64) -> Option<(u32, u32)> {
    let side = |value: f64| -> Option<u32> {
        if !value.is_finite() || value <= 0.0 {
            return None;
        }
        let rounded = value.ceil();
        if rounded > f64::from(MAX_RASTER_SIDE) {
            return None;
        }
        Some(rounded as u32)
    };
    Some((side(width)?, side(height)?))
}

/// turns premultiplied rgba (core graphics
/// direct2d and swash composite into it) into straight rgba in place, rounded
/// a fully transparent pixel becomes all zero
pub fn unpremultiply_rgba(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        match alpha {
            0 => pixel[..3].fill(0),
            255 => {}
            _ => {
                for channel in &mut pixel[..3] {
                    let straight = (u16::from(*channel) * 255 + alpha / 2) / alpha;
                    *channel = straight.min(255) as u8;
                }
            }
        }
    }
}

fn failed(reason: impl Into<String>) -> EmojiRasterError {
    EmojiRasterError::Failed { reason: reason.into() }
}
