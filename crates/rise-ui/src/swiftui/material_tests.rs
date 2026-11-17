use crate::swiftui::material::{Material, UIBlurEffectStyle};

const STYLES: [UIBlurEffectStyle; 10] = [
    UIBlurEffectStyle::SystemUltraThinMaterialLight,
    UIBlurEffectStyle::SystemThinMaterialLight,
    UIBlurEffectStyle::SystemMaterialLight,
    UIBlurEffectStyle::SystemThickMaterialLight,
    UIBlurEffectStyle::SystemChromeMaterialLight,
    UIBlurEffectStyle::SystemUltraThinMaterialDark,
    UIBlurEffectStyle::SystemThinMaterialDark,
    UIBlurEffectStyle::SystemMaterialDark,
    UIBlurEffectStyle::SystemThickMaterialDark,
    UIBlurEffectStyle::SystemChromeMaterialDark,
];

#[test]
fn thicker_materials_hide_more_of_the_backdrop() {
    let order = [Material::UltraThin, Material::Thin, Material::Regular, Material::Thick, Material::UltraThick];
    for pair in order.windows(2) {
        assert!(pair[0].painted_opacity() < pair[1].painted_opacity(), "{:?} vs {:?}", pair[0], pair[1]);
    }
}

#[test]
fn every_painted_material_stays_translucent() {
    for style in STYLES {
        let opacity = style.painted_opacity();
        assert!(opacity > 0.0 && opacity < 1.0, "{style:?}");
    }
}

#[test]
fn raw_values_follow_uikit_and_stay_distinct() {
    let raws: Vec<i64> = STYLES.iter().map(|style| style.raw_value()).collect();
    assert_eq!(raws, (11..=20).collect::<Vec<i64>>());
}
