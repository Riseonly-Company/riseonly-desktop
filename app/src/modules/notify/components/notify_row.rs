use gpui::{App, ElementId, FontWeight, HighlightStyle, RenderOnce, SharedString, StyledText, Window};
use rise_theme::AppTheme;
use rise_ui::swiftui::prelude::*;

use crate::modules::notify::components::notification_avatar_stack::NotificationAvatarStack;
use crate::modules::notify::engine::rise_notifications_presentation::RiseNotificationRow;
use crate::modules::notify::stores::notify_actions::notify_types::NotificationKind;
use crate::modules::user::components::achievements::achievement_card::AchievementCard;
use crate::modules::user::stores::user_achievements::user_achievements_services::user_achievements_services::UserAchievementsServicesStore;

const FONT_SIZE: f32 = 14.5;
const AVATAR_SIDE: f32 = 46.0;

/// one feed row, strings arrive prepared
/// the row only lays out runs and applies the theme
#[derive(IntoElement, PartialEq)]
pub struct NotifyRow {
    row: RiseNotificationRow,
}

impl NotifyRow {
    pub fn new(row: RiseNotificationRow) -> Self {
        Self { row }
    }

    fn text(row: &RiseNotificationRow, theme: &AppTheme) -> StyledText {
        let mut result = String::new();
        let mut highlights = Vec::new();
        if !row.leading.is_empty() {
            let weight = if row.leading_is_heading { FontWeight::BOLD } else { FontWeight::SEMIBOLD };
            highlights.push((
                0..row.leading.len(),
                HighlightStyle { font_weight: Some(weight), ..Default::default() },
            ));
            result.push_str(&row.leading);
            if !row.text.is_empty() {
                result.push(' ');
            }
        }
        result.push_str(&row.text);
        if !row.time.is_empty() {
            let start = result.len();
            result.push_str("   ");
            result.push_str(&row.time);
            highlights.push((
                start..result.len(),
                HighlightStyle { color: Some(theme.text.secondary), ..Default::default() },
            ));
        }
        StyledText::new(SharedString::from(result)).with_highlights(highlights)
    }
}

impl RenderOnce for NotifyRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let row = self.row;
        let avatar_id = ElementId::Name(format!("notify-row-avatar-{}", row.id).into());
        let achievement = row.item.achievement_id().and_then(|id| UserAchievementsServicesStore::own_achievement(id, cx));
        let avatar = match achievement {
            Some(achievement) => div()
                .flex_none()
                .size(px(AVATAR_SIDE))
                .child(AchievementCard::new(achievement, AVATAR_SIDE).shows_title(false).shows_badge(false))
                .into_any_element(),
            None => NotificationAvatarStack::new(avatar_id, row.facepile.clone())
                .badge_symbol(row.badge_symbol.to_owned())
                .badge_color(row.item.badge_color(&theme))
                .container_size(AVATAR_SIDE)
                .empty_glyph(row.item.empty_avatar_glyph(&theme))
                .into_any_element(),
        };
        hstack(12.0)
            .w_full()
            .items_center()
            .px(px(14.0))
            .py(px(11.0))
            .bg(if row.is_read { theme.bg._100 } else { theme.bg._200 })
            .hover({
                let hover = row_hover_fill(&theme);
                move |style| style.bg(hover)
            })
            .child(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(FONT_SIZE))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(theme.text.primary)
                    .when(row.item.kind != NotificationKind::Announcement, |text| text.line_clamp(3))
                    .child(Self::text(&row, &theme)),
            )
            .when(!row.is_read, |content| {
                content.child(div().size(px(9.0)).flex_none().rounded_full().bg(theme.primary._100))
            })
    }
}
