use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{Animation, AnimationExt, App, ElementId, FontWeight, Hsla, Rgba, SharedString, Window};
use rise_catalog::AchievementRarity;
use rise_energy::rise_energy_saving::{RiseEnergySavingFeature, RiseEnergySavingStore};
use rise_i18n::tr;
use rise_platform::power::system_low_power_mode;
use rise_theme::color;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::environment::{ScenePhase, reduce_motion, scene_phase};
use rise_ui::swiftui::prelude::*;

/// a tier is flat or shimmering never both,
/// canon gives a flat tier a color and a gradient tier the inverse
#[derive(Clone, Debug, PartialEq)]
pub enum AchievementRarityStyle {
    Flat(Option<Hsla>),
    Shimmer { colors: Vec<Hsla>, seconds: f64 },
}

/// the ten tiers are fixed for the process so the table resolves once and not per render
pub enum AchievementRarityPalette {}

static RESOLVED: LazyLock<HashMap<AchievementRarity, AchievementRarityStyle>> = LazyLock::new(|| {
    AchievementRarity::ALL
        .into_iter()
        .map(|tier| (tier, AchievementRarityPalette::resolve(tier)))
        .collect()
});

impl AchievementRarityPalette {
    pub fn style(tier: AchievementRarity) -> AchievementRarityStyle {
        RESOLVED.get(&tier).cloned().unwrap_or(AchievementRarityStyle::Flat(None))
    }

    fn resolve(tier: AchievementRarity) -> AchievementRarityStyle {
        if let Some(hex) = tier.color() {
            return AchievementRarityStyle::Flat(Some(color(hex)));
        }

        let colors: Vec<Hsla> = tier.gradient().iter().map(|hex| color(hex)).collect();

        match tier.shimmer_ms() {
            Some(milliseconds) if colors.len() >= 2 && milliseconds > 0 => {
                AchievementRarityStyle::Shimmer { colors, seconds: f64::from(milliseconds) / 1_000.0 }
            }
            _ => AchievementRarityStyle::Flat(colors.first().copied()),
        }
    }
}

#[derive(IntoElement)]
pub struct AchievementRarityChip {
    tier: AchievementRarity,
    px: f32,
    font_weight: FontWeight,
}

impl AchievementRarityChip {
    pub fn new(tier: AchievementRarity) -> Self {
        Self { tier, px: 14.0, font_weight: FontWeight::SEMIBOLD }
    }

    pub fn px(mut self, px: f32) -> Self {
        self.px = px;
        self
    }

    pub fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.font_weight = font_weight;
        self
    }

    fn name(&self) -> String {
        tr(self.tier.i18n_key())
    }

    fn label(&self, color: Hsla) -> MainText {
        MainText::new(self.name()).px(self.px).font_weight(self.font_weight).color(color)
    }

    /// gradient strip seen through the glyphs
    /// gpui text has no mask so each char takes the strip color at its place
    /// canon gradients wrap seamlessly
    fn shimmer(&self, colors: Vec<Hsla>, seconds: f64, cx: &mut App) -> AnyElement {
        let name = SharedString::from(self.name());
        let (px, font_weight) = (self.px, self.font_weight);
        let label = move |progress: f32| {
            MainText::new(name.clone())
                .px(px)
                .font_weight(font_weight)
                .color_runs(sampled_color_runs(&name, &colors, progress))
        };

        if !Self::allows_decorative_motion(cx) {
            return label(0.0).into_any_element();
        }

        let id = ElementId::Name(SharedString::from(format!("achievement-rarity-chip-{}", self.tier.wire())));
        div()
            .with_animation(id, Animation::new(Duration::from_secs_f64(seconds)).repeat(), move |frame, progress| {
                frame.child(label(progress))
            })
            .into_any_element()
    }

    fn allows_decorative_motion(cx: &mut App) -> bool {
        if reduce_motion(cx)
            || system_low_power_mode()
            || RiseEnergySavingStore::shared(cx)
                .read(cx)
                .reduces(RiseEnergySavingFeature::InterfaceEffects)
            || scene_phase(cx) != ScenePhase::Active
        {
            return false;
        }
        true
    }
}

impl RenderOnce for AchievementRarityChip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        match AchievementRarityPalette::style(self.tier) {
            AchievementRarityStyle::Flat(color) => {
                let color = color.unwrap_or(rise_ui::theme(cx).text.primary);
                self.label(color).into_any_element()
            }
            AchievementRarityStyle::Shimmer { colors, seconds } => self.shimmer(colors, seconds, cx),
        }
    }
}

/// one color per character from an even strip of colors moved left by progress of the text width
pub(crate) fn sampled_color_runs(text: &str, colors: &[Hsla], progress: f32) -> Vec<(Range<usize>, Hsla)> {
    let count = text.chars().count().max(1) as f32;
    text.char_indices()
        .enumerate()
        .map(|(index, (start, character))| {
            let place = ((index as f32 + 0.5) / count + progress).fract();
            (start..start + character.len_utf8(), strip_color(colors, place))
        })
        .collect()
}

pub(crate) fn strip_color(colors: &[Hsla], place: f32) -> Hsla {
    let Some(&first) = colors.first() else {
        return gpui::transparent_black();
    };
    if colors.len() == 1 {
        return first;
    }
    let scaled = place.clamp(0.0, 1.0) * (colors.len() - 1) as f32;
    let index = (scaled.floor() as usize).min(colors.len() - 2);
    let (from, to): (Rgba, Rgba) = (colors[index].to_rgb(), colors[index + 1].to_rgb());
    let t = scaled - index as f32;
    Rgba {
        r: from.r + (to.r - from.r) * t,
        g: from.g + (to.g - from.g) * t,
        b: from.b + (to.b - from.b) * t,
        a: from.a + (to.a - from.a) * t,
    }
    .into()
}
