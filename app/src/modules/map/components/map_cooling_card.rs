use std::rc::Rc;
use std::sync::LazyLock;

use gpui::{App, ElementId, Entity, FontWeight, Hsla, Rgba, SharedString, Window};
use rise_canon::RiseLimits;
use rise_i18n::{tr, tr_plural};
use rise_theme::AppTheme;
use rise_ui::grouped_btns::GroupedCardMetrics;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::stepped_slider_ui::SteppedSliderUi;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::UnitPoint;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;

pub type MapCoolingCommit = Rc<dyn Fn(i64, &mut Window, &mut App)>;

static MINIMUM_WALKS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_track_heat_walks_min"));
static MAXIMUM_WALKS: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_track_heat_walks_max"));
static MINIMUM: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_track_cooling_days_min"));
static MAXIMUM: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_track_cooling_days_max"));

/// how long a walked line takes to cool from red back to blue
/// rail is logarithmic so the first week takes a third, side buttons step one day
#[derive(IntoElement)]
pub struct MapCoolingCard {
    days: i64,
    walks: i64,
    on_commit: MapCoolingCommit,
    on_commit_walks: MapCoolingCommit,
}

/// drafts shown while the rail moves and until the stored value arrives
#[derive(Default)]
struct MapCoolingCardDrafts {
    draft: Option<i64>,
    walks_draft: Option<i64>,
}

#[derive(Clone)]
struct MapCoolingCardParts {
    drafts: Entity<MapCoolingCardDrafts>,
    days: i64,
    walks: i64,
    on_commit: MapCoolingCommit,
    on_commit_walks: MapCoolingCommit,
}

impl MapCoolingCard {
    const RAIL_STEPS: f64 = 100.0;
    const MAGNETS: [i64; 6] = [1, 7, 30, 90, 180, 365];

    pub fn new(
        days: i64,
        walks: i64,
        on_commit: impl Fn(i64, &mut Window, &mut App) + 'static,
        on_commit_walks: impl Fn(i64, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { days, walks, on_commit: Rc::new(on_commit), on_commit_walks: Rc::new(on_commit_walks) }
    }

    /// the line colors themselves, cold to hot
    pub fn scale() -> [Hsla; 6] {
        [
            Self::rgb(0.42, 0.78, 1.0),
            Self::rgb(0.13, 0.36, 0.96),
            Self::rgb(0.13, 0.77, 0.37),
            Self::rgb(0.98, 0.80, 0.08),
            Self::rgb(0.98, 0.51, 0.13),
            Self::rgb(0.94, 0.20, 0.20),
        ]
    }

    fn rgb(r: f32, g: f32, b: f32) -> Hsla {
        Hsla::from(Rgba { r, g, b, a: 1.0 })
    }

    fn step_button(
        id: &'static str,
        symbol: &str,
        theme: &AppTheme,
        is_enabled: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        plain(id).disabled(!is_enabled).on_press(action).child(
            div()
                .size(px(30.0))
                .flex_none()
                .rounded_full()
                .bg(theme.text.secondary.opacity(0.16))
                .flex()
                .items_center()
                .justify_center()
                .children(IconUi::sized(
                    symbol,
                    px(12.0),
                    theme.text.primary.opacity(if is_enabled { 1.0 } else { 0.3 }),
                    false,
                )),
        )
    }

    fn position(days: i64) -> f64 {
        let clamped = days.max(*MINIMUM).min(*MAXIMUM) as f64;
        (clamped.ln() / (*MAXIMUM as f64).ln() * Self::RAIL_STEPS).round()
    }

    fn days(position: f64) -> i64 {
        let raw = (position / Self::RAIL_STEPS * (*MAXIMUM as f64).ln()).exp();
        (raw.round() as i64).max(*MINIMUM).min(*MAXIMUM)
    }
}

impl MapCoolingCardParts {
    fn nudge(&self, delta: i64, shown: i64, window: &mut Window, cx: &mut App) {
        self.commit((shown + delta).max(*MINIMUM).min(*MAXIMUM), window, cx);
    }

    fn commit_walks(&self, next: i64, window: &mut Window, cx: &mut App) {
        let clamped = next.max(*MINIMUM_WALKS).min(*MAXIMUM_WALKS);
        self.drafts.update(cx, |drafts, cx| {
            drafts.walks_draft = Some(clamped);
            cx.notify();
        });
        if clamped == self.walks {
            return;
        }
        (self.on_commit_walks)(clamped, window, cx);
    }

    fn commit(&self, next: i64, window: &mut Window, cx: &mut App) {
        self.drafts.update(cx, |drafts, cx| {
            drafts.draft = Some(next);
            cx.notify();
        });
        if next == self.days {
            return;
        }
        (self.on_commit)(next, window, cx);
    }

    fn set_walks_draft(&self, value: i64, cx: &mut App) {
        self.drafts.update(cx, |drafts, cx| {
            drafts.walks_draft = Some(value);
            cx.notify();
        });
    }

    fn set_draft(&self, value: i64, cx: &mut App) {
        self.drafts.update(cx, |drafts, cx| {
            drafts.draft = Some(value);
            cx.notify();
        });
    }
}

impl RenderOnce for MapCoolingCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let drafts = window.use_keyed_state("map-cooling-card-drafts", cx, |_, _| MapCoolingCardDrafts::default());
        let parts = MapCoolingCardParts {
            drafts: drafts.clone(),
            days: self.days,
            walks: self.walks,
            on_commit: self.on_commit,
            on_commit_walks: self.on_commit_walks,
        };
        let days_drafts = drafts.clone();
        on_change("map-cooling-card-days", self.days, false, window, cx, move |_, _, _, cx| {
            days_drafts.update(cx, |drafts, cx| {
                drafts.draft = None;
                cx.notify();
            });
        });
        let walks_drafts = drafts.clone();
        on_change("map-cooling-card-walks", self.walks, false, window, cx, move |_, _, _, cx| {
            walks_drafts.update(cx, |drafts, cx| {
                drafts.walks_draft = None;
                cx.notify();
            });
        });
        let shown = drafts.read(cx).draft.unwrap_or(self.days);
        let shown_walks = drafts.read(cx).walks_draft.unwrap_or(self.walks);

        let walks_minus = parts.clone();
        let walks_change = parts.clone();
        let walks_commit = parts.clone();
        let walks_plus = parts.clone();
        let days_minus = parts.clone();
        let days_change = parts.clone();
        let days_commit = parts.clone();
        let days_plus = parts;

        vstack(12.0)
            .w_full()
            .child(
                div().w_full().h(px(6.0)).rounded_full().overflow_hidden().child(
                    LinearGradient::colors(MapCoolingCard::scale(), UnitPoint::LEADING, UnitPoint::TRAILING)
                        .view()
                        .size_full(),
                ),
            )
            .child(
                hstack(None)
                    .w_full()
                    .items_baseline()
                    .child(MainText::new(tr("map_settings_heat_walks_title")).px(16.0).number_of_lines(1))
                    .child(spacer_min_w(8.0))
                    .child(
                        MainText::new(tr_plural("map_settings_heat_walks", shown_walks))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .primary(true)
                            .number_of_lines(1)
                            .monospaced_digits(true),
                    ),
            )
            .child(
                hstack(10.0)
                    .w_full()
                    .items_center()
                    .child(MapCoolingCard::step_button(
                        "map-cooling-walks-minus",
                        "minus",
                        &theme,
                        shown_walks > *MINIMUM_WALKS,
                        move |window, cx| walks_minus.commit_walks(shown_walks - 1, window, cx),
                    ))
                    .child(
                        div().flex_1().h(px(28.0)).child(SteppedSliderUi::new(
                            ElementId::Name(SharedString::from("map-cooling-walks-rail")),
                            shown_walks as f64,
                            (*MINIMUM_WALKS as f64)..=(*MAXIMUM_WALKS as f64),
                            1.0,
                            move |value, _, cx| walks_change.set_walks_draft(value.round() as i64, cx),
                            move |value, window, cx| walks_commit.commit_walks(value.round() as i64, window, cx),
                        )),
                    )
                    .child(MapCoolingCard::step_button(
                        "map-cooling-walks-plus",
                        "plus",
                        &theme,
                        shown_walks < *MAXIMUM_WALKS,
                        move |window, cx| walks_plus.commit_walks(shown_walks + 1, window, cx),
                    )),
            )
            .child(
                hstack(None)
                    .w_full()
                    .items_baseline()
                    .child(MainText::new(tr("map_settings_cooling_title")).px(16.0).number_of_lines(1))
                    .child(spacer_min_w(8.0))
                    .child(
                        MainText::new(tr_plural("map_settings_cooling_days", shown))
                            .px(16.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .primary(true)
                            .number_of_lines(1)
                            .monospaced_digits(true),
                    ),
            )
            .child(
                hstack(10.0)
                    .w_full()
                    .items_center()
                    .child(MapCoolingCard::step_button(
                        "map-cooling-days-minus",
                        "minus",
                        &theme,
                        shown > *MINIMUM,
                        move |window, cx| days_minus.nudge(-1, shown, window, cx),
                    ))
                    .child(
                        div().flex_1().h(px(28.0)).child(
                            SteppedSliderUi::new(
                                ElementId::Name(SharedString::from("map-cooling-days-rail")),
                                MapCoolingCard::position(shown),
                                0.0..=MapCoolingCard::RAIL_STEPS,
                                1.0,
                                move |value, _, cx| days_change.set_draft(MapCoolingCard::days(value), cx),
                                move |value, window, cx| days_commit.commit(MapCoolingCard::days(value), window, cx),
                            )
                            .breakpoints(MapCoolingCard::MAGNETS.iter().map(|days| MapCoolingCard::position(*days)).collect())
                            .shows_step_ticks(false),
                        ),
                    )
                    .child(MapCoolingCard::step_button(
                        "map-cooling-days-plus",
                        "plus",
                        &theme,
                        shown < *MAXIMUM,
                        move |window, cx| days_plus.nudge(1, shown, window, cx),
                    )),
            )
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
    }
}
