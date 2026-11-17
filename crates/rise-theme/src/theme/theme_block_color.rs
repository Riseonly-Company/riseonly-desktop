use std::sync::LazyLock;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

use super::theme_palette::ThemePalette;

struct StockBlocks {
    dark_bg_200: String,
    light_bg_200: String,
    auto_block_hexes: [String; 2],
}

static STOCK_BLOCKS: LazyLock<StockBlocks> = LazyLock::new(|| {
    let dark = ThemePalette::default_dark().bg_200;
    let light = ThemePalette::default_light().bg_200;
    StockBlocks {
        auto_block_hexes: [dark.to_uppercase(), light.to_uppercase()],
        dark_bg_200: dark,
        light_bg_200: light,
    }
});

const DARK_BLOCK_LIFT: f64 = 18.0 / 253.0;
const LIGHT_BLOCK_PRESS: f64 = 8.0 / 255.0;
const LIGHT_PAGE_LUMINANCE: f64 = 0.45;

/// the theme palette extension, the palette type lives in canon
pub trait ThemeBlockColor {
    fn block_color_is_auto(&self) -> bool;
    fn auto_block_hex(&self) -> String;
    fn block_hex(&self) -> String;
    fn level_following_block(&self, level_hex: &str, stock_dark: &str, stock_light: &str) -> Option<String>;
}

impl ThemeBlockColor for ThemePalette {
    fn block_color_is_auto(&self) -> bool {
        let value = trimming_whitespaces_and_newlines(&self.bg_200).to_uppercase();
        STOCK_BLOCKS.auto_block_hexes.contains(&value)
    }

    fn auto_block_hex(&self) -> String {
        let Some(channels) = channels(&self.bg_100) else {
            return self.bg_200.clone();
        };
        let luminance = luminance(&channels);
        let stepped = channels.map(|channel| {
            let next = if luminance >= LIGHT_PAGE_LUMINANCE {
                channel * (1.0 - LIGHT_BLOCK_PRESS)
            } else {
                channel + (255.0 - channel) * DARK_BLOCK_LIFT
            };
            next.round().clamp(0.0, 255.0) as u8
        });
        hex(stepped)
    }

    fn block_hex(&self) -> String {
        if self.block_color_is_auto() { self.auto_block_hex() } else { self.bg_200.clone() }
    }

    fn level_following_block(&self, level_hex: &str, stock_dark: &str, stock_light: &str) -> Option<String> {
        let level = channels(level_hex)?;
        let block = channels(&self.block_hex())?;
        let is_dark = luminance(&block) < LIGHT_PAGE_LUMINANCE;
        let stock_level = channels(if is_dark { stock_dark } else { stock_light })?;
        let stock_block =
            channels(if is_dark { &STOCK_BLOCKS.dark_bg_200 } else { &STOCK_BLOCKS.light_bg_200 })?;
        let is_stock_level = Some(level) == channels(stock_dark) || Some(level) == channels(stock_light);
        if !is_stock_level || block == stock_block {
            return None;
        }
        let derived: [u8; 3] = std::array::from_fn(|index| {
            let (b, s, sb) = (block[index], stock_level[index], stock_block[index]);
            let value = if is_dark { b + (255.0 - b) * ((s - sb) / (255.0 - sb).max(1.0)) } else { b * (s / sb.max(1.0)) };
            value.round().clamp(0.0, 255.0) as u8
        });
        Some(hex(derived))
    }
}

fn channels(hex: &str) -> Option<[f64; 3]> {
    let value = trimming_whitespaces_and_newlines(hex);
    let value = value.strip_prefix('#').unwrap_or(value);
    if value.chars().count() != 6 {
        return None;
    }
    let number = i64::from_str_radix(value, 16).ok()?;
    Some([((number >> 16) & 0xFF) as f64, ((number >> 8) & 0xFF) as f64, (number & 0xFF) as f64])
}

fn luminance(channels: &[f64; 3]) -> f64 {
    let linear = channels.map(|channel| {
        let value = channel / 255.0;
        if value <= 0.03928 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    });
    0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
}

fn hex(channels: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", channels[0], channels[1], channels[2])
}
