use gpui::{Div, FontWeight, div, prelude::*};
use rise_i18n::tr;
use rise_theme::{AppTheme, ProfileMetrics};
use rise_ui::TabItem;

use crate::modules::post::engine::rise_post_rpc::UserFeedKind;
use crate::modules::user::engine::rise_user_engine_models::{GoalItem, PlanItem};

/// The tabs a profile shows. `comments` is in the reference and not here: it is
/// answered by comment-service, which has no domain in this client yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileContentTab {
    Publications,
    Threads,
    Reposts,
    Goals,
    Plans,
}

impl ProfileContentTab {
    pub const ALL: [ProfileContentTab; 5] = [
        ProfileContentTab::Publications,
        ProfileContentTab::Threads,
        ProfileContentTab::Reposts,
        ProfileContentTab::Goals,
        ProfileContentTab::Plans,
    ];

    pub fn feed(self) -> Option<UserFeedKind> {
        match self {
            Self::Publications => Some(UserFeedKind::Publications),
            Self::Threads => Some(UserFeedKind::Threads),
            Self::Reposts => Some(UserFeedKind::Reposts),
            Self::Goals | Self::Plans => None,
        }
    }

    pub fn title_key(self) -> &'static str {
        match self {
            Self::Publications => "profile_tab_publications",
            Self::Threads => "profile_tab_threads",
            Self::Reposts => "profile_tab_reposts",
            Self::Goals => "goals_title",
            Self::Plans => "plans_title",
        }
    }

    pub fn tab_id(self) -> &'static str {
        match self {
            Self::Publications => "profile.publications",
            Self::Threads => "profile.threads",
            Self::Reposts => "profile.reposts",
            Self::Goals => "profile.goals",
            Self::Plans => "profile.plans",
        }
    }

    pub fn empty_key(self) -> &'static str {
        match self {
            Self::Goals => "goals_empty",
            Self::Plans => "plans_empty",
            other => other.feed().expect("a feed tab").empty_key(),
        }
    }

    pub fn empty_symbol(self) -> &'static str {
        match self {
            Self::Goals => "target",
            Self::Plans => "calendar",
            other => other.feed().expect("a feed tab").empty_symbol(),
        }
    }

    pub fn items() -> Vec<TabItem> {
        Self::ALL
            .iter()
            .map(|tab| TabItem::new(tab.tab_id(), tr(tab.title_key())))
            .collect()
    }
}

pub struct ProfileContent;

impl ProfileContent {
    pub fn goal(theme: &AppTheme, goal: &GoalItem) -> Div {
        let metrics = theme.profile;
        let title = theme
            .typography
            .style(ProfileMetrics::ROW_TITLE_SIZE, FontWeight::NORMAL);
        let meta = theme
            .typography
            .style(ProfileMetrics::ROW_META_SIZE, FontWeight::NORMAL);
        let percent = (goal.progress.clamp(0.0, 1.0) * 100.0).round() as i64;

        Self::row(theme)
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(metrics.row_gap)
                    .child(
                        div()
                            .flex_1()
                            .text_size(title.size)
                            .line_height(title.line_height)
                            .font(title.font)
                            .text_color(theme.text.primary)
                            .truncate()
                            .child(goal.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(meta.size)
                            .line_height(meta.line_height)
                            .font(meta.font)
                            .text_color(theme.text.secondary)
                            .child(format!("{percent}%")),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(metrics.progress_height)
                    .rounded(metrics.progress_height)
                    .bg(theme.bg._300)
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(goal.progress.clamp(0.0, 1.0)))
                            .rounded(metrics.progress_height)
                            .bg(theme.primary._100),
                    ),
            )
    }

    pub fn plan(theme: &AppTheme, plan: &PlanItem, now_ms: i64) -> Div {
        let title = theme
            .typography
            .style(ProfileMetrics::ROW_TITLE_SIZE, FontWeight::NORMAL);
        let meta = theme
            .typography
            .style(ProfileMetrics::ROW_META_SIZE, FontWeight::NORMAL);

        let mut row = Self::row(theme).child(
            div()
                .w_full()
                .text_size(title.size)
                .line_height(title.line_height)
                .font(title.font)
                .text_color(theme.text.primary)
                .child(plan.title.clone()),
        );

        let elapsed = rise_i18n::relative_time::relative(Some(plan.date_ms), "", now_ms);
        if plan.date_ms > 0 && !elapsed.is_empty() {
            row = row.child(
                div()
                    .text_size(meta.size)
                    .line_height(meta.line_height)
                    .font(meta.font)
                    .text_color(theme.text.secondary)
                    .child(elapsed),
            );
        }

        row
    }

    fn row(theme: &AppTheme) -> Div {
        let metrics = theme.profile;

        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(metrics.row_gap)
            .px(metrics.row_padding_x)
            .py(metrics.row_padding_y)
            .rounded(metrics.row_radius)
            .bg(theme.bg._200)
            .border(metrics.surface_border)
            .border_color(theme.border._100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_three_post_tabs_address_a_feed() {
        let feeds: Vec<Option<UserFeedKind>> = ProfileContentTab::ALL
            .iter()
            .map(|tab| tab.feed())
            .collect();

        assert_eq!(
            feeds,
            vec![
                Some(UserFeedKind::Publications),
                Some(UserFeedKind::Threads),
                Some(UserFeedKind::Reposts),
                None,
                None
            ],
            "goals and plans come inside the profile payload and cost no request"
        );
    }

    #[test]
    fn every_tab_has_an_id_of_its_own() {
        let mut ids: Vec<&str> = ProfileContentTab::ALL
            .iter()
            .map(|tab| tab.tab_id())
            .collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();

        assert_eq!(
            ids.len(),
            count,
            "a repeated ElementId is dropped in release without a word"
        );
    }

    #[test]
    fn every_tab_names_strings_the_catalogue_carries() {
        let catalogue = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/locales/locale-en.json");
        let text = std::fs::read_to_string(catalogue).expect("the English catalogue ships");

        for tab in ProfileContentTab::ALL {
            for key in [tab.title_key(), tab.empty_key()] {
                assert!(
                    text.contains(&format!("\"{key}\"")),
                    "{key} renders as itself, which is how a missing string ships"
                );
            }
        }
    }
}
