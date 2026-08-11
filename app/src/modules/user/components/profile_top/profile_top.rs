use std::rc::Rc;

use gpui::{
    ClickEvent, Context, Div, ElementId, FontWeight, IntoElement, SharedString, Stateful, Window,
    div, prelude::*,
};
use rise_i18n::tr;
use rise_theme::{AppTheme, ProfileMetrics};
use rise_ui::{AvatarSpec, AvatarUi, BadgeUi, IconSize, IconUi, ImageUi, SkeletonUi, StoryRing};

use crate::modules::user::engine::rise_user_engine_models::{FollowButtonState, ProfileModel};

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProfileTopAction {
    ToggleFollow,
    ShowAbout,
    OpenLink(String),
}

pub type ProfileTopHandler<V> = Rc<dyn Fn(&mut V, ProfileTopAction, &mut Window, &mut Context<V>)>;

pub struct ProfileTop;

impl ProfileTop {
    pub fn render<V: 'static>(
        theme: &AppTheme,
        profile: &ProfileModel,
        is_viewer: bool,
        follow_in_flight: bool,
        handler: &ProfileTopHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let metrics = theme.profile;

        let mut column = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(metrics.block_gap)
            .child(Self::banner(theme, profile, cx))
            .child(Self::about(theme, profile, handler, cx));

        if is_viewer {
            if let Some(pills) = Self::rise_pills(theme, profile) {
                column = column.child(pills);
            }
        } else {
            column = column.child(Self::actions(theme, profile, follow_in_flight, handler, cx));
        }

        column
    }

    pub fn skeleton(theme: &AppTheme) -> impl IntoElement {
        let theme = theme.clone();

        SkeletonUi::group("profile.skeleton", move |band| {
            let metrics = theme.profile;

            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(metrics.block_gap)
                .child(
                    div()
                        .w_full()
                        .h(metrics.banner_height)
                        .bg(theme.bg._200)
                        .rounded_b(metrics.banner_radius),
                )
                .child(
                    div()
                        .px(metrics.padding_x)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(metrics.identity_gap)
                        .child(SkeletonUi::circle(&theme, band, metrics.avatar_size))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(metrics.identity_row_gap)
                                .child(SkeletonUi::line(&theme, band, metrics.content_width / 3.))
                                .child(SkeletonUi::line(&theme, band, metrics.content_width / 5.)),
                        ),
                )
                .child(
                    div().px(metrics.padding_x).child(
                        div()
                            .w_full()
                            .h(metrics.stats_height)
                            .rounded(metrics.surface_radius)
                            .bg(theme.bg._200),
                    ),
                )
        })
    }

    fn banner<V: 'static>(theme: &AppTheme, profile: &ProfileModel, cx: &mut Context<V>) -> Div {
        let metrics = theme.profile;

        let mut cover = div()
            .w_full()
            .h(metrics.banner_height)
            .bg(theme.bg._200)
            .rounded_b(metrics.banner_radius)
            .overflow_hidden();

        if let Some(url) = profile.cover_image_url.as_deref() {
            cover = cover.child(
                ImageUi::remote(url.to_owned(), cx as &gpui::App)
                    .w_full()
                    .h(metrics.banner_height)
                    .object_fit(gpui::ObjectFit::Cover),
            );
        }

        div()
            .w_full()
            .flex()
            .flex_col()
            .child(cover)
            .child(Self::identity(theme, profile, cx).mt(-metrics.avatar_hang))
    }

    fn identity<V: 'static>(theme: &AppTheme, profile: &ProfileModel, cx: &mut Context<V>) -> Div {
        let metrics = theme.profile;
        let name = theme
            .typography
            .style(ProfileMetrics::NAME_SIZE, FontWeight::BOLD);
        let tag = theme
            .typography
            .style(ProfileMetrics::TAG_SIZE, FontWeight::NORMAL);

        let mut identity = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.pill_gap)
            .child(
                div()
                    .text_size(name.size)
                    .line_height(name.line_height)
                    .font(name.font)
                    .text_color(theme.text.primary)
                    .truncate()
                    .child(profile.name.clone()),
            );

        for badge in BadgeUi::for_author(
            theme,
            metrics.badge_size,
            profile.is_official,
            profile.is_premium,
        ) {
            identity = identity.child(badge);
        }

        let mut names = div()
            .flex()
            .flex_col()
            .gap(metrics.identity_row_gap)
            .pt(metrics.identity_top)
            .child(identity);

        if !profile.tag.is_empty() {
            names = names.child(
                div()
                    .text_size(tag.size)
                    .line_height(tag.line_height)
                    .font(tag.font)
                    .text_color(theme.text.secondary)
                    .truncate()
                    .child(format!("@{}", profile.tag)),
            );
        }

        div()
            .w_full()
            .px(metrics.padding_x)
            .flex()
            .flex_row()
            .items_start()
            .gap(metrics.identity_gap)
            .child(
                div()
                    .rounded(metrics.avatar_size)
                    .border(metrics.avatar_border)
                    .border_color(theme.bg._100)
                    .child(AvatarUi::render(
                        theme,
                        metrics.avatar_size,
                        AvatarSpec {
                            source: profile.avatar_url.as_deref(),
                            name: Some(&profile.name),
                            user_id: Some(&profile.id),
                            ring: StoryRing::None,
                            show_border: false,
                        },
                        cx as &gpui::App,
                    )),
            )
            .child(names)
    }

    fn about<V: 'static>(
        theme: &AppTheme,
        profile: &ProfileModel,
        handler: &ProfileTopHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let metrics = theme.profile;
        let label = theme
            .typography
            .style(ProfileMetrics::LABEL_SIZE, FontWeight::NORMAL);
        let bio = theme
            .typography
            .style(ProfileMetrics::BIO_SIZE, FontWeight::NORMAL);

        let mut block = div()
            .w_full()
            .px(metrics.padding_x)
            .flex()
            .flex_col()
            .gap(metrics.about_gap)
            .child(
                div()
                    .pl(metrics.about_inset)
                    .text_size(label.size)
                    .line_height(label.line_height)
                    .font(label.font)
                    .text_color(theme.text.secondary)
                    .child(tr("profile_about_title")),
            );

        if !profile.description.trim().is_empty() {
            block = block.child(
                div()
                    .px(metrics.about_inset)
                    .text_size(bio.size)
                    .line_height(bio.line_height)
                    .font(bio.font)
                    .text_color(theme.text.primary)
                    .child(profile.description.clone()),
            );
        }

        block = block.child(Self::stats(theme, profile));

        let has_detail = !profile.who.trim().is_empty()
            || !profile.stack.is_empty()
            || !profile.p_lang.is_empty()
            || !profile.social_links.is_empty();

        if has_detail {
            block = block.child(Self::more_button(theme, handler, cx));
        }

        block
    }

    fn stats(theme: &AppTheme, profile: &ProfileModel) -> Div {
        let metrics = theme.profile;
        let value = theme
            .typography
            .style(ProfileMetrics::STAT_VALUE_SIZE, FontWeight::BOLD);
        let label = theme
            .typography
            .style(ProfileMetrics::STAT_LABEL_SIZE, FontWeight::NORMAL);

        let segments = [
            (profile.posts_count, "stats_posts"),
            (profile.subscribers_count, "stats_subscribers"),
            (profile.subs_count, "stats_subs"),
            (profile.friends_count, "stats_friends"),
        ];

        let mut bar = div()
            .w_full()
            .h(metrics.stats_height)
            .flex()
            .flex_row()
            .items_center()
            .rounded(metrics.surface_radius)
            .bg(theme.bg._200)
            .border(metrics.surface_border)
            .border_color(theme.border._100);

        for (index, (amount, key)) in segments.into_iter().enumerate() {
            if index > 0 {
                bar = bar.child(
                    div()
                        .w(metrics.surface_border)
                        .h(metrics.stats_divider_height)
                        .bg(theme.border._100),
                );
            }

            bar = bar.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(metrics.stats_value_gap)
                    .child(
                        div()
                            .text_size(value.size)
                            .line_height(value.line_height)
                            .font(value.font.clone())
                            .text_color(theme.text.primary)
                            .child(format_count(amount)),
                    )
                    .child(
                        div()
                            .text_size(label.size)
                            .line_height(label.line_height)
                            .font(label.font.clone())
                            .text_color(theme.text.secondary)
                            .child(tr(key)),
                    ),
            );
        }

        bar
    }

    fn rise_pills(theme: &AppTheme, profile: &ProfileModel) -> Option<Div> {
        let metrics = theme.profile;
        let pills = [
            ("arrow.up.forward.circle.fill", profile.level, "rise_level"),
            ("flame.fill", profile.streak, "rise_streak"),
            ("star.fill", profile.rating, "rise_rating"),
        ];
        if pills.iter().all(|(_, amount, _)| *amount <= 0) {
            return None;
        }

        let value = theme
            .typography
            .style(ProfileMetrics::PILL_VALUE_SIZE, FontWeight::SEMIBOLD);
        let label = theme
            .typography
            .style(ProfileMetrics::PILL_LABEL_SIZE, FontWeight::NORMAL);

        let mut row = div()
            .w_full()
            .px(metrics.padding_x)
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.pill_row_gap);

        for (symbol, amount, key) in pills {
            if amount <= 0 {
                continue;
            }

            let mut pill = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(metrics.pill_gap)
                .px(metrics.pill_padding_x)
                .py(metrics.pill_padding_y)
                .rounded(metrics.surface_radius)
                .bg(theme.bg._200)
                .border(metrics.surface_border)
                .border_color(theme.border._100);

            if let Some(glyph) = IconUi::render(theme, symbol, IconSize::Small, theme.primary._100)
            {
                pill = pill.child(glyph);
            }

            row = row.child(
                pill.child(
                    div()
                        .text_size(value.size)
                        .line_height(value.line_height)
                        .font(value.font.clone())
                        .text_color(theme.text.primary)
                        .child(amount.to_string()),
                )
                .child(
                    div()
                        .text_size(label.size)
                        .line_height(label.line_height)
                        .font(label.font.clone())
                        .text_color(theme.text.secondary)
                        .child(tr(key)),
                ),
            );
        }

        Some(row)
    }

    fn actions<V: 'static>(
        theme: &AppTheme,
        profile: &ProfileModel,
        follow_in_flight: bool,
        handler: &ProfileTopHandler<V>,
        cx: &mut Context<V>,
    ) -> Div {
        let metrics = theme.profile;
        let state = profile.follow_button_state();
        let style = theme
            .typography
            .style(ProfileMetrics::ACTION_SIZE, FontWeight::SEMIBOLD);
        let color = if state.is_primary() {
            theme.primary._100
        } else {
            theme.text.primary
        };

        let handler = Rc::clone(handler);
        let mut button = div()
            .id(ElementId::Name(SharedString::from("profile.follow")))
            .flex_1()
            .h(metrics.action_height)
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(metrics.pill_gap)
            .px(metrics.pill_padding_x)
            .rounded(metrics.surface_radius)
            .bg(theme.bg._200)
            .border(metrics.surface_border)
            .border_color(theme.border._100)
            .cursor_pointer()
            .opacity(if follow_in_flight { 0.7 } else { 1.0 })
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                handler(view, ProfileTopAction::ToggleFollow, window, cx);
            }));

        if state == FollowButtonState::Friends
            && let Some(glyph) = IconUi::sized("person.2.fill", metrics.badge_size, color, true)
        {
            button = button.child(glyph);
        }

        div().w_full().px(metrics.padding_x).child(
            button.child(
                div()
                    .text_size(style.size)
                    .line_height(style.line_height)
                    .font(style.font)
                    .text_color(color)
                    .child(tr(state.title_key())),
            ),
        )
    }

    fn more_button<V: 'static>(
        theme: &AppTheme,
        handler: &ProfileTopHandler<V>,
        cx: &mut Context<V>,
    ) -> Stateful<Div> {
        let metrics = theme.profile;
        let style = theme
            .typography
            .style(ProfileMetrics::ACTION_SIZE, FontWeight::SEMIBOLD);
        let handler = Rc::clone(handler);

        let mut button = div()
            .id(ElementId::Name(SharedString::from("profile.more")))
            .w_full()
            .h(metrics.action_height)
            .flex()
            .flex_row()
            .items_center()
            .gap(metrics.action_gap)
            .px(metrics.pill_padding_x)
            .rounded(metrics.surface_radius)
            .bg(theme.bg._200)
            .border(metrics.surface_border)
            .border_color(theme.border._100)
            .cursor_pointer()
            .on_click(cx.listener(move |view, _: &ClickEvent, window, cx| {
                handler(view, ProfileTopAction::ShowAbout, window, cx);
            }));

        if let Some(glyph) = IconUi::primary(theme, "info.circle", IconSize::Regular) {
            button = button.child(glyph);
        }

        button = button.child(
            div()
                .flex_1()
                .text_size(style.size)
                .line_height(style.line_height)
                .font(style.font)
                .text_color(theme.text.primary)
                .child(tr("profile_more_button")),
        );

        if let Some(glyph) = IconUi::secondary(theme, "chevron.right", IconSize::Small) {
            button = button.child(glyph);
        }

        button
    }
}

/// Matches the reference's `formatNumber`: a thousand becomes `1.0K`, and
/// everything below it keeps its exact value.
pub fn format_count(amount: i64) -> String {
    if amount >= 1_000 {
        return format!("{:.1}K", amount as f64 / 1_000.0);
    }
    amount.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_is_abbreviated_exactly_where_the_reference_abbreviates_it() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_000), "1.0K");
        assert_eq!(format_count(1_234), "1.2K");
        assert_eq!(format_count(12_000), "12.0K");
    }

    #[test]
    fn the_follow_button_names_a_string_for_every_state_it_can_be_in() {
        let catalogue = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/locales/locale-en.json");
        let text = std::fs::read_to_string(catalogue).expect("the English catalogue ships");

        for state in [
            FollowButtonState::Subscribe,
            FollowButtonState::Following,
            FollowButtonState::SubscribeBack,
            FollowButtonState::Friends,
        ] {
            assert!(
                text.contains(&format!("\"{}\"", state.title_key())),
                "{} renders as itself, which is how a missing string ships",
                state.title_key()
            );
        }
    }

    #[test]
    fn every_string_the_header_draws_is_in_the_catalogue() {
        let catalogue = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/locales/locale-en.json");
        let text = std::fs::read_to_string(catalogue).expect("the English catalogue ships");

        for key in [
            "profile_about_title",
            "profile_more_button",
            "stats_posts",
            "stats_subscribers",
            "stats_subs",
            "stats_friends",
            "rise_level",
            "rise_streak",
            "rise_rating",
        ] {
            assert!(
                text.contains(&format!("\"{key}\"")),
                "{key} is untranslated"
            );
        }
    }
}
