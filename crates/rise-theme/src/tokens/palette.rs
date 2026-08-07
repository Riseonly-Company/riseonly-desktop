use serde::{Deserialize, Serialize};

/// Wire-compatible with the backend's `ThemeConfig.palette_json`, so a palette
/// authored on any client round-trips through theme-service unchanged.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct ThemePalette {
    pub bg_000: String,
    pub bg_100: String,
    pub bg_200: String,
    pub bg_300: String,
    pub bg_400: String,
    pub bg_500: String,
    pub bg_600: String,
    pub bg_700: String,

    pub border_100: String,
    pub border_200: String,
    pub border_300: String,
    pub border_400: String,
    pub border_500: String,
    pub border_600: String,

    pub radius_100: f32,
    pub radius_200: f32,
    pub radius_300: f32,
    pub radius_400: f32,
    pub radius_500: f32,
    pub radius_600: f32,

    pub btn_bg_000: String,
    pub btn_bg_100: String,
    pub btn_bg_200: String,
    pub btn_bg_300: String,
    pub btn_bg_400: String,
    pub btn_bg_500: String,
    pub btn_bg_600: String,

    pub btn_height_100: f32,
    pub btn_height_200: f32,
    pub btn_height_300: f32,
    pub btn_height_400: f32,
    pub btn_height_500: f32,
    pub btn_height_600: f32,

    pub btn_radius_000: f32,
    pub btn_radius_100: f32,
    pub btn_radius_200: f32,
    pub btn_radius_300: f32,

    pub primary_100: String,
    pub primary_200: String,
    pub primary_300: String,
    pub success_100: String,
    pub success_200: String,
    pub success_300: String,
    pub error_100: String,
    pub error_200: String,
    pub error_300: String,

    pub text_100: String,
    pub secondary_100: String,

    pub input_bg_100: String,
    pub input_bg_200: String,
    pub input_bg_300: String,
    pub input_border_300: String,
    pub input_height_300: f32,
    pub input_radius_300: f32,

    pub gradient_primary_start: String,
    pub gradient_primary_end: String,
}

impl ThemePalette {
    pub fn default_dark() -> Self {
        Self {
            bg_000: "#000000".into(),
            bg_100: "#020202".into(),
            bg_200: "#141414".into(),
            bg_300: "#171717".into(),
            bg_400: "#1D1D1D".into(),
            bg_500: "#232323".into(),
            bg_600: "#292929".into(),
            bg_700: "#060606".into(),
            border_100: "#252525".into(),
            border_200: "#2B2B2B".into(),
            border_300: "#313131".into(),
            border_400: "#373737".into(),
            border_500: "#3D3D3D".into(),
            border_600: "#434343".into(),
            radius_100: 20.0,
            radius_200: 15.0,
            radius_300: 10.0,
            radius_400: 30.0,
            radius_500: 40.0,
            radius_600: 50.0,
            btn_bg_000: "#000000".into(),
            btn_bg_100: "#0A0A0A".into(),
            btn_bg_200: "#141414".into(),
            btn_bg_300: "#1E1E1E".into(),
            btn_bg_400: "#282828".into(),
            btn_bg_500: "#323232".into(),
            btn_bg_600: "#3C3C3C".into(),
            btn_height_100: 55.0,
            btn_height_200: 50.0,
            btn_height_300: 45.0,
            btn_height_400: 40.0,
            btn_height_500: 35.0,
            btn_height_600: 30.0,
            btn_radius_000: 10.0,
            btn_radius_100: 20.0,
            btn_radius_200: 30.0,
            btn_radius_300: 40.0,
            primary_100: "#FF5A5A".into(),
            primary_200: "#FF4848".into(),
            primary_300: "#F04545".into(),
            success_100: "#00FF00".into(),
            success_200: "#00C800".into(),
            success_300: "#009600".into(),
            error_100: "#FF1212".into(),
            error_200: "#FF0A0A".into(),
            error_300: "#FF0000".into(),
            text_100: "#FFFFFF".into(),
            secondary_100: "#C8C8C8".into(),
            input_bg_100: "#0C0C0C".into(),
            input_bg_200: "#111111".into(),
            input_bg_300: "#161616".into(),
            input_border_300: "#242424".into(),
            input_height_300: 45.0,
            input_radius_300: 30.0,
            gradient_primary_start: "#FF5A5A".into(),
            gradient_primary_end: "#F04545".into(),
        }
    }

    pub fn default_light() -> Self {
        Self {
            bg_000: "#FFFFFF".into(),
            bg_100: "#FFFFFF".into(),
            bg_200: "#F7F7F7".into(),
            bg_300: "#F2F2F7".into(),
            bg_400: "#E5E5EA".into(),
            bg_500: "#E9F0FA".into(),
            bg_600: "#EFEFF4".into(),
            bg_700: "#F8F8F8".into(),
            border_100: "#C8C7CC".into(),
            border_200: "#C8C7CC".into(),
            border_300: "#C7C7CC".into(),
            border_400: "#B2B2B2".into(),
            border_500: "#BAB9BE".into(),
            border_600: "#D6D6DC".into(),
            btn_bg_000: "#FFFFFF".into(),
            btn_bg_100: "#FFFFFF".into(),
            btn_bg_200: "#F7F7F7".into(),
            btn_bg_300: "#F2F2F7".into(),
            btn_bg_400: "#E5E5EA".into(),
            btn_bg_500: "#E9F0FA".into(),
            btn_bg_600: "#E9E9EA".into(),
            primary_100: "#FF5A5A".into(),
            primary_200: "#FF4848".into(),
            primary_300: "#F04545".into(),
            success_100: "#26972C".into(),
            success_200: "#35C759".into(),
            success_300: "#00C900".into(),
            error_100: "#FF3B30".into(),
            error_200: "#FF3824".into(),
            error_300: "#CF3030".into(),
            text_100: "#000000".into(),
            secondary_100: "#8E8E93".into(),
            input_bg_100: "#F2F2F7".into(),
            input_bg_200: "#E9E9E9".into(),
            input_bg_300: "#FFFFFF".into(),
            input_border_300: "#C7C7CC".into(),
            ..Self::default_dark()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_and_dark_share_geometry_and_differ_in_colour() {
        let dark = ThemePalette::default_dark();
        let light = ThemePalette::default_light();

        assert_eq!(dark.radius_100, light.radius_100);
        assert_eq!(dark.btn_height_300, light.btn_height_300);
        assert_eq!(dark.input_height_300, light.input_height_300);

        assert_ne!(dark.bg_000, light.bg_000);
        assert_ne!(dark.text_100, light.text_100);
        assert_eq!(dark.primary_100, light.primary_100);
    }

    #[test]
    fn palette_round_trips_through_json() {
        let original = ThemePalette::default_dark();
        let encoded = serde_json::to_string(&original).unwrap();
        let decoded: ThemePalette = serde_json::from_str(&encoded).unwrap();
        assert_eq!(original, decoded);
    }
}
