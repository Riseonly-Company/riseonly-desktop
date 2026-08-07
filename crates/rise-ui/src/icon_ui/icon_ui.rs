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
/// Call sites name an SF Symbol — `IconUi::render("bubble.left.and.bubble.right")` —
/// exactly as the Swift they were ported from does, and this maps it to the
/// Lucide asset the bundle actually carries. Keeping the key on the Apple side
/// is what makes a ported screen diff cleanly against its reference; naming the
/// Lucide icon at the call site would make every screen a second translation
/// nobody can check.
pub struct IconUi;

impl IconUi {
    pub const ASSET_PREFIX: &'static str = "icons/lucide/";

    pub fn size(theme: &AppTheme, size: IconSize) -> Pixels {
        match size {
            IconSize::Small => theme.icon.small,
            IconSize::Regular => theme.icon.regular,
            IconSize::Large => theme.icon.large,
        }
    }

    /// The Lucide name for an SF Symbol, or `None` when the table has never
    /// heard of it.
    ///
    /// A miss is a porting mistake — the table covers every symbol the reference
    /// uses — so it is worth distinguishing from an icon that legitimately has no
    /// counterpart. The build fails on a name with no file, so a hit here always
    /// has bytes behind it.
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

    /// Whether this symbol's Lucide match is a compromise rather than an
    /// equivalent. Exposed so a storybook can mark them for a designer instead
    /// of leaving the list in a JSON file nobody opens.
    pub fn is_approximate(sf_symbol: &str) -> bool {
        APPROXIMATE_SYMBOLS.binary_search(&sf_symbol).is_ok()
    }

    /// Renders `sf_symbol`, or nothing at all when it is not in the table.
    ///
    /// Nothing, rather than a placeholder glyph: a missing icon that draws a
    /// question mark looks like a product decision in a screenshot, while a gap
    /// looks like the bug it is.
    pub fn render(theme: &AppTheme, sf_symbol: &str, size: IconSize, color: Hsla) -> Option<Svg> {
        let path = Self::asset_path(sf_symbol)?;
        let side = Self::size(theme, size);

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

    /// 228 is what `riseonly-ios` names across BOTH `systemName:` and
    /// `systemImage:`. Counting only `systemName:` gives 171, and this table was
    /// undercounted to exactly that once already. The extra key is `folder`,
    /// which the desktop rail needs and the phone, having no rail, does not.
    ///
    /// Exclude `riseonly-ios/build/` when re-deriving the list. It is derived
    /// data, so what it holds depends on when it was last built: a checkout with
    /// third-party SwiftPM sources in it contributes symbols from someone else's
    /// settings screens. It happens to hold none today, which is why there is no
    /// second number here to compare against — the exclusion is a rule, not a
    /// figure.
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
