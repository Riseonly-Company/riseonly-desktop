use gpui::Hsla;
use rise_i18n::tr;
use rise_theme::color;

use crate::core::ui::settings_icon_palette::SettingsIconPalette;
use crate::modules::rtc::shared::rise_rtc_settings_accent::RiseRTCSettingsAccent;

#[derive(Clone, Debug, PartialEq)]
pub struct PremiumFeature {
    pub id: String,
    pub icon: String,
    pub color: Hsla,
    pub title: String,
    pub subtitle: String,
}

impl PremiumFeature {
    fn new(id: &str, icon: &str, color: Hsla, title: &str, subtitle: &str) -> Self {
        Self {
            id: id.to_owned(),
            icon: icon.to_owned(),
            color,
            title: tr(title),
            subtitle: tr(subtitle),
        }
    }
}

pub enum PremiumFeaturesConfig {}

impl PremiumFeaturesConfig {
    pub fn features(primary_color: Hsla) -> Vec<PremiumFeature> {
        vec![
            PremiumFeature::new("themes", "paintpalette.fill", SettingsIconPalette::customization(), "premium_feature_themes", "premium_feature_themes_sub"),
            PremiumFeature::new("limits", "arrow.up.circle.fill", SettingsIconPalette::stickers(), "premium_feature_stickers", "premium_feature_stickers_sub"),
            PremiumFeature::new("uploads", "icloud.and.arrow.up.fill", primary_color, "premium_feature_uploads", "premium_feature_uploads_sub"),
            PremiumFeature::new("badge", "seal.fill", color("#E9C300"), "premium_feature_badge", "premium_feature_badge_sub"),
            PremiumFeature::new("hd_calls", "video.fill", RiseRTCSettingsAccent::quality(), "premium_feature_hd_calls", "premium_feature_hd_calls_sub"),
            PremiumFeature::new("stealth", "eye.slash.fill", SettingsIconPalette::privacy(), "premium_feature_stealth", "premium_feature_stealth_sub"),
            PremiumFeature::new("packs", "face.smiling.inverse", SettingsIconPalette::stickers(), "premium_feature_packs", "premium_feature_packs_sub"),
            PremiumFeature::new("gift", "gift.fill", SettingsIconPalette::subscription(), "premium_feature_gift", "premium_feature_gift_sub"),
            PremiumFeature::new("playlists", "person.2.wave.2.fill", SettingsIconPalette::subscription(), "premium_feature_playlists", "premium_feature_playlists_sub"),
        ]
    }
}
