use gpui::{Hsla, Pixels, SharedString, Svg, prelude::*, svg};
use rise_theme::AppTheme;

include!(concat!(env!("OUT_DIR"), "/sf_to_lucide.rs"));

/// Which step of the icon ramp this glyph sits on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum IconSize {
    Small,
    #[default]
    Regular,
    Large,
}

/// An icon, addressed the way `riseonly-ios` addresses it.
///
/// Call sites name an SF Symbol — `IconUi::render("bubble.left.and.bubble.right")`
/// — and this maps it to the Lucide asset the bundle actually carries.
pub struct IconUi;

impl IconUi {
    pub const ASSET_PREFIX: &'static str = "icons/lucide/";

    /// What a filled glyph is addressed as.
    ///
    /// Lucide is stroke-only, so this path has no file on disk: the asset source
    /// synthesises it from the plain document.
    pub const FILLED_SUFFIX: &'static str = ".filled.svg";
    const PLAIN_SUFFIX: &'static str = ".svg";

    /// A Lucide document with its interior painted.
    pub fn fill_document(source: &[u8]) -> Vec<u8> {
        const UNFILLED: &str = "fill=\"none\"";
        const FILLED: &str = "fill=\"currentColor\"";

        match std::str::from_utf8(source) {
            Ok(text) if text.contains(UNFILLED) => text.replacen(UNFILLED, FILLED, 1).into_bytes(),
            // Nothing to fill: an outline is a visible compromise, a blank square is not.
            _ => source.to_vec(),
        }
    }

    /// The plain document behind a synthesised filled path, or `None` when the
    /// path is not one.
    pub fn plain_path_for_filled(path: &str) -> Option<String> {
        let stem = path.strip_suffix(Self::FILLED_SUFFIX)?;
        Some(format!("{stem}{}", Self::PLAIN_SUFFIX))
    }

    pub fn size(theme: &AppTheme, size: IconSize) -> Pixels {
        match size {
            IconSize::Small => theme.icon.small,
            IconSize::Regular => theme.icon.regular,
            IconSize::Large => theme.icon.large,
        }
    }

    /// The Lucide name for an SF Symbol, or `None` when the table has never
    /// heard of it. The build fails on a name with no file, so a hit always has
    /// bytes behind it.
    pub fn lucide_name(sf_symbol: &str) -> Option<&'static str> {
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
            SharedString::from(format!(
                "{}{lucide}{}",
                Self::ASSET_PREFIX,
                Self::FILLED_SUFFIX
            ))
        })
    }

    /// Whether this symbol's Lucide match is a compromise rather than an
    /// equivalent.
    pub fn is_approximate(sf_symbol: &str) -> bool {
        APPROXIMATE_SYMBOLS.binary_search(&sf_symbol).is_ok()
    }

    /// Renders `sf_symbol`, or nothing at all when it is not in the table.
    pub fn render(theme: &AppTheme, sf_symbol: &str, size: IconSize, color: Hsla) -> Option<Svg> {
        let path = Self::asset_path(sf_symbol)?;
        let side = Self::size(theme, size);

        Some(svg().path(path).size(side).text_color(color))
    }

    /// The same glyph, filled or not, decided by a state rather than by a name:
    /// `is_liked ? "heart.fill" : "heart"` ports as one symbol and a bool.
    pub fn toggled(
        theme: &AppTheme,
        sf_symbol: &str,
        size: IconSize,
        color: Hsla,
        is_filled: bool,
    ) -> Option<Svg> {
        let path = if is_filled {
            Self::filled_asset_path(sf_symbol)?
        } else {
            Self::asset_path(sf_symbol)?
        };

        Some(
            svg()
                .path(path)
                .size(Self::size(theme, size))
                .text_color(color),
        )
    }

    /// An icon at an explicit side length, which must already be density-scaled.
    pub fn sized(sf_symbol: &str, side: Pixels, color: Hsla, is_filled: bool) -> Option<Svg> {
        let path = if is_filled {
            Self::filled_asset_path(sf_symbol)?
        } else {
            Self::asset_path(sf_symbol)?
        };

        Some(svg().path(path).size(side).text_color(color))
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

    /// 228 = `systemName:` AND `systemImage:` (the first alone gives 171), excluding `riseonly-ios/build/`.
    #[test]
    fn the_table_covers_the_whole_reference_and_not_a_subset_of_it() {
        let used =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/sf-to-lucide.json");
        assert!(used.is_file());
        assert!(
            SF_TO_LUCIDE.len() >= 229,
            "228 reference symbols plus folder; the table has {}",
            SF_TO_LUCIDE.len()
        );
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
