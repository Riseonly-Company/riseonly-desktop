use gpui::{App, FontWeight, Hsla, Window};
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::rounded_gradient::{rounded_gradient_fill, rounded_gradient_stroke_border};
use rise_ui::swiftui::transition::UnitPoint;

use crate::modules::user::components::achievements::achievement_artwork::{
    AchievementArtwork, AchievementArtworkPolicy,
};
use crate::modules::user::components::achievements::achievement_rarity_chip::{
    AchievementRarityPalette, AchievementRarityStyle,
};
use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;

/// radius must hold at both card sizes and the skeleton computes the same value
/// so the formula lives in one place
pub enum AchievementSquareGeometry {}

impl AchievementSquareGeometry {
    pub fn corner_radius(side: f32) -> f32 {
        16.0_f32.min(side * 0.18)
    }
}

/// purely presentational, no button or gesture
/// card sits in a hold menu container that swallows touches
/// interactive children would never fire
#[derive(IntoElement)]
pub struct AchievementCard {
    achievement: RiseAchievement,
    side: f32,
    shows_title: bool,
    shows_badge: bool,
    is_pending: bool,
}

impl AchievementCard {
    pub const TITLE_LINES: usize = 2;

    pub fn new(achievement: RiseAchievement, side: f32) -> Self {
        Self { achievement, side, shows_title: true, shows_badge: true, is_pending: false }
    }

    pub fn shows_title(mut self, shows_title: bool) -> Self {
        self.shows_title = shows_title;
        self
    }

    pub fn shows_badge(mut self, shows_badge: bool) -> Self {
        self.shows_badge = shows_badge;
        self
    }

    pub fn is_pending(mut self, is_pending: bool) -> Self {
        self.is_pending = is_pending;
        self
    }

    fn square(&self, theme: &AppTheme) -> Div {
        let colors = self.frame_colors(theme);
        let corner_radius = self.corner_radius();

        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(self.side))
            .opacity(if self.achievement.hidden { 0.5 } else { 1.0 })
            .child(rounded_gradient_fill(Self::frame_gradient(&colors, 0.14), corner_radius).absolute().inset_0())
            .child(
                rounded_gradient_stroke_border(Self::frame_gradient(&colors, 0.9), corner_radius, 2.0)
                    .absolute()
                    .inset_0(),
            )
            .child(AchievementArtwork::new(self.achievement.clone(), self.side * 0.72).loops(true))
    }

    fn frame_colors(&self, theme: &AppTheme) -> Vec<Hsla> {
        match AchievementRarityPalette::style(self.achievement.rarity_tier()) {
            AchievementRarityStyle::Flat(Some(color)) => vec![color],
            AchievementRarityStyle::Shimmer { colors, .. } => colors,
            AchievementRarityStyle::Flat(None) => {
                vec![AchievementArtworkPolicy::accent_color(&self.achievement).unwrap_or(theme.primary._100)]
            }
        }
    }

    fn frame_gradient(colors: &[Hsla], opacity: f32) -> LinearGradient {
        LinearGradient::colors(
            colors.iter().map(|color| color.opacity(opacity)),
            UnitPoint::TOP_LEADING,
            UnitPoint::BOTTOM_TRAILING,
        )
    }

    fn corner_radius(&self) -> f32 {
        AchievementSquareGeometry::corner_radius(self.side)
    }

    fn title_size(&self) -> f32 {
        if self.side < 120.0 { 11.0 } else { 13.0 }
    }

    /// owner only wins over hidden, a hidden row already sits under a header saying so
    fn badge(&self) -> Option<String> {
        if self.achievement.owner_only {
            return Some(tr("achievements_owner_only_badge"));
        }
        if self.achievement.hidden {
            return Some(tr("achievements_hidden_badge"));
        }

        None
    }

    fn title_block(&self, theme: &AppTheme) -> Div {
        let badge = self.badge();
        let badge_visible = self.shows_badge && badge.is_some();

        vstack(2.0)
            .w(px(self.side))
            .child(
                zstack(Alignment::Top)
                    .w_full()
                    .child(
                        MainText::new(" \n ")
                            .px(self.title_size())
                            .font_weight(FontWeight::SEMIBOLD)
                            .number_of_lines(Self::TITLE_LINES)
                            .opacity(0.0),
                    )
                    .child(
                        MainText::new(self.achievement.title.clone())
                            .px(self.title_size())
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD)
                            .number_of_lines(Self::TITLE_LINES)
                            .w_full(),
                    ),
            )
            .child(
                SecondaryText::new(badge.unwrap_or_else(|| " ".to_owned()))
                    .px(11.0)
                    .tac(TextAlignT::Center)
                    .color(theme.text.secondary)
                    .line_limit(1)
                    .w_full()
                    .opacity(if badge_visible { 1.0 } else { 0.0 }),
            )
    }
}

impl RenderOnce for AchievementCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let square = self.square(theme);
        let title_block = self.shows_title.then(|| self.title_block(theme));

        vstack(8.0)
            .w(px(self.side))
            .opacity(if self.is_pending { 0.55 } else { 1.0 })
            .child(square)
            .children(title_block)
    }
}
