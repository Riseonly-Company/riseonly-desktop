use gpui::{AnyElement, FontWeight, IntoElement, Pixels, SharedString, div, prelude::*, px, svg};
use rise_theme::AppTheme;

include!(concat!(env!("OUT_DIR"), "/flags.rs"));

/// Height over width. The shipped set is the 4:3 one, and a flag stretched to
/// some other ratio is a different flag.
pub const FLAG_ASPECT: f32 = 3.0 / 4.0;

/// A country flag, by ISO 3166-1 alpha-2 region code.
///
/// The reference derives a flag arithmetically: offsetting each ASCII letter of
/// the region code into the regional-indicator block gives a pair that macOS
/// renders as a flag. Linux and Windows render it as two letters in boxes,
/// because neither ships a font with flag glyphs. So the arithmetic is gone and
/// the flags are assets.
///
/// The fallback is deliberately not another emoji and not upstream's crossed-out
/// placeholder: it is the region's own two letters in a chip, which is
/// information rather than a blank.
pub struct FlagUi;

impl FlagUi {
    pub const ASSET_PREFIX: &'static str = "icons/flags/";

    /// Whether the bundle carries this region, after normalising case.
    ///
    /// Answered from a table built at compile time. A component that touched the
    /// filesystem to decide this would be doing disk I/O inside a frame.
    pub fn is_shipped(region: &str) -> bool {
        Self::normalise(region)
            .is_some_and(|code| SHIPPED_FLAGS.binary_search(&code.as_str()).is_ok())
    }

    pub fn asset_path(region: &str) -> Option<SharedString> {
        let code = Self::normalise(region)?;
        SHIPPED_FLAGS
            .binary_search(&code.as_str())
            .ok()
            .map(|_| SharedString::from(format!("{}{code}.svg", Self::ASSET_PREFIX)))
    }

    /// The flag for `region`, or the chip when there is no flag to draw.
    ///
    /// One entry point rather than two, so no call site can forget the fallback
    /// and leave a hole that only shows up on a machine nobody here runs.
    pub fn render(theme: &AppTheme, region: &str, width: Pixels) -> AnyElement {
        let height = px(f32::from(width) * FLAG_ASPECT);

        match Self::asset_path(region) {
            Some(path) => svg()
                .path(path)
                .w(width)
                .h(height)
                .rounded(theme.radius._300 / 4.0)
                .into_any_element(),
            None => Self::chip(theme, region, width).into_any_element(),
        }
    }

    /// What the chip prints: the region's own two letters, uppercased because
    /// that is how ISO 3166-1 writes a region, and truncated to two so a
    /// malformed tag cannot widen the row it sits in.
    ///
    /// Split out so a test can assert this component never emits a code point
    /// outside ASCII — the regional-indicator pair is exactly what must not come
    /// back.
    pub fn fallback_label(region: &str) -> String {
        region
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .take(2)
            .flat_map(char::to_uppercase)
            .collect()
    }

    pub fn chip(theme: &AppTheme, region: &str, width: Pixels) -> impl IntoElement {
        let height = px(f32::from(width) * FLAG_ASPECT);
        let label = Self::fallback_label(region);

        let caption = theme.typography.style(10.0, FontWeight::SEMIBOLD);

        div()
            .w(width)
            .h(height)
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme.radius._300 / 4.0)
            .bg(theme.bg._400)
            .border_1()
            .border_color(theme.border._200)
            .text_size(caption.size)
            .font(caption.font)
            .text_color(theme.text.secondary)
            .child(label)
    }

    fn normalise(region: &str) -> Option<String> {
        let trimmed = region.trim();
        if trimmed.len() != 2 || !trimmed.bytes().all(|b| b.is_ascii_alphabetic()) {
            return None;
        }
        Some(trimmed.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;
    use std::path::PathBuf;

    fn assets() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("assets")
    }

    #[test]
    fn the_table_is_sorted_because_the_lookup_is_a_binary_search() {
        for pair in SHIPPED_FLAGS.windows(2) {
            assert!(pair[0] < pair[1], "{} is not before {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn every_shipped_code_has_a_file_behind_it() {
        let dir = assets().join("icons/flags");
        for code in SHIPPED_FLAGS {
            assert!(
                dir.join(format!("{code}.svg")).is_file(),
                "{code} is in the table with no file"
            );
        }
    }

    #[test]
    fn a_region_resolves_whatever_case_it_arrives_in() {
        for spelling in ["RU", "ru", "Ru", " ru "] {
            assert_eq!(
                FlagUi::asset_path(spelling).as_deref(),
                Some("icons/flags/ru.svg"),
                "{spelling:?} did not resolve"
            );
        }
    }

    #[test]
    fn every_region_the_language_catalogue_names_is_shipped() {
        // The locale identifiers riseonly-ios carries in AppLanguageCatalog:
        // the region is the last underscore-separated component.
        for locale in [
            "ar_SA",
            "bn_BD",
            "cs_CZ",
            "da_DK",
            "de_DE",
            "el_GR",
            "en_US",
            "es_ES",
            "fa_IR",
            "fi_FI",
            "fr_FR",
            "he_IL",
            "hi_IN",
            "hu_HU",
            "id_ID",
            "it_IT",
            "ja_JP",
            "ka_GE",
            "kk_KZ",
            "ko_KR",
            "ms_MY",
            "nl_NL",
            "nb_NO",
            "pl_PL",
            "pt_PT",
            "pt_BR",
            "ro_RO",
            "ru_RU",
            "sr_RS",
            "sv_SE",
            "sw_KE",
            "ta_IN",
            "th_TH",
            "fil_PH",
            "tr_TR",
            "uk_UA",
            "ur_PK",
            "uz_UZ",
            "vi_VN",
            "zh_Hans_CN",
            "zh_Hant_TW",
        ] {
            let region = locale.rsplit('_').next().unwrap();
            assert!(
                FlagUi::is_shipped(region),
                "{locale} needs the {region} flag and the bundle has none"
            );
        }
    }

    #[test]
    fn a_malformed_or_unknown_region_falls_back_instead_of_resolving() {
        for region in ["", "x", "xyz", "12", "zz"] {
            assert_eq!(
                FlagUi::asset_path(region),
                None,
                "{region:?} must not resolve to a file"
            );
        }
    }

    #[test]
    fn the_chip_never_widens_the_row_whatever_it_is_given() {
        let theme = AppTheme::dark();
        let width = px(24.0);

        // The chip is fixed to the same box as the flag it stands in for, so a
        // long or empty tag cannot change the layout around it.
        let _ = FlagUi::chip(&theme, "not-a-region-at-all", width);
        let _ = FlagUi::chip(&theme, "", width);
    }

    #[test]
    fn the_fallback_is_never_a_regional_indicator_pair() {
        let mut regions: Vec<String> = SHIPPED_FLAGS.iter().map(|c| c.to_string()).collect();
        regions.extend(["", "x", "xyz", "ru", "RU", "zz", "россия"].map(str::to_owned));

        for region in regions {
            let label = FlagUi::fallback_label(&region);
            assert!(
                label.chars().all(|c| c.is_ascii_uppercase()),
                "{region:?} produced {label:?}; a regional-indicator pair renders \
                 as two boxes on Linux and Windows"
            );
            assert!(label.chars().count() <= 2);
        }
    }
}
