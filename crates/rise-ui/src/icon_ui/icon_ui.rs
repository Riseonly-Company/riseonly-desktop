use std::cell::RefCell;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, Hsla, ObjectFit, Pixels, RenderImage, SharedString, Svg, div, prelude::*, svg,
};
use rise_theme::AppTheme;

use crate::swiftui::alpha_shadow::{AlphaShadow, AlphaShadowModifiers};

const SILHOUETTE_SCALE: f32 = 4.0;

thread_local! {
    static LAYOUT_IS_RTL: Cell<bool> = const { Cell::new(false) };
    static SILHOUETTES: RefCell<HashMap<SharedString, Option<Arc<RenderImage>>>> = RefCell::new(HashMap::new());
}

fn silhouette(path: &SharedString, cx: &App) -> Option<Arc<RenderImage>> {
    SILHOUETTES.with(|silhouettes| {
        silhouettes
            .borrow_mut()
            .entry(path.clone())
            .or_insert_with(|| {
                let bytes = cx.asset_source().load(path.as_ref()).ok().flatten()?;
                cx.svg_renderer().render_single_frame(&bytes, SILHOUETTE_SCALE).ok()
            })
            .clone()
    })
}

include!(concat!(env!("OUT_DIR"), "/sf_to_lucide.rs"));

/// which step of the icon ramp this glyph sits on
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum IconSize {
    Small,
    #[default]
    Regular,
    Large,
}

/// what one element of a Lucide document becomes in a filled glyph
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FillRole {
    Body,
    Cut,
    Hole,
    Line,
    Slash,
    Drop,
}

const B: FillRole = FillRole::Body;
const K: FillRole = FillRole::Cut;
const H: FillRole = FillRole::Hole;
const L: FillRole = FillRole::Line;
const S: FillRole = FillRole::Slash;
const D: FillRole = FillRole::Drop;

const COMPOSITE_FILLS: &[(&str, &[FillRole])] = &[
    ("app-window", &[B, D, D, D]),
    ("archive", &[B, B, K]),
    ("badge-check", &[B, K]),
    ("badge-x", &[B, K, K]),
    ("ban", &[B, K]),
    ("battery-charging", &[K, B, L, B]),
    ("bell-off", &[B, B, S, B]),
    ("book", &[L]),
    ("bookmark-x", &[K, B, K]),
    ("briefcase", &[L, B]),
    ("bug", &[K, B, L, L, L, L, L, L, L, L, B]),
    ("building-2", &[K, K, H, B, B]),
    ("camera", &[B, K]),
    ("chart-bar", &[L, L, L, L]),
    ("circle-alert", &[B, K, K]),
    ("circle-arrow-down", &[B, K, K]),
    ("circle-arrow-up", &[B, K, K]),
    ("circle-check", &[B, K]),
    ("circle-dashed", &[L, L, L, L, L, L, L, L]),
    ("circle-dot", &[B, H]),
    ("circle-minus", &[B, K]),
    ("circle-pause", &[B, K, K]),
    ("circle-play", &[H, B]),
    ("circle-plus", &[B, K, K]),
    ("circle-question-mark", &[B, K, K]),
    ("circle-stop", &[B, H]),
    ("circle-user-round", &[H, H, B]),
    ("circle-x", &[B, K, K]),
    ("clock", &[B, K]),
    ("cloud-upload", &[K, B, K]),
    ("contact-round", &[L, H, L, H, B]),
    ("contrast", &[L, B]),
    ("credit-card", &[B, K]),
    ("delete", &[B, K, K]),
    ("disc-album", &[B, K, K]),
    ("ear-off", &[B, L, B, K, S]),
    ("earth", &[K, K, K, B]),
    ("eraser", &[B, K]),
    ("eye", &[B, K]),
    ("eye-off", &[B, K, B, S]),
    ("file", &[B, K]),
    ("file-down", &[B, K, K, K]),
    ("file-text", &[B, K, K, K, K]),
    ("film", &[B, K, K, K, K, K, K, K]),
    ("flashlight", &[K, B, K]),
    ("flashlight-off", &[K, K, B, S, B]),
    ("folder-cog", &[B, L, L, L, L, L, L, L, L, L]),
    ("folder-plus", &[K, K, B]),
    ("forward", &[B, L]),
    ("gamepad-2", &[K, K, K, K, B]),
    ("gift", &[K, B, B, B]),
    ("graduation-cap", &[B, L, B]),
    ("grid-3x3", &[B, K, K, K, K]),
    ("hard-drive", &[K, B, K, K]),
    ("house", &[H, B]),
    ("image", &[B, H, K]),
    ("images", &[K, B, H, B]),
    ("inbox", &[K, B]),
    ("info", &[B, K, K]),
    ("key-round", &[B, H]),
    ("keyboard", &[K, K, K, K, K, K, K, K, B]),
    ("lock", &[B, L]),
    ("lock-open", &[B, L]),
    ("log-out", &[L, L, L]),
    ("mail", &[K, B]),
    ("map", &[B, K, K]),
    ("map-pin", &[B, K]),
    ("megaphone", &[B, B, K]),
    ("message-square-text", &[B, K, K, K]),
    ("message-square-warning", &[B, K, K]),
    ("mic", &[L, L, B]),
    ("mic-off", &[L, B, L, L, S, B]),
    ("mic-vocal", &[B, L, B]),
    ("music", &[L, B, B]),
    ("navigation-off", &[B, B, S]),
    ("notepad-text", &[L, L, L, B, K, K, K]),
    ("octagon-x", &[K, B, K]),
    ("package", &[B, K, K, K]),
    ("palette", &[B, H, H, H, H]),
    ("phone-off", &[B, S, B]),
    ("pill", &[B, K]),
    ("pin-off", &[L, B, S, B]),
    ("reply", &[L, B]),
    ("rotate-ccw", &[L, L]),
    ("route", &[B, L, B]),
    ("send", &[B, K]),
    ("settings", &[B, H]),
    ("shield-alert", &[B, K, K]),
    ("shield-check", &[B, K]),
    ("shield-half", &[L, L]),
    ("shield-user", &[B, H, H]),
    ("smile", &[B, K, K, K]),
    ("square-arrow-out-up-right", &[B, L, L]),
    ("square-check", &[B, K]),
    ("square-play", &[B, H]),
    ("square-user-round", &[H, H, B]),
    ("switch-camera", &[B, B, K, L, L]),
    ("tag", &[B, H]),
    ("terminal", &[L, L]),
    ("trash-2", &[K, K, B, B, B]),
    ("triangle-alert", &[B, K, K]),
    ("trophy", &[B, B, L, B, B, L]),
    ("tv", &[L, B]),
    ("undo-2", &[L, L]),
    ("user-round-check", &[B, B, L]),
    ("user-round-question", &[B, B, L, L]),
    ("video-off", &[B, B, S]),
    ("volume-2", &[B, L, L]),
    ("volume-off", &[L, L, S, B, B]),
];

const DISC_GLYPHS: &[(&str, &[FillRole])] = &[
    ("at-sign", &[K, K]),
    ("audio-lines", &[K, K, K, K, K, K]),
    ("circle-arrow-out-up-right", &[D, K, K]),
    ("command", &[K]),
    ("hourglass", &[K, K, K, K]),
    ("link", &[K, K]),
    ("map-pin", &[H, D]),
    ("navigation", &[H]),
    ("reply", &[K, K]),
    ("rotate-ccw", &[K, K]),
    ("undo-2", &[K, K]),
];

const CUT_MASK_BOUNDS: &str = "x=\"-2\" y=\"-2\" width=\"28\" height=\"28\"";
const DISC_GLYPH_TRANSFORM: &str = "translate(12 12) scale(0.6) translate(-12 -12)";

fn fill_roles(table: &'static [(&'static str, &'static [FillRole])], lucide: &str) -> Option<&'static [FillRole]> {
    table
        .binary_search_by(|(name, _)| (*name).cmp(lucide))
        .ok()
        .map(|index| table[index].1)
}

/// root tag and flat element list of a Lucide document, none for any other shape
fn document_parts(text: &str) -> Option<(&str, Vec<&str>)> {
    let root = text.find("<svg")?;
    let body_start = root + text[root..].find('>')? + 1;
    let body_end = text.rfind("</svg>")?;
    let mut rest = text.get(body_start..body_end)?.trim_start();
    let mut elements = Vec::new();

    while !rest.is_empty() {
        if !rest.starts_with('<') {
            return None;
        }
        let end = rest.find("/>")? + 2;
        let element = &rest[..end];
        if element[1..].contains('<') {
            return None;
        }
        elements.push(element);
        rest = rest[end..].trim_start();
    }

    Some((&text[..body_start], elements))
}

fn compose_filled_document(root: &str, elements: &[&str], roles: &[FillRole], is_disc: bool) -> String {
    let mut cuts = String::new();
    let mut bodies = String::new();
    let mut lines = String::new();

    for (element, role) in elements.iter().zip(roles) {
        match role {
            FillRole::Body => bodies.push_str(element),
            FillRole::Cut => cuts.push_str(&format!("<g fill=\"none\">{element}</g>")),
            FillRole::Hole => cuts.push_str(&format!("<g fill=\"#000\">{element}</g>")),
            FillRole::Line => lines.push_str(element),
            FillRole::Slash => {
                cuts.push_str(&format!("<g fill=\"none\" stroke-width=\"5\">{element}</g>"));
                lines.push_str(element);
            }
            FillRole::Drop => {}
        }
    }

    let (cut_group, disc) = if is_disc {
        (
            format!("<g color=\"#000\" stroke=\"#000\" stroke-width=\"3\" transform=\"{DISC_GLYPH_TRANSFORM}\">"),
            "<circle cx=\"12\" cy=\"12\" r=\"10\"/>",
        )
    } else {
        ("<g color=\"#000\" stroke=\"#000\">".to_owned(), "")
    };

    format!(
        "{root}<defs><mask id=\"rise-cut\" maskUnits=\"userSpaceOnUse\" {CUT_MASK_BOUNDS}>\
         <rect {CUT_MASK_BOUNDS} fill=\"#fff\" stroke=\"none\"/>{cut_group}{cuts}</g></mask></defs>\
         <g mask=\"url(#rise-cut)\">{disc}{bodies}</g><g fill=\"none\">{lines}</g></svg>"
    )
}

/// an icon, call sites name an sf symbol and this maps it to the bundled Lucide asset
pub struct IconUi;

impl IconUi {
    pub fn set_layout_is_rtl(is_rtl: bool) {
        LAYOUT_IS_RTL.with(|cell| cell.set(is_rtl));
    }

    /// trailing chevron of a disclosure row, points along the reading direction
    pub fn trailing_chevron() -> &'static str {
        if LAYOUT_IS_RTL.with(Cell::get) { "chevron.left" } else { "chevron.right" }
    }

    fn directional_symbol(sf_symbol: &str) -> &str {
        if sf_symbol == "chevron.backward" && LAYOUT_IS_RTL.with(Cell::get) { "chevron.right" } else { sf_symbol }
    }

    pub const ASSET_PREFIX: &'static str = "icons/lucide/";

    pub const FILLED_SUFFIX: &'static str = ".filled.svg";
    const DISC_FILLED_SUFFIX: &'static str = ".disc.filled.svg";
    const PLAIN_SUFFIX: &'static str = ".svg";

    /// lucide document with its interior painted
    pub fn fill_document(source: &[u8]) -> Vec<u8> {
        const UNFILLED: &str = "fill=\"none\"";
        const FILLED: &str = "fill=\"currentColor\"";

        match std::str::from_utf8(source) {
            Ok(text) if text.contains(UNFILLED) => text.replacen(UNFILLED, FILLED, 1).into_bytes(),
            _ => source.to_vec(),
        }
    }

    /// the document a synthesised filled path stands for, composite symbols keep their inner figure
    pub fn filled_document(path: &str, source: &[u8]) -> Vec<u8> {
        let filled = Self::fill_document(source);
        let name = path.rsplit('/').next().unwrap_or(path);
        let (lucide, table, is_disc) = match name.strip_suffix(Self::DISC_FILLED_SUFFIX) {
            Some(lucide) => (lucide, DISC_GLYPHS, true),
            None => (
                name.strip_suffix(Self::FILLED_SUFFIX).unwrap_or(name),
                COMPOSITE_FILLS,
                false,
            ),
        };
        let Some(roles) = fill_roles(table, lucide) else {
            return filled;
        };
        let Ok(text) = std::str::from_utf8(&filled) else {
            return filled;
        };
        let composed = match document_parts(text) {
            Some((root, elements)) if elements.len() == roles.len() => {
                Some(compose_filled_document(root, &elements, roles, is_disc))
            }
            _ => None,
        };

        composed.map_or(filled, String::into_bytes)
    }

    /// true when the symbol name itself asks for the filled drawing, whatever the caller passes
    pub fn names_filled_symbol(sf_symbol: &str) -> bool {
        sf_symbol.ends_with(".fill") || sf_symbol.contains(".fill.")
    }

    /// plain document behind a synthesised filled path, none when the path is not one
    pub fn plain_path_for_filled(path: &str) -> Option<String> {
        let stem = path
            .strip_suffix(Self::DISC_FILLED_SUFFIX)
            .or_else(|| path.strip_suffix(Self::FILLED_SUFFIX))?;
        Some(format!("{stem}{}", Self::PLAIN_SUFFIX))
    }

    pub fn size(theme: &AppTheme, size: IconSize) -> Pixels {
        match size {
            IconSize::Small => theme.icon.small,
            IconSize::Regular => theme.icon.regular,
            IconSize::Large => theme.icon.large,
        }
    }

    /// lucide name for an sf symbol or none, the build fails on a name with no file
    pub fn lucide_name(sf_symbol: &str) -> Option<&'static str> {
        let sf_symbol = Self::directional_symbol(sf_symbol);
        SF_TO_LUCIDE
            .binary_search_by(|(name, _)| (*name).cmp(sf_symbol))
            .ok()
            .map(|index| SF_TO_LUCIDE[index].1)
    }

    pub fn asset_path(sf_symbol: &str) -> Option<SharedString> {
        Self::lucide_name(sf_symbol)
            .map(|lucide| SharedString::from(format!("{}{lucide}.svg", Self::ASSET_PREFIX)))
    }

    pub fn filled_asset_path(sf_symbol: &str) -> Option<SharedString> {
        Self::lucide_name(sf_symbol).map(|lucide| {
            let is_disc = sf_symbol.ends_with(".circle.fill") && fill_roles(DISC_GLYPHS, lucide).is_some();
            let suffix = if is_disc {
                Self::DISC_FILLED_SUFFIX
            } else {
                Self::FILLED_SUFFIX
            };
            SharedString::from(format!("{}{lucide}{suffix}", Self::ASSET_PREFIX))
        })
    }

    fn drawing_path(sf_symbol: &str, is_filled: bool) -> Option<SharedString> {
        if is_filled || Self::names_filled_symbol(sf_symbol) {
            Self::filled_asset_path(sf_symbol)
        } else {
            Self::asset_path(sf_symbol)
        }
    }

    pub fn is_approximate(sf_symbol: &str) -> bool {
        APPROXIMATE_SYMBOLS.binary_search(&sf_symbol).is_ok()
    }

    pub fn render(theme: &AppTheme, sf_symbol: &str, size: IconSize, color: Hsla) -> Option<Svg> {
        let path = Self::drawing_path(sf_symbol, false)?;
        let side = Self::size(theme, size);

        Some(svg().path(path).size(side).text_color(color))
    }

    /// same glyph filled or not, picked by a bool and not by a name
    pub fn toggled(
        theme: &AppTheme,
        sf_symbol: &str,
        size: IconSize,
        color: Hsla,
        is_filled: bool,
    ) -> Option<Svg> {
        let path = Self::drawing_path(sf_symbol, is_filled)?;

        Some(
            svg()
                .path(path)
                .size(Self::size(theme, size))
                .text_color(color),
        )
    }

    /// icon at an explicit side length, must already be density scaled
    pub fn sized(sf_symbol: &str, side: Pixels, color: Hsla, is_filled: bool) -> Option<Svg> {
        let path = Self::drawing_path(sf_symbol, is_filled)?;

        Some(svg().path(path).size(side).text_color(color))
    }

    pub fn sized_with_shadow(
        id: impl Into<ElementId>,
        sf_symbol: &str,
        side: Pixels,
        color: Hsla,
        is_filled: bool,
        shadow: AlphaShadow,
        cx: &App,
    ) -> Option<AnyElement> {
        let path = Self::drawing_path(sf_symbol, is_filled)?;
        let image = silhouette(&path, cx);
        let icon = svg().path(path).size(side).text_color(color);
        Some(match image {
            Some(image) => div()
                .size(side)
                .flex_none()
                .child(icon)
                .alpha_shadow_source(image, ObjectFit::Fill)
                .alpha_shadow(id, shadow)
                .into_any_element(),
            None => icon.into_any_element(),
        })
    }

    pub fn primary(theme: &AppTheme, sf_symbol: &str, size: IconSize) -> Option<Svg> {
        Self::render(theme, sf_symbol, size, theme.text.primary)
    }

    pub fn secondary(theme: &AppTheme, sf_symbol: &str, size: IconSize) -> Option<Svg> {
        Self::render(theme, sf_symbol, size, theme.text.secondary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn assets() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("assets")
    }

    #[test]
    fn the_table_is_sorted_because_the_lookup_is_a_binary_search() {
        for pair in SF_TO_LUCIDE.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "{} is not before {}",
                pair[0].0,
                pair[1].0
            );
        }
        for pair in APPROXIMATE_SYMBOLS.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        for pair in LUCIDE_ICONS.windows(2) {
            assert!(pair[0] < pair[1]);
        }
    }

    #[test]
    fn every_sf_symbol_the_reference_uses_resolves_to_an_icon() {
        let listing = assets().join("icons/lucide");

        for (sf_symbol, lucide) in SF_TO_LUCIDE {
            assert_eq!(IconUi::lucide_name(sf_symbol), Some(lucide));
            assert!(
                listing.join(format!("{lucide}.svg")).is_file(),
                "{sf_symbol} maps to {lucide}, which has no file"
            );
        }
    }

    #[test]
    fn the_asset_path_is_the_one_the_bundle_lays_out() {
        let path = IconUi::asset_path("arrow.left").expect("arrow.left is in the reference");
        assert_eq!(path.as_ref(), "icons/lucide/arrow-left.svg");
        assert!(assets().join(path.as_ref()).is_file());
    }

    #[test]
    fn a_symbol_outside_the_table_resolves_to_nothing_rather_than_to_a_wrong_glyph() {
        assert_eq!(IconUi::lucide_name("not.a.symbol"), None);
        assert_eq!(IconUi::asset_path("not.a.symbol"), None);
    }

    #[test]
    fn every_approximate_symbol_is_a_symbol_the_table_actually_maps() {
        for sf_symbol in APPROXIMATE_SYMBOLS {
            assert!(
                IconUi::lucide_name(sf_symbol).is_some(),
                "{sf_symbol} is listed as approximate but is not in the map"
            );
            assert!(IconUi::is_approximate(sf_symbol));
        }
    }

    #[test]
    fn the_table_covers_the_whole_reference_and_not_a_subset_of_it() {
        let used =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/sf-to-lucide.json");
        assert!(used.is_file());
        assert!(
            SF_TO_LUCIDE.len() >= 517,
            "516 reference symbols plus folder; the table has {}",
            SF_TO_LUCIDE.len()
        );
    }

    const RISE_SYMBOL_NAMES: [&str; 10] = [
        "chevron.up.2",
        "arrow.up",
        "chevron.down.2",
        "arrow.down",
        "headphones.slash",
        "speaker.slash",
        "music.microphone",
        "music.mic",
        "plus.circle.dashed",
        "plus.circle",
    ];

    #[test]
    fn every_glyph_rise_symbol_can_pick_has_an_icon() {
        let listing = assets().join("icons/lucide");

        for sf_symbol in RISE_SYMBOL_NAMES {
            let lucide = IconUi::lucide_name(sf_symbol)
                .unwrap_or_else(|| panic!("{sf_symbol} from RiseSymbol is not in the table"));
            assert!(
                listing.join(format!("{lucide}.svg")).is_file(),
                "{sf_symbol} maps to {lucide}, which has no file"
            );
        }
    }

    #[test]
    fn a_filled_icon_is_the_same_document_with_its_interior_painted() {
        let source = std::fs::read(assets().join("icons/lucide/heart.svg")).unwrap();
        let filled = IconUi::fill_document(&source);
        let text = String::from_utf8(filled).unwrap();

        assert!(text.contains("fill=\"currentColor\""));
        assert!(
            !text.contains("fill=\"none\""),
            "an unfilled root leaves every path unpainted, so the icon would not change"
        );
        assert!(
            text.contains("stroke=\"currentColor\""),
            "the outline stays: a filled SF Symbol keeps its silhouette"
        );
    }

    #[test]
    fn a_document_with_nothing_to_fill_is_handed_back_rather_than_mangled() {
        let untouched = b"<svg><path d=\"M0 0\"/></svg>";
        assert_eq!(IconUi::fill_document(untouched), untouched.to_vec());
        assert_eq!(IconUi::fill_document(&[0xFF, 0xFE]), vec![0xFF, 0xFE]);
    }

    #[test]
    fn a_filled_path_names_the_plain_document_it_is_synthesised_from() {
        let filled = IconUi::filled_asset_path("heart.fill").unwrap();
        assert_eq!(filled.as_ref(), "icons/lucide/heart.filled.svg");
        assert_eq!(
            IconUi::plain_path_for_filled(filled.as_ref()),
            Some("icons/lucide/heart.svg".to_owned())
        );
        assert!(assets().join("icons/lucide/heart.svg").is_file());
    }

    const ALL_BODY_FILLS: &[&str] = &["bell", "bell-dot", "crown", "hammer", "hand", "layout-grid", "lightbulb", "messages-square", "pause", "pin", "pointer", "shopping-cart", "skip-back", "skip-forward", "thumbs-down", "thumbs-up", "user-round", "user-round-plus", "users-round", "video"];

    #[test]
    fn every_fill_table_is_sorted_and_names_each_element_of_a_real_document() {
        for table in [COMPOSITE_FILLS, DISC_GLYPHS] {
            for pair in table.windows(2) {
                assert!(pair[0].0 < pair[1].0, "{} is not before {}", pair[0].0, pair[1].0);
            }
            for (lucide, roles) in table {
                let source = std::fs::read(assets().join(format!("icons/lucide/{lucide}.svg"))).unwrap();
                let text = String::from_utf8(source).unwrap();
                let (_, elements) = document_parts(&text).unwrap();
                assert_eq!(elements.len(), roles.len(), "{lucide}");
            }
        }
    }

    #[test]
    fn every_multi_element_glyph_a_fill_symbol_reaches_was_classified() {
        for (sf_symbol, lucide) in SF_TO_LUCIDE {
            if !IconUi::names_filled_symbol(sf_symbol) {
                continue;
            }
            let path = IconUi::filled_asset_path(sf_symbol).unwrap();
            if path.ends_with(IconUi::DISC_FILLED_SUFFIX) {
                continue;
            }
            let source = std::fs::read(assets().join(format!("icons/lucide/{lucide}.svg"))).unwrap();
            let text = String::from_utf8(source).unwrap();
            let (_, elements) = document_parts(&text).unwrap();
            assert!(
                elements.len() < 2
                    || fill_roles(COMPOSITE_FILLS, lucide).is_some()
                    || ALL_BODY_FILLS.contains(&lucide),
                "{sf_symbol} -> {lucide} needs a row in COMPOSITE_FILLS or ALL_BODY_FILLS"
            );
        }
    }

    #[test]
    fn a_composite_symbol_keeps_its_figure_cut_out_of_the_silhouette() {
        let path = IconUi::filled_asset_path("exclamationmark.triangle.fill").unwrap();
        let plain = IconUi::plain_path_for_filled(path.as_ref()).unwrap();
        let source = std::fs::read(assets().join(&plain)).unwrap();
        let text = String::from_utf8(IconUi::filled_document(path.as_ref(), &source)).unwrap();

        let mark = text.find("M12 9v4").expect("the exclamation mark survives");
        let silhouette = text.find("url(#rise-cut)").expect("the silhouette is masked");
        assert!(mark < silhouette, "the mark belongs to the mask, not to the paint");
        assert!(text.contains("fill=\"currentColor\""));
        assert!(text.ends_with("</svg>"));
    }

    #[test]
    fn a_strike_through_stays_visible_and_clears_a_gap() {
        let path = IconUi::filled_asset_path("bell.slash.fill").unwrap();
        let plain = IconUi::plain_path_for_filled(path.as_ref()).unwrap();
        let source = std::fs::read(assets().join(&plain)).unwrap();
        let text = String::from_utf8(IconUi::filled_document(path.as_ref(), &source)).unwrap();

        assert_eq!(text.matches("m2 2 20 20").count(), 2);
    }

    #[test]
    fn a_circle_symbol_without_an_enclosure_gets_a_disc_and_its_plain_sibling_does_not() {
        let disc = IconUi::filled_asset_path("arrowshape.turn.up.left.circle.fill").unwrap();
        assert_eq!(disc.as_ref(), "icons/lucide/reply.disc.filled.svg");
        assert_eq!(
            IconUi::plain_path_for_filled(disc.as_ref()),
            Some("icons/lucide/reply.svg".to_owned())
        );
        let source = std::fs::read(assets().join("icons/lucide/reply.svg")).unwrap();
        let text = String::from_utf8(IconUi::filled_document(disc.as_ref(), &source)).unwrap();
        assert!(text.contains("<circle cx=\"12\" cy=\"12\" r=\"10\"/>"));

        let plain = IconUi::filled_asset_path("arrowshape.turn.up.left.fill").unwrap();
        assert_eq!(plain.as_ref(), "icons/lucide/reply.filled.svg");
    }

    #[test]
    fn a_single_silhouette_is_only_painted() {
        let source = std::fs::read(assets().join("icons/lucide/heart.svg")).unwrap();
        assert_eq!(
            IconUi::filled_document("icons/lucide/heart.filled.svg", &source),
            IconUi::fill_document(&source)
        );
    }

    #[test]
    fn the_name_of_a_fill_symbol_wins_over_the_flag_of_its_caller() {
        assert_eq!(
            IconUi::drawing_path("xmark.circle.fill", false),
            IconUi::filled_asset_path("xmark.circle.fill")
        );
        assert_eq!(
            IconUi::drawing_path("person.fill.checkmark", false),
            IconUi::filled_asset_path("person.fill.checkmark")
        );
        assert_eq!(IconUi::drawing_path("heart", false), IconUi::asset_path("heart"));
        assert_eq!(IconUi::drawing_path("heart", true), IconUi::filled_asset_path("heart"));
    }

    #[test]
    fn a_plain_path_is_not_mistaken_for_a_synthesised_one() {
        assert_eq!(
            IconUi::plain_path_for_filled("icons/lucide/heart.svg"),
            None
        );
        assert_eq!(IconUi::plain_path_for_filled("fonts/Inter-Bold.ttf"), None);
    }

    #[test]
    fn the_states_a_post_and_a_comment_toggle_between_all_have_a_glyph() {
        for symbol in [
            "heart",
            "heart.fill",
            "bookmark",
            "bookmark.fill",
            "bubble.right",
            "arrow.2.squarepath",
            "hand.thumbsdown",
            "hand.thumbsdown.fill",
            "arrowshape.turn.up.left",
            "checkmark.seal.fill",
            "ellipsis",
            "eye.slash",
        ] {
            let plain = IconUi::asset_path(symbol)
                .unwrap_or_else(|| panic!("{symbol} has no Lucide mapping"));
            assert!(
                assets().join(plain.as_ref()).is_file(),
                "{symbol} maps to {plain}, which has no file"
            );

            let filled = IconUi::filled_asset_path(symbol).unwrap();
            let plain_again = IconUi::plain_path_for_filled(filled.as_ref()).unwrap();
            assert_eq!(plain_again, plain.as_ref());
        }
    }

    #[test]
    fn icon_sizes_come_from_the_theme_and_move_with_density() {
        use rise_theme::{Appearance, Density, ThemePalette};

        let normal = AppTheme::dark();
        let dense = AppTheme::new(
            &ThemePalette::default_dark(),
            Appearance::Dark,
            Density::new(1.25),
        );

        assert!(IconUi::size(&dense, IconSize::Regular) > IconUi::size(&normal, IconSize::Regular));
    }
}
