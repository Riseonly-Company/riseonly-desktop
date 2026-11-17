use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use std::rc::Rc;

use gpui::{
    App, Bounds, FontWeight, Hsla, PathBuilder, Pixels, Point, Rgba, SharedString, Subscription, Task, Window, canvas,
    fill, linear_color_stop, linear_gradient, point, relative, size,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, TimeZone};
use rise_core::foundation::foundation_compare::compare_case_insensitive;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_i18n::app_region_catalog::AppRegionCatalog;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::language_store::Locale;
use rise_i18n::tr;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::secondary_text::SecondaryText;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::NavigationStoreKey;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::{PostStatsCountry, PostStatsDay, RiseStoredPostStats};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

const CARD_RADIUS: f32 = 16.0;
const CARD_PADDING: f32 = 14.0;
const TREND_HEIGHT: f32 = 130.0;
const TREND_LINE_WIDTH: f32 = 2.0;
const BAR_HEIGHT: f32 = 8.0;
const BAR_MIN_WIDTH: f32 = 6.0;
const COLLECTOR_UNKNOWN_COUNTRY: &str = "Unknown";

struct PostStatsPageState {
    _subscriptions: Vec<Subscription>,
}

impl PostStatsPageState {
    fn new(cx: &mut Context<Self>) -> Self {
        let interactions = PostInteractionsStore::shared(cx);
        let services = PostServicesStore::shared(cx);
        let actions = PostActionsStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            _subscriptions: vec![
                cx.observe(&interactions, |_, _, cx| cx.notify()),
                cx.observe(&services, |_, _, cx| cx.notify()),
                cx.observe(&actions, |_, _, cx| cx.notify()),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
                cx.observe(&pending_requests, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }
}

#[derive(IntoElement)]
pub struct PostStats {
    pub post_id: String,
    on_back: Option<Rc<dyn Fn(&mut App)>>,
}

impl PostStats {
    const DAILY_WINDOW_DAYS: i64 = 30;

    pub fn new(post_id: impl Into<String>) -> Self {
        Self { post_id: post_id.into(), on_back: None }
    }

    /// in the right panel the back chevron closes the panel, the stack under it stays put
    pub fn on_back(mut self, on_back: impl Fn(&mut App) + 'static) -> Self {
        self.on_back = Some(Rc::new(on_back));
        self
    }

    fn stats(&self, cx: &mut App) -> Option<RiseStoredPostStats> {
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.post_stats(&self.post_id, cx))
    }

    fn like_color() -> Hsla {
        Rgba { r: 0.90, g: 0.20, b: 0.32, a: 1.0 }.into()
    }

    fn favorite_color(theme: &AppTheme) -> Hsla {
        UIColor::system_orange().resolved_color(theme.appearance)
    }

    fn comment_color() -> Hsla {
        Rgba { r: 0.20, g: 0.50, b: 0.90, a: 1.0 }.into()
    }

    fn share_color() -> Hsla {
        Rgba { r: 0.16, g: 0.71, b: 0.45, a: 1.0 }.into()
    }

    fn content(&self, theme: &AppTheme, cx: &mut App) -> AnyElement {
        if let Some(stats) = self.stats(cx) {
            if stats.has_no_data() {
                return Self::empty_state(theme).into_any_element();
            }
            let days = PostStatsDailySeries::filled(&stats.days, Self::DAILY_WINDOW_DAYS, Date::now());
            return vstack(14.0)
                .w_full()
                .child(div().w_full().child(Self::hero_grid(&stats, theme)).guide_anchor(RiseGuideAnchor::PostStatsSummary))
                .when(!days.is_empty(), |column| {
                    column.child(
                        div().w_full().child(Self::trend_card(&days, theme)).guide_anchor(RiseGuideAnchor::PostStatsDaily),
                    )
                })
                .child(Self::engagement_card(&stats, theme))
                .when(!stats.countries.is_empty(), |column| {
                    column.child(
                        div()
                            .w_full()
                            .child(Self::countries_card(&stats.countries, theme))
                            .guide_anchor(RiseGuideAnchor::PostStatsCountries),
                    )
                })
                .into_any_element();
        }
        let store = PostInteractionsStore::shared(cx);
        let error = store.update(cx, |store, cx| store.post_stats_error_text(&self.post_id, cx));
        let is_loading = store.update(cx, |store, cx| store.is_post_stats_loading(&self.post_id, cx));
        if let Some(message) = error
            && !is_loading
        {
            return self.error_state(message, theme).into_any_element();
        }
        div().w_full().flex().justify_center().pt(px(100.0)).child(progress_view()).into_any_element()
    }

    fn hero_grid(stats: &RiseStoredPostStats, theme: &AppTheme) -> Div {
        let row = || div().w_full().flex().flex_row().items_stretch().gap(px(10.0));
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                row()
                    .child(Self::hero_tile(
                        RiseFormat::compact_count(stats.views_total),
                        tr("post_stats_views"),
                        theme.primary._100,
                        theme,
                    ))
                    .child(Self::hero_tile(
                        RiseFormat::compact_count(stats.unique_viewers),
                        tr("post_stats_unique_viewers"),
                        Self::comment_color(),
                        theme,
                    )),
            )
            .child(
                row()
                    .child(Self::hero_tile(
                        RiseFormat::spelled_duration(stats.avg_dwell_ms),
                        tr("post_stats_avg_dwell"),
                        Self::favorite_color(theme),
                        theme,
                    ))
                    .child(Self::hero_tile(
                        RiseFormat::compact_count(stats.follows_gained),
                        tr("post_stats_follows"),
                        Self::share_color(),
                        theme,
                    )),
            )
    }

    fn hero_tile(value: String, label: String, accent: Hsla, theme: &AppTheme) -> Div {
        vstack(6.0)
            .items_start()
            .flex_1()
            .min_w(px(0.0))
            .p(px(CARD_PADDING))
            .bg(theme.bg._200)
            .clip_shape(Shape::continuous(CARD_RADIUS))
            .child(div().size(px(8.0)).rounded_full().bg(accent))
            .child(MainText::new(value).px(24.0).font_weight(FontWeight::BOLD).color(theme.text.primary))
            .child(MainText::new(label).px(13.0).color(theme.text.secondary))
    }

    fn card(spacing: f32, theme: &AppTheme) -> Div {
        vstack(spacing)
            .items_start()
            .w_full()
            .p(px(CARD_PADDING))
            .bg(theme.bg._200)
            .clip_shape(Shape::continuous(CARD_RADIUS))
    }

    fn trend_card(days: &[PostStatsDay], theme: &AppTheme) -> Div {
        let vals: Vec<f64> = days.iter().map(|day| day.views as f64).collect();
        let max_v = vals.iter().copied().fold(0.0, f64::max).max(1.0);
        let primary = theme.primary._100;
        let chart = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| paint_views_trend(bounds, &vals, max_v, primary, window),
        )
        .w_full()
        .h(px(TREND_HEIGHT));
        let labels = match (days.first(), days.last()) {
            (Some(first), Some(last)) if first.day != last.day => Some(
                hstack(None::<f32>)
                    .w_full()
                    .child(SecondaryText::new(Self::short_day(&first.day)).px(11.0))
                    .child(spacer())
                    .child(SecondaryText::new(Self::short_day(&last.day)).px(11.0)),
            ),
            _ => None,
        };
        Self::card(12.0, theme)
            .child(Self::card_title(tr("post_stats_daily"), theme))
            .child(chart)
            .children(labels)
    }

    fn engagement_card(stats: &RiseStoredPostStats, theme: &AppTheme) -> Div {
        let rows: [(String, i64, Hsla, &'static str); 5] = [
            (tr("post_stats_likes"), stats.likes_total, Self::like_color(), "heart.fill"),
            (tr("post_stats_favorites"), stats.favorites_total, Self::favorite_color(theme), "bookmark.fill"),
            (tr("post_stats_comments"), stats.comments_total, Self::comment_color(), "bubble.right.fill"),
            (tr("post_stats_reposts"), stats.reposts_total, Self::share_color(), "arrow.2.squarepath"),
            (tr("post_stats_shares"), stats.shares_total, Self::share_color(), "arrowshape.turn.up.right.fill"),
        ];
        let max_v = rows.iter().map(|row| row.1).max().unwrap_or(0).max(1);
        let text_primary = theme.text.primary;
        Self::card(14.0, theme)
            .child(Self::card_title(tr("post_stats_engagement"), theme))
            .children(rows.into_iter().map(move |(label, value, color, icon)| {
                vstack(6.0)
                    .items_start()
                    .w_full()
                    .child(
                        hstack(8.0)
                            .w_full()
                            .child(
                                div()
                                    .w(px(18.0))
                                    .flex()
                                    .justify_center()
                                    .children(IconUi::sized(icon, px(12.0), color, icon.ends_with(".fill"))),
                            )
                            .child(MainText::new(label).px(13.0).color(text_primary))
                            .child(spacer())
                            .child(
                                MainText::new(RiseFormat::compact_count(value))
                                    .px(13.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(text_primary),
                            ),
                    )
                    .child(stats_bar(value as f64 / max_v as f64, color))
            }))
    }

    fn countries_card(countries: &[PostStatsCountry], theme: &AppTheme) -> Div {
        let max_views = countries.iter().map(|country| country.unique_views).max().unwrap_or(1).max(1);
        let text_primary = theme.text.primary;
        let primary = theme.primary._100;
        Self::card(14.0, theme)
            .child(Self::card_title(tr("post_stats_countries"), theme))
            .children(countries.iter().map(move |country| {
                vstack(6.0)
                    .items_start()
                    .w_full()
                    .child(
                        hstack(None::<f32>)
                            .w_full()
                            .child(
                                div().min_w(px(0.0)).child(
                                    MainText::new(Self::country_name(&country.country))
                                        .px(13.0)
                                        .color(text_primary)
                                        .number_of_lines(1),
                                ),
                            )
                            .child(spacer())
                            .child(SecondaryText::new(RiseFormat::compact_count(country.unique_views)).px(12.0)),
                    )
                    .child(stats_bar(country.unique_views as f64 / max_views as f64, primary))
            }))
    }

    fn card_title(text: String, theme: &AppTheme) -> MainText {
        MainText::new(text).px(13.0).font_weight(FontWeight::SEMIBOLD).color(theme.text.secondary)
    }

    fn empty_state(theme: &AppTheme) -> Div {
        vstack(16.0)
            .w_full()
            .pt(px(80.0))
            .px(px(24.0))
            .children(IconUi::sized("chart.bar.xaxis", px(46.0), theme.text.secondary.opacity(0.5), false))
            .child(SecondaryText::new(tr("post_stats_empty")).px(14.0).tac(TextAlignT::Center))
    }

    fn error_state(&self, message: String, theme: &AppTheme) -> Div {
        let post_id = self.post_id.clone();
        vstack(16.0)
            .w_full()
            .pt(px(80.0))
            .px(px(24.0))
            .children(IconUi::sized("exclamationmark.triangle", px(40.0), theme.semantic.error_100.opacity(0.7), false))
            .child(SecondaryText::new(message).px(14.0).tac(TextAlignT::Center))
            .child(
                plain(SharedString::from(format!("post-stats-retry-{}", self.post_id)))
                    .on_press(move |_, cx| {
                        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_post_stats(&post_id, cx));
                    })
                    .child(
                        MainText::new(tr("shorts_retry"))
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(theme.primary._100),
                    ),
            )
    }

    fn country_name(raw: &str) -> String {
        let trimmed = trimming_whitespaces(raw);
        if trimmed.is_empty() || compare_case_insensitive(trimmed, COLLECTOR_UNKNOWN_COUNTRY) == Ordering::Equal {
            return tr("post_stats_country_unknown");
        }
        let Some(code) = PostStatsCountryCodes::code(trimmed) else {
            return trimmed.to_owned();
        };
        Locale::app_locale().localized_string_for_region_code(code).unwrap_or_else(|| trimmed.to_owned())
    }

    fn short_day(iso: &str) -> String {
        let parts: Vec<&str> = iso.split('-').filter(|part| !part.is_empty()).collect();
        let [_, month, day] = parts.as_slice() else {
            return iso.to_owned();
        };
        format!("{day}.{month}")
    }
}

impl RenderOnce for PostStats {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let post_id = self.post_id.clone();
        let _state = window.use_keyed_state(SharedString::from(format!("post-stats-state-{post_id}")), cx, |_, cx| {
            PostStatsPageState::new(cx)
        });
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let content = self.content(&theme, cx);

        let refresh_id = post_id.clone();
        let load_id = post_id.clone();
        let close_id = post_id.clone();
        let on_back = self.on_back.clone();
        div()
            .size_full()
            .child(
                SettingsWrapperUi::new(tr("post_stats_title"), move |_, cx| {
                    if let Some(on_back) = &on_back {
                        on_back(cx);
                    } else if let Some(nav_store) = &nav_store {
                        nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                    }
                })
                .content(
                    scroll_view(SharedString::from(format!("post-stats-scroll-{post_id}")))
                        .list_shell_scroll_under_chrome(cx)
                        .bg(theme.bg._100)
                        .refreshable(move |_, cx| {
                            PostInteractionsStore::shared(cx)
                                .update(cx, |store, cx| store.refresh_post_stats(&refresh_id, cx));
                            Task::ready(())
                        })
                        .child(div().w_full().px(px(12.0)).pt(px(12.0)).pb(px(60.0)).child(content)),
                ),
            )
            .on_appear(SharedString::from(format!("post-stats-appear-{post_id}")), move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.load_post_stats(&load_id, cx));
            })
            .and_on_disappear(move |_, cx| {
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.close_post_stats(&close_id, cx));
            })
    }
}

fn stats_bar(ratio: f64, color: Hsla) -> Div {
    let bar = div().absolute().top_0().h_full().w(relative(ratio as f32)).min_w(px(BAR_MIN_WIDTH)).rounded_full().bg(color);
    let bar = if LayoutDirection::current().is_right_to_left() { bar.right_0() } else { bar.left_0() };
    div()
        .relative()
        .w_full()
        .h(px(BAR_HEIGHT))
        .child(div().absolute().inset_0().rounded_full().bg(color.opacity(0.16)))
        .child(bar)
}

fn paint_views_trend(bounds: Bounds<Pixels>, vals: &[f64], max_v: f64, primary: Hsla, window: &mut Window) {
    let n = vals.len();
    if n == 0 {
        return;
    }
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    let line: Vec<Point<Pixels>> = vals
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = if n > 1 { index as f32 / (n - 1) as f32 * width } else { width / 2.0 };
            let y = height - 3.0 - (value / max_v) as f32 * (height - 6.0);
            point(bounds.origin.x + px(x), bounds.origin.y + px(y))
        })
        .collect();
    let bottom = bounds.origin.y + bounds.size.height;

    let mut area = Vec::with_capacity(n + 2);
    area.push(point(line[0].x, bottom));
    area.extend_from_slice(&line);
    area.push(point(line[n - 1].x, bottom));
    let mut fill_builder = PathBuilder::fill();
    fill_builder.add_polygon(&area, true);
    if let Ok(path) = fill_builder.build() {
        window.paint_path(
            path,
            linear_gradient(
                180.0,
                linear_color_stop(primary.opacity(0.30), 0.0),
                linear_color_stop(primary.opacity(0.02), 1.0),
            ),
        );
    }

    let mut line_builder = PathBuilder::stroke(px(TREND_LINE_WIDTH));
    line_builder.add_polygon(&line, false);
    if let Ok(path) = line_builder.build() {
        window.paint_path(path, primary);
    }
    let half = TREND_LINE_WIDTH / 2.0;
    for vertex in &line {
        let dot = Bounds::new(
            point(vertex.x - px(half), vertex.y - px(half)),
            size(px(TREND_LINE_WIDTH), px(TREND_LINE_WIDTH)),
        );
        window.paint_quad(fill(dot, primary).corner_radii(px(half)));
    }
}

pub enum PostStatsCountryCodes {}

impl PostStatsCountryCodes {
    fn by_name() -> &'static HashMap<String, &'static str> {
        static BY_NAME: LazyLock<HashMap<String, &'static str>> = LazyLock::new(|| {
            AppRegionCatalog::all().iter().map(|region| (region.english_name.to_lowercase(), region.code)).collect()
        });
        &BY_NAME
    }

    pub fn code(raw: &str) -> Option<&'static str> {
        if let Some(region) = AppRegionCatalog::region(raw) {
            return Some(region.code);
        }
        Self::by_name().get(&raw.to_lowercase()).copied()
    }
}

/// server returns only days with activity, bucketed by UTC date
/// a single active day drew a zero width chart so the window is filled with empty days to full length
pub enum PostStatsDailySeries {}

impl PostStatsDailySeries {
    fn utc_calendar() -> Calendar {
        let mut calendar = Calendar::gregorian();
        calendar.time_zone = TimeZone::with_identifier("UTC").unwrap_or_else(TimeZone::current);
        calendar
    }

    fn day_formatter() -> &'static Mutex<DateFormatter> {
        static DAY_FORMATTER: LazyLock<Mutex<DateFormatter>> = LazyLock::new(|| {
            let calendar = PostStatsDailySeries::utc_calendar();
            let mut formatter = DateFormatter::new();
            formatter.set_time_zone(calendar.time_zone.clone());
            formatter.set_calendar(calendar);
            formatter.set_locale("en_US_POSIX");
            formatter.set_date_format("yyyy-MM-dd");
            Mutex::new(formatter)
        });
        &DAY_FORMATTER
    }

    pub fn filled(days: &[PostStatsDay], length: i64, now: Date) -> Vec<PostStatsDay> {
        if length <= 0 || !days.iter().any(|day| !day.is_empty()) {
            return Vec::new();
        }
        let calendar = Self::utc_calendar();
        let formatter = Self::day_formatter().lock().unwrap_or_else(PoisonError::into_inner);

        let today = calendar.start_of_day(&now);
        let latest = days.iter().filter_map(|day| formatter.date_from(&day.day)).max().unwrap_or(today);
        let end = today.max(latest);
        let mut by_day: HashMap<&str, &PostStatsDay> = HashMap::with_capacity(days.len());
        for day in days {
            by_day.entry(day.day.as_str()).or_insert(day);
        }

        (0..length)
            .rev()
            .filter_map(|offset| {
                let date = calendar.date_by_adding(CalendarComponent::Day, -offset, &end)?;
                let key = formatter.string_from(&date);
                Some(match by_day.get(key.as_str()) {
                    Some(day) => (*day).clone(),
                    None => PostStatsDay { day: key, ..PostStatsDay::default() },
                })
            })
            .collect()
    }
}
