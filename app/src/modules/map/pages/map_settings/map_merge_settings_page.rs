use std::ops::RangeInclusive;
use std::rc::Rc;

use gpui::AppContext as _;
use gpui::{App, Bounds, Corners, ElementId, FontWeight, Hsla, Point, SharedString, Task, Window, canvas, fill, point, size, white};
use rise_i18n::{FormatArg, tr, tr_args, tr_plural};
use rise_theme::AppTheme;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::settings_icon_palette::SettingsIconPalette;
use rise_ui::stepped_slider_ui::SteppedSliderUi;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::segmented_picker::SegmentedPicker;
use rise_ui::switch_ui::SwitchUi;

use crate::modules::map::components::map_cooling_card::MapCoolingCard;
use crate::modules::map::stores::map_actions::map_types::{
    MapMergePreview, MapMergePreviewSource, MapMergePreviewState, MapMergePreviewStroke, MapSettings, MapSettingsPatch,
};
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::media_picker::editor::editor_overlay_canvas::EditorOverlayCanvas;

#[derive(Default)]
struct MapMergeSettingsSectionDrafts {
    radius_draft: Option<i64>,
    lanes_draft: Option<i64>,
}

fn update_settings(patch: MapSettingsPatch, cx: &mut App) {
    MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.update_settings(patch, cx));
}

fn preview_merge(radius_m: i64, lanes: i64, source: Option<MapMergePreviewSource>, debounced: bool, cx: &mut App) {
    MapInteractionsStore::shared(cx)
        .update(cx, |interactions, cx| interactions.preview_merge(radius_m, lanes, source, debounced, cx));
}

/// line merging
/// a switch and the hitbox and lanes as numbers on sliders
/// plus a server drawn preview with the real merge
#[derive(IntoElement)]
pub struct MapMergeSettingsSection {
    settings: MapSettings,
}

impl MapMergeSettingsSection {
    pub fn new(settings: MapSettings) -> Self {
        Self { settings }
    }

    fn set_enabled(enabled: bool, current: bool, cx: &mut App) {
        if enabled == current {
            return;
        }
        update_settings(MapSettingsPatch { track_merge_enabled: Some(enabled), ..MapSettingsPatch::default() }, cx);
    }

    fn enabled_row(settings: &MapSettings) -> GroupedBtn {
        let enabled = settings.track_merge_enabled;
        GroupedBtn::new("map_merge_enabled", tr("map_merge_enabled"))
            .icon("arrow.triangle.merge")
            .icon_background(SettingsIconPalette::customization())
            .icon_foreground(white())
            .show_chevron(false)
            .trailing_slot(SwitchUi::new(
                "map-merge-enabled-switch",
                Binding::new(move |_| enabled, move |next, _, cx| Self::set_enabled(next, enabled, cx)),
            ))
            .callback(move |_, cx| Self::set_enabled(!enabled, enabled, cx))
    }

    fn radius_control(
        settings: &MapSettings,
        drafts: &gpui::Entity<MapMergeSettingsSectionDrafts>,
        radius: i64,
        lanes: i64,
    ) -> MapMergeStepper {
        let stored = settings.track_merge_radius_m;
        let draft_drafts = drafts.clone();
        let commit_drafts = drafts.clone();
        MapMergeStepper {
            id: "map-merge-radius",
            title: tr("map_merge_radius_title"),
            value: radius,
            range: MapSettings::minimum_merge_radius()..=MapSettings::maximum_merge_radius(),
            shows_ticks: false,
            text: Rc::new(|value: i64| tr_args("map_home_track_radius_value", &[FormatArg::Int(value)])),
            on_draft: Rc::new(move |next: i64, cx: &mut App| {
                draft_drafts.update(cx, |drafts, cx| {
                    drafts.radius_draft = Some(next);
                    cx.notify();
                });
                preview_merge(next, lanes, None, true, cx);
            }),
            on_commit: Rc::new(move |next: i64, cx: &mut App| {
                commit_drafts.update(cx, |drafts, cx| {
                    drafts.radius_draft = Some(next);
                    cx.notify();
                });
                preview_merge(next, lanes, None, false, cx);
                if next == stored {
                    return;
                }
                update_settings(MapSettingsPatch { track_merge_radius_m: Some(next), ..MapSettingsPatch::default() }, cx);
            }),
        }
    }

    fn lanes_control(
        settings: &MapSettings,
        drafts: &gpui::Entity<MapMergeSettingsSectionDrafts>,
        radius: i64,
        lanes: i64,
    ) -> MapMergeStepper {
        let stored = settings.track_merge_lanes;
        let draft_drafts = drafts.clone();
        let commit_drafts = drafts.clone();
        MapMergeStepper {
            id: "map-merge-lanes",
            title: tr("map_merge_lanes_title"),
            value: lanes,
            range: MapSettings::minimum_merge_lanes()..=MapSettings::maximum_merge_lanes(),
            shows_ticks: true,
            text: Rc::new(|value: i64| value.to_string()),
            on_draft: Rc::new(move |next: i64, cx: &mut App| {
                draft_drafts.update(cx, |drafts, cx| {
                    drafts.lanes_draft = Some(next);
                    cx.notify();
                });
                preview_merge(radius, next, None, true, cx);
            }),
            on_commit: Rc::new(move |next: i64, cx: &mut App| {
                commit_drafts.update(cx, |drafts, cx| {
                    drafts.lanes_draft = Some(next);
                    cx.notify();
                });
                preview_merge(radius, next, None, false, cx);
                if next == stored {
                    return;
                }
                update_settings(MapSettingsPatch { track_merge_lanes: Some(next), ..MapSettingsPatch::default() }, cx);
            }),
        }
    }

    fn preview_card(
        settings: &MapSettings,
        state: &MapMergePreviewState,
        source: MapMergePreviewSource,
        radius: i64,
        lanes: i64,
        theme: &AppTheme,
    ) -> AnyElement {
        let preview = state.preview();
        let picker_sources = [MapMergePreviewSource::Sample, MapMergePreviewSource::Mine];
        let selected_index = picker_sources.iter().position(|candidate| *candidate == source).unwrap_or_default();

        let mut card = vstack(14.0)
            .w_full()
            .child(SegmentedPicker::new(
                "map-merge-preview-source",
                vec![SharedString::from(tr("map_merge_preview_sample")), SharedString::from(tr("map_merge_preview_mine"))],
                selected_index,
                move |index, _, cx| {
                    if let Some(chosen) = picker_sources.get(index) {
                        preview_merge(radius, lanes, Some(*chosen), false, cx);
                    }
                },
            ))
            .child(
                Shape::continuous(14.0)
                    .round(div())
                    .relative()
                    .w_full()
                    .aspect_ratio(1.0)
                    .overflow_hidden()
                    .bg(theme.bg._100)
                    .child(MapMergePreviewCanvas::new(
                        preview.cloned(),
                        settings.track_merge_enabled,
                        settings.track_heat_walks,
                    ))
                    .child(Self::preview_overlay(state, theme)),
            );

        if let Some(preview) = preview {
            let mut legend = hstack(14.0)
                .w_full()
                .flex_wrap()
                .gap_y(px(6.0))
                .items_center()
                .child(Self::legend_line(theme.text.secondary.opacity(0.6), 1.5, tr("map_merge_preview_legend_walks")));
            if settings.track_merge_enabled {
                legend = legend
                    .child(Self::legend_line(MapCoolingCard::scale()[2], 3.0, tr("map_merge_preview_legend_lines")))
                    .child(Self::legend_line(theme.primary._100.opacity(0.25), 9.0, tr("map_merge_radius_title")));
            }
            card = card.child(legend).child(
                div().w_full().child(
                    MainText::new(Self::counts(preview, settings))
                        .px(13.0)
                        .color(theme.text.secondary)
                        .number_of_lines(1)
                        .monospaced_digits(true),
                ),
            );
        }

        card.px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
            .into_any_element()
    }

    fn preview_overlay(state: &MapMergePreviewState, theme: &AppTheme) -> AnyElement {
        let message = |text: String| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.0))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .child(MainText::new(text).px(14.0).tac(TextAlignT::Center).color(theme.text.secondary)),
                )
                .into_any_element()
        };
        match state {
            MapMergePreviewState::Loading(None) | MapMergePreviewState::Idle => div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(progress_view())
                .into_any_element(),
            MapMergePreviewState::Loading(Some(_)) => div()
                .absolute()
                .inset_0()
                .flex()
                .items_start()
                .justify_end()
                .p(px(10.0))
                .child(progress_view().control_size(ControlSize::Small))
                .into_any_element(),
            MapMergePreviewState::Failed(text) => {
                message(text.clone().unwrap_or_else(|| tr("map_load_failed")))
            }
            MapMergePreviewState::Loaded(preview) if preview.walk_count == 0 => message(tr("map_merge_preview_empty")),
            MapMergePreviewState::Loaded(_) => div().into_any_element(),
        }
    }

    fn legend_line(color: Hsla, width: f32, text: String) -> impl IntoElement {
        hstack(6.0)
            .flex_none()
            .items_center()
            .child(
                div()
                    .w(px(18.0))
                    .h(px(10.0))
                    .flex()
                    .items_center()
                    .child(div().w(px(18.0)).h(px(width)).rounded_full().bg(color)),
            )
            .child(MainText::new(text).px(12.0).number_of_lines(1))
    }

    fn counts(preview: &MapMergePreview, settings: &MapSettings) -> String {
        let walks = tr_plural("map_merge_walks", preview.walk_count);
        if !settings.track_merge_enabled {
            return walks;
        }
        format!("{walks}, {}", tr_plural("map_merge_lines", preview.line_count))
    }
}

impl RenderOnce for MapMergeSettingsSection {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let settings = self.settings;
        let drafts = window.use_keyed_state("map-merge-settings-drafts", cx, |_, _| MapMergeSettingsSectionDrafts::default());

        let radius_change = drafts.clone();
        on_change("map-merge-settings-radius", settings.track_merge_radius_m, false, window, cx, move |_, _, _, cx| {
            radius_change.update(cx, |drafts, cx| {
                drafts.radius_draft = None;
                cx.notify();
            });
        });
        let lanes_change = drafts.clone();
        on_change("map-merge-settings-lanes", settings.track_merge_lanes, false, window, cx, move |_, _, _, cx| {
            lanes_change.update(cx, |drafts, cx| {
                drafts.lanes_draft = None;
                cx.notify();
            });
        });

        let radius = drafts.read(cx).radius_draft.unwrap_or(settings.track_merge_radius_m);
        let lanes = drafts.read(cx).lanes_draft.unwrap_or(settings.track_merge_lanes);
        let interactions = MapInteractionsStore::shared(cx);
        let (merge_preview, merge_preview_source) = {
            let interactions = interactions.read(cx);
            (interactions.merge_preview.clone(), interactions.merge_preview_source)
        };
        let is_truncated = merge_preview.preview().is_some_and(|preview| preview.truncated);

        let preview_group = GroupedBtns::new(
            "map-merge-preview-group",
            vec![GroupedBtn::new("map_merge_preview", "").content_slot(Self::preview_card(
                &settings,
                &merge_preview,
                merge_preview_source,
                radius,
                lanes,
                &theme,
            ))],
        )
        .title(tr("map_merge_preview_title").to_uppercase());
        let preview_group =
            if is_truncated { preview_group.end_group_title(tr("map_merge_preview_truncated")) } else { preview_group };

        vstack(22.0)
            .w_full()
            .child(
                GroupedBtns::new("map-merge-enabled-group", vec![Self::enabled_row(&settings)])
                    .title(tr("map_merge_title").to_uppercase())
                    .end_group_title(tr("map_merge_enabled_hint")),
            )
            .child(
                GroupedBtns::new(
                    "map-merge-radius-group",
                    vec![GroupedBtn::new("map_merge_radius", "").content_slot(Self::radius_control(
                        &settings, &drafts, radius, lanes,
                    ))],
                )
                .end_group_title(tr("map_merge_radius_hint")),
            )
            .child(
                GroupedBtns::new(
                    "map-merge-lanes-group",
                    vec![GroupedBtn::new("map_merge_lanes", "").content_slot(Self::lanes_control(
                        &settings, &drafts, radius, lanes,
                    ))],
                )
                .end_group_title(tr("map_merge_lanes_hint")),
            )
            .child(preview_group)
            .on_appear("map-merge-settings-section", move |_, cx| preview_merge(radius, lanes, None, false, cx))
            .and_on_disappear(|_, cx| {
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.merge_preview_disappeared(cx));
            })
    }
}

/// a number with its slider and a step either side
/// slider reports every position while it moves and commits once it rests
/// side buttons commit at once
#[derive(IntoElement)]
struct MapMergeStepper {
    id: &'static str,
    title: String,
    value: i64,
    range: RangeInclusive<i64>,
    shows_ticks: bool,
    text: Rc<dyn Fn(i64) -> String>,
    on_draft: Rc<dyn Fn(i64, &mut App)>,
    on_commit: Rc<dyn Fn(i64, &mut App)>,
}

impl MapMergeStepper {
    fn step_button(
        id: ElementId,
        symbol: &str,
        theme: &AppTheme,
        is_enabled: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> PlainButton {
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
}

impl RenderOnce for MapMergeStepper {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let (value, minimum, maximum) = (self.value, *self.range.start(), *self.range.end());
        let clamped = move |next: i64| maximum.min(minimum.max(next));
        let on_draft = self.on_draft;
        let slider_commit = self.on_commit.clone();
        let minus_commit = self.on_commit.clone();
        let plus_commit = self.on_commit;
        let child_id = |name: &'static str| {
            ElementId::NamedChild(std::sync::Arc::new(ElementId::Name(self.id.into())), SharedString::new_static(name))
        };

        vstack(12.0)
            .w_full()
            .child(
                hstack(None)
                    .w_full()
                    .items_baseline()
                    .child(MainText::new(self.title).px(16.0).number_of_lines(1))
                    .child(spacer_min_w(8.0))
                    .child(
                        MainText::new((self.text)(value))
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
                    .child(Self::step_button(child_id("minus"), "minus", &theme, value > minimum, move |_, cx| {
                        minus_commit(clamped(value - 1), cx);
                    }))
                    .child(
                        div().flex_1().h(px(28.0)).child(
                            SteppedSliderUi::new(
                                child_id("rail"),
                                value as f64,
                                (minimum as f64)..=(maximum as f64),
                                1.0,
                                move |next, _, cx| on_draft(clamped(next.round() as i64), cx),
                                move |next, _, cx| slider_commit(clamped(next.round() as i64), cx),
                            )
                            .shows_step_ticks(self.shows_ticks),
                        ),
                    )
                    .child(Self::step_button(child_id("plus"), "plus", &theme, value < maximum, move |_, cx| {
                        plus_commit(clamped(value + 1), cx);
                    })),
            )
            .child(
                hstack(None)
                    .w_full()
                    .child(MainText::new((self.text)(minimum)).px(12.0))
                    .child(spacer())
                    .child(MainText::new((self.text)(maximum)).px(12.0)),
            )
            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
    }
}

/// one tessellated mark of the preview, laid out from the canvas origin
#[derive(Clone)]
enum MapMergePreviewMark {
    Path(gpui::Path<gpui::Pixels>, Hsla),
    Dot { centre: Point<f32>, radius: f32, color: Hsla },
}

/// everything the preview marks are built from, equal inputs reuse the marks
#[derive(Clone, PartialEq)]
struct MapMergePreviewMarksKey {
    preview: MapMergePreview,
    is_merging: bool,
    heat_walks: i64,
    side: f32,
    walk_color: Hsla,
    band_color: Hsla,
}

#[derive(Default)]
struct MapMergePreviewMarks {
    key: Option<MapMergePreviewMarksKey>,
    marks: Rc<Vec<MapMergePreviewMark>>,
    pending: Option<MapMergePreviewMarksKey>,
    build: Option<Task<()>>,
}

impl MapMergePreviewMarks {
    fn request(&mut self, key: MapMergePreviewMarksKey, cx: &mut gpui::Context<Self>) {
        if self.key.as_ref() == Some(&key) {
            self.pending = None;
            self.build = None;
            return;
        }
        if self.pending.as_ref() == Some(&key) {
            return;
        }
        self.pending = Some(key.clone());
        self.build = Some(cx.spawn(async move |this, cx| {
            let built_key = key.clone();
            let marks = cx.background_spawn(async move { MapMergePreviewCanvas::marks(&built_key) }).await;
            let _ = this.update(cx, |cache, cx| {
                if cache.pending.as_ref() != Some(&key) {
                    return;
                }
                cache.marks = Rc::new(marks);
                cache.key = Some(key);
                cache.pending = None;
                cx.notify();
            });
        }));
    }
}

/// preview window drawn to scale
/// hitbox as a band around every line
/// walks as recorded under it and lines in the colors the map gives them
#[derive(IntoElement)]
pub struct MapMergePreviewCanvas {
    preview: Option<MapMergePreview>,
    is_merging: bool,
    heat_walks: i64,
}

impl MapMergePreviewCanvas {
    pub fn new(preview: Option<MapMergePreview>, is_merging: bool, heat_walks: i64) -> Self {
        Self { preview, is_merging, heat_walks }
    }

    /// the map's own scale from walked once through the owner's count of walks up to the hottest color
    pub fn colour(passes: i64, heat_walks: i64) -> Hsla {
        let scale = MapCoolingCard::scale();
        let hottest = heat_walks.max(2) as f64;
        let temperature = ((passes as f64 - 1.0) / (hottest - 1.0)).max(0.0).min(1.0);
        scale[(temperature * (scale.len() - 1) as f64).round() as usize]
    }

    fn path(stroke: &MapMergePreviewStroke, side: f32) -> Vec<Point<f32>> {
        stroke.points.iter().map(|point| Point { x: point.x as f32 * side, y: point.y as f32 * side }).collect()
    }

    fn dot(stroke: &MapMergePreviewStroke, radius: f32, side: f32, color: Hsla, marks: &mut Vec<MapMergePreviewMark>) {
        if let Some(first) = stroke.points.first() {
            let centre = Point { x: first.x as f32 * side, y: first.y as f32 * side };
            marks.push(MapMergePreviewMark::Dot { centre, radius, color });
        }
    }

    fn stroke(sub_paths: &[Vec<Point<f32>>], line_width: f32, color: Hsla, marks: &mut Vec<MapMergePreviewMark>) {
        if let Some(path) = EditorOverlayCanvas::round_stroke_path(sub_paths, line_width, Point::default()) {
            marks.push(MapMergePreviewMark::Path(path, color));
        }
    }

    fn marks(key: &MapMergePreviewMarksKey) -> Vec<MapMergePreviewMark> {
        let (preview, side) = (&key.preview, key.side);
        let mut marks = Vec::new();
        let cold = MapCoolingCard::scale()[0];

        if !key.is_merging {
            for walk in &preview.walks {
                if walk.points.len() == 1 {
                    Self::dot(walk, 2.0, side, cold, &mut marks);
                } else {
                    Self::stroke(&[Self::path(walk, side)], 2.0, cold, &mut marks);
                }
            }
            return marks;
        }

        let bands: Vec<Vec<Point<f32>>> = preview
            .lines
            .iter()
            .filter(|line| line.points.len() > 1)
            .map(|line| Self::path(line, side))
            .collect();
        let band_width = (2.0 * preview.radius_m as f64 / preview.window_meters * side as f64).max(1.0) as f32;
        Self::stroke(&bands, band_width, key.band_color.opacity(0.14), &mut marks);

        for walk in preview.walks.iter().filter(|walk| walk.points.len() > 1) {
            Self::stroke(&[Self::path(walk, side)], 1.0, key.walk_color.opacity(0.45), &mut marks);
        }
        for line in &preview.lines {
            let color = Self::colour(line.passes, key.heat_walks);
            if line.points.len() == 1 {
                Self::dot(line, 2.5, side, color, &mut marks);
            } else {
                Self::stroke(&[Self::path(line, side)], 3.0, color, &mut marks);
            }
        }
        marks
    }

    fn paint_mark(mark: &MapMergePreviewMark, origin: Point<gpui::Pixels>, window: &mut Window) {
        match mark {
            MapMergePreviewMark::Path(path, color) => {
                let mut path = path.clone();
                for vertex in &mut path.vertices {
                    vertex.xy_position += origin;
                }
                path.bounds.origin += origin;
                window.paint_path(path, *color);
            }
            MapMergePreviewMark::Dot { centre, radius, color } => {
                let at = point(origin.x + px(centre.x - radius), origin.y + px(centre.y - radius));
                window.paint_quad(
                    fill(Bounds::new(at, size(px(radius * 2.0), px(radius * 2.0))), *color).corner_radii(Corners::all(px(*radius))),
                );
            }
        }
    }
}

impl RenderOnce for MapMergePreviewCanvas {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let walk_color = theme.text.secondary;
        let band_color = theme.primary._100;
        let (preview, is_merging, heat_walks) = (self.preview, self.is_merging, self.heat_walks);
        let cache = window.use_keyed_state(ElementId::Name("map-merge-preview-marks".into()), cx, |_, _| {
            MapMergePreviewMarks::default()
        });

        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let Some(preview) = preview else {
                    return;
                };
                let side = f32::from(bounds.size.width).min(f32::from(bounds.size.height));
                let key = MapMergePreviewMarksKey { preview, is_merging, heat_walks, side, walk_color, band_color };
                let marks = cache.update(cx, |cache, cx| {
                    cache.request(key, cx);
                    cache.marks.clone()
                });
                for mark in marks.iter() {
                    Self::paint_mark(mark, bounds.origin, window);
                }
            },
        )
        .size_full()
    }
}
