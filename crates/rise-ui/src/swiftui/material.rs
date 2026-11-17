#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Material {
    UltraThin,
    Thin,
    Regular,
    Thick,
    UltraThick,
    Bar,
}

impl Material {
    /// how much of the base color the painted material keeps
    pub fn painted_opacity(self) -> f32 {
        match self {
            Self::UltraThin => 0.40,
            Self::Thin => 0.55,
            Self::Regular => 0.70,
            Self::Thick => 0.82,
            Self::UltraThick => 0.90,
            Self::Bar => 0.85,
        }
    }
}

/// blur effect styles the app asks for
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UIBlurEffectStyle {
    SystemUltraThinMaterialLight,
    SystemThinMaterialLight,
    SystemMaterialLight,
    SystemThickMaterialLight,
    SystemChromeMaterialLight,
    SystemUltraThinMaterialDark,
    SystemThinMaterialDark,
    SystemMaterialDark,
    SystemThickMaterialDark,
    SystemChromeMaterialDark,
}

impl UIBlurEffectStyle {
    /// raw value of the enum, used as the view identity of the effect view
    pub fn raw_value(self) -> i64 {
        match self {
            Self::SystemUltraThinMaterialLight => 11,
            Self::SystemThinMaterialLight => 12,
            Self::SystemMaterialLight => 13,
            Self::SystemThickMaterialLight => 14,
            Self::SystemChromeMaterialLight => 15,
            Self::SystemUltraThinMaterialDark => 16,
            Self::SystemThinMaterialDark => 17,
            Self::SystemMaterialDark => 18,
            Self::SystemThickMaterialDark => 19,
            Self::SystemChromeMaterialDark => 20,
        }
    }

    /// material of the same thickness, same painted opacity
    pub fn material(self) -> Material {
        match self {
            Self::SystemUltraThinMaterialLight | Self::SystemUltraThinMaterialDark => Material::UltraThin,
            Self::SystemThinMaterialLight | Self::SystemThinMaterialDark => Material::Thin,
            Self::SystemMaterialLight | Self::SystemMaterialDark => Material::Regular,
            Self::SystemThickMaterialLight | Self::SystemThickMaterialDark => Material::Thick,
            Self::SystemChromeMaterialLight | Self::SystemChromeMaterialDark => Material::Bar,
        }
    }

    pub fn painted_opacity(self) -> f32 {
        self.material().painted_opacity()
    }
}
