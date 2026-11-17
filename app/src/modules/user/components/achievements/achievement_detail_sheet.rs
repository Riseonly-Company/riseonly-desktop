use std::rc::Rc;

use gpui::{App, FontWeight, Window};
use rise_canon::{RiseGuideAnchor, rise_key};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::app_date_formatter::AppDateFormatter;
use rise_i18n::date_format::date_formatter::Style;
use rise_i18n::{tr, tr_plural};
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::swiftui::prelude::*;

use crate::modules::user::components::achievements::achievement_artwork::AchievementArtwork;
use crate::modules::user::components::achievements::achievement_rarity_chip::AchievementRarityChip;
use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::stores::user_achievements::user_achievements_interactions::user_achievements_interactions::UserAchievementsInteractionsStore;

pub enum AchievementDateFormatting {}

impl AchievementDateFormatting {
    pub fn granted_at(milliseconds: i64) -> Option<String> {
        if milliseconds <= 0 {
            return None;
        }

        let mut formatter = AppDateFormatter::make(None, None, None);
        formatter.set_date_style(Style::Long);
        formatter.set_time_style(Style::None);
        Some(formatter.string_from(&Date::from_time_interval_since_1970(milliseconds as f64 / 1_000.0)))
    }
}

#[derive(IntoElement)]
pub struct AchievementDetailSheet {
    achievement: RiseAchievement,
    can_share: bool,
    on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl AchievementDetailSheet {
    pub fn new(achievement: RiseAchievement, can_share: bool) -> Self {
        Self { achievement, can_share, on_dismiss: Rc::new(|_: &mut Window, _: &mut App| {}) }
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(on_dismiss);
        self
    }

    fn description_text(&self) -> String {
        trimming_whitespaces_and_newlines(&self.achievement.description).to_owned()
    }

    fn rows(&self) -> Vec<GroupedBtn> {
        let mut result = vec![self.rarity_row()];

        let how_to_earn = trimming_whitespaces_and_newlines(&self.achievement.how_to_earn);
        if !how_to_earn.is_empty() {
            result.push(
                GroupedBtn::new(rise_key::user::HOW_TO_EARN, how_to_earn.to_owned())
                    .icon("target")
                    .icon_background(SettingsIconPalette::achievements())
                    .pretitle(tr("achievements_how_to_earn_title"))
                    .show_chevron(false),
            );
        }

        if let Some(granted) = AchievementDateFormatting::granted_at(self.achievement.granted_at_ms) {
            result.push(
                GroupedBtn::new("granted_at", granted)
                    .icon("calendar")
                    .icon_background(SettingsIconPalette::privacy_achievements())
                    .pretitle(tr("achievements_granted_at_title"))
                    .show_chevron(false),
            );
        }

        if self.achievement.owner_only {
            result.push(
                GroupedBtn::new(rise_key::user::OWNER_ONLY, tr("achievements_owner_only_badge"))
                    .icon("eye.slash.fill")
                    .icon_background(SettingsIconPalette::privacy())
                    .show_chevron(false),
            );
        }

        if self.can_share {
            result.push(self.share_row());
        }

        result
    }

    fn rarity_row(&self) -> GroupedBtn {
        let tier = self.achievement.rarity_tier();

        GroupedBtn::new("rarity", tr("achievement_rarity_label"))
            .icon("sparkles")
            .icon_background(SettingsIconPalette::customization())
            .accessibility_value(tr(tier.i18n_key()))
            .trailing_slot(AchievementRarityChip::new(tier))
    }

    fn share_row(&self) -> GroupedBtn {
        let on_dismiss = self.on_dismiss.clone();
        let achievement = self.achievement.clone();

        GroupedBtn::new("share", tr("achievement_share_button"))
            .icon("square.and.arrow.up")
            .icon_background(SettingsIconPalette::profile())
            .guide_anchor(RiseGuideAnchor::AchievementShare)
            .callback(move |window, cx| {
                on_dismiss(window, cx);
                UserAchievementsInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.share_achievement_to_profile(&achievement, cx));
            })
    }
}

impl RenderOnce for AchievementDetailSheet {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let text_secondary = rise_ui::theme(cx).text.secondary;
        let description_text = self.description_text();
        let rows = self.rows();

        let description = (!description_text.is_empty()).then(|| {
            div().w_full().pl(px(20.0)).pr(px(20.0)).child(
                MainText::new(description_text)
                    .px(14.0)
                    .tac(TextAlignT::Center)
                    .color(text_secondary)
                    .w_full(),
            )
        });
        let holders = (self.achievement.granted_count > 0).then(|| {
            SecondaryText::new(tr_plural("achievements_holders_count", self.achievement.granted_count))
                .px(13.0)
                .tac(TextAlignT::Center)
                .color(text_secondary)
                .w_full()
        });

        vstack(18.0)
            .w_full()
            .child(div().pt(px(8.0)).child(AchievementArtwork::new(self.achievement.clone(), 132.0)))
            .child(
                vstack(8.0)
                    .w_full()
                    .child(
                        MainText::new(self.achievement.title.clone())
                            .px(21.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::BOLD)
                            .w_full(),
                    )
                    .children(description),
            )
            .when(!rows.is_empty(), |column| {
                column.child(div().w_full().child(GroupedBtns::new("achievement-detail-rows", rows)))
            })
            .children(holders)
            .child(spacer_min_h(24.0))
    }
}
