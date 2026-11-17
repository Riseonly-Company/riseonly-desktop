use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{
    Anchor, App, AsyncApp, Bounds, Context, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, MouseDownEvent, Pixels, Render, RenderOnce,
    SharedString, Subscription, Task, Window, anchored, deferred, point, white,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal, RiseLimits};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent, TimeZone};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::language_store::LanguageStore;
use rise_i18n::tr;
use rise_theme::avatar_palette::AvatarPalette;
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, Appearance, RiseThemeAppearance, ThemeStore};
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::icon_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::{PageHeaderBackButton, PageHeaderIconButton};
use rise_ui::page_header_ui::PageBesideSplitListKey;
use rise_ui::swiftui::date_picker::{DatePicker, DatePickerComponents};
use rise_ui::swiftui::prelude::*;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{CreatePostIntent, RootRoute};
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::map::components::map_editing_controls::{MapEditingAction, MapEditingActionButton, MapEditingToolbar, map_control_hint};
use crate::modules::map::components::map_editing_session::{MapEditingMode, MapEditingSession};
use crate::modules::map::components::map_journey_canvas::MapJourneyCanvas;
use crate::modules::map::components::map_photo_grid_card::MapPhotoGridCard;
use crate::modules::map::components::pins::map_friend_pin_view::MapPinPalette;
use crate::modules::map::engine::core::rise_map_photo_upload::MapPhotoURL;
use crate::modules::map::engine::core::rise_map_resource_repository::RiseMapFailure;
use crate::modules::map::engine::map_journey_geometry::MapJourneyGeometry;
use crate::modules::map::engine::rise_map_engine_models::RiseMapLoadState;
use crate::modules::map::engine::rise_map_journeys_repository::{MapJourneyPoint, MapJourneySelection};
use crate::modules::map::engine::rise_map_photos_repository::RiseMapOpenGridSnapshot;
use crate::modules::map::engine::rise_map_presentation::RiseMapPresentation;
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapTrackTilesTick;
use crate::modules::map::pages::world_map::world_map_page::WorldMapPage;
use crate::modules::map::shared::map_gallery_scout::MapGalleryDraft;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoGrid;
use crate::modules::map::stores::map_actions::map_tracks_types::MapTrackPeriod;
use crate::modules::map::stores::map_interactions::map_interactions::MapInteractionsStore;
use crate::modules::map::stores::map_services::map_services::MapServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::PlanDateFormatting;

const MILLISECONDS_IN_DAY: i64 = 86_400_000;
const DATE_POPOVER_WIDTH: f32 = 300.0;
const DATE_POPOVER_HEIGHT: f32 = 340.0;
const DATE_POPOVER_GAP: f32 = 6.0;
const DATE_POPOVER_MARGIN: f32 = 8.0;

/// journey composer page, the world map in journey edit mode
pub struct MapJourneyComposerPage {
    page: Entity<WorldMapPage>,
}

impl MapJourneyComposerPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self { page: cx.new(|cx| WorldMapPage::new(Some(MapEditingMode::Journey), true, window, cx)) }
    }
}

impl Render for MapJourneyComposerPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.page.clone()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MapJourneyDateField {
    From,
    Until,
}

pub struct MapJourneyComposerControlsState {
    from: Date,
    until: Date,
    preparing: bool,
    preparation_hint: Option<String>,
    work: Option<Task<()>>,
    selection_id: String,
    open_date_picker: Option<MapJourneyDateField>,
    date_chip_bounds: [Option<Bounds<Pixels>>; 2],
}

fn new_selection_id() -> String {
    Uuid::new_v4().uuid_string().to_lowercase()
}

impl MapJourneyComposerControlsState {
    fn new() -> Self {
        let now = RiseClock::now();
        Self {
            from: Calendar::current().date_by_adding(CalendarComponent::Day, -7, &now).unwrap_or(now),
            until: now,
            preparing: false,
            preparation_hint: None,
            work: None,
            selection_id: new_selection_id(),
            open_date_picker: None,
            date_chip_bounds: [None, None],
        }
    }

    fn earliest_start(&self) -> Date {
        let days = -(MapJourneyComposerControls::max_period_ms() / MILLISECONDS_IN_DAY - 1);
        Calendar::current().date_by_adding(CalendarComponent::Day, days, &self.until).unwrap_or(self.until)
    }

    fn latest_end(&self) -> Date {
        let days = MapJourneyComposerControls::max_period_ms() / MILLISECONDS_IN_DAY - 1;
        RiseClock::now().min(Calendar::current().date_by_adding(CalendarComponent::Day, days, &self.from).unwrap_or(self.from))
    }
}

#[derive(IntoElement)]
pub struct MapJourneyComposerControls {
    session: Entity<MapEditingSession>,
    state: Entity<MapJourneyComposerControlsState>,
    on_close: MapEditingAction,
}

impl MapJourneyComposerControls {
    pub fn new(
        session: Entity<MapEditingSession>,
        state: Entity<MapJourneyComposerControlsState>,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { session, state, on_close: Rc::new(on_close) }
    }

    pub fn composer_state(window: &mut Window, cx: &mut App) -> Entity<MapJourneyComposerControlsState> {
        window.use_keyed_state("map-journey-composer-state", cx, |_, _| MapJourneyComposerControlsState::new())
    }

    /// escape closes an open date popover first
    /// then does the same as the cancel or reset button
    pub fn escape(
        state: &Entity<MapJourneyComposerControlsState>,
        session: &Entity<MapEditingSession>,
        on_close: &MapEditingAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        if state.read(cx).open_date_picker.is_some() {
            state.update(cx, |state, cx| {
                state.open_date_picker = None;
                cx.notify();
            });
            return;
        }
        Self::cancel_or_reset(state, session, on_close, window, cx);
    }

    fn cancel_or_reset(
        state: &Entity<MapJourneyComposerControlsState>,
        session: &Entity<MapEditingSession>,
        on_close: &MapEditingAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.update(cx, |state, cx| {
            state.work = None;
            state.selection_id = new_selection_id();
            state.preparing = false;
            state.preparation_hint = None;
            cx.notify();
        });
        let is_drawing = session.read(cx).drawing();
        session.update(cx, |session, cx| {
            session.set_saving(false);
            if !is_drawing {
                session.reset_outline();
            }
            cx.notify();
        });
        if is_drawing {
            on_close(window, cx);
        }
    }

    fn max_grids() -> i64 {
        RiseLimits::service_value("map_journey_max_grids")
    }

    fn max_media() -> i64 {
        RiseLimits::service_value("map_journey_max_media")
    }

    fn max_period_ms() -> i64 {
        RiseLimits::service_value("map_journey_max_period_ms")
    }

    fn refresh_candidates(
        state: &Entity<MapJourneyComposerControlsState>,
        session: &Entity<MapEditingSession>,
        cx: &mut App,
    ) {
        if state.read(cx).preparing {
            return;
        }
        let grids = MapServicesStore::photos(cx).grids;
        let owner = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let outline = session.read(cx).polygon.clone();
        let task_state = state.clone();
        let task_session = session.clone();
        let task = cx.spawn(async move |cx: &mut AsyncApp| {
            let task_owner = owner.clone();
            let computing = cx.background_spawn(async move {
                let own: Vec<MapPhotoGrid> = grids
                    .into_iter()
                    .filter(|grid| task_owner.as_deref() == Some(grid.owner_id.as_str()) && !grid.is_draft && !grid.is_pending)
                    .collect();
                let candidates: Vec<MapPhotoGrid> = if outline.is_empty() {
                    Vec::new()
                } else {
                    own.iter()
                        .filter(|grid| {
                            MapJourneyGeometry::contains(&MapJourneyPoint { lat: grid.latitude, lng: grid.longitude }, &outline)
                        })
                        .cloned()
                        .collect()
                };
                (own, candidates)
            });
            let (own, candidates) = computing.await;
            cx.update(|cx| {
                let current_owner = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
                if !task_session.read(cx).is_current() || owner != current_owner {
                    return;
                }
                let candidate_ids: HashSet<String> = candidates.iter().map(|grid| grid.id.clone()).collect();
                let selection_changed = task_session.update(cx, |session, cx| {
                    session.own_grids = own;
                    session.candidates = candidates;
                    session.photo_revision = session.photo_revision.wrapping_add(1);
                    let kept: HashSet<String> = session.selected.intersection(&candidate_ids).cloned().collect();
                    let changed = kept != session.selected;
                    if changed {
                        session.set_selected(kept);
                    }
                    cx.notify();
                    changed
                });
                if selection_changed {
                    task_state.update(cx, |state, cx| {
                        state.selection_id = new_selection_id();
                        cx.notify();
                    });
                }
            });
        });
        state.update(cx, |state, _| state.work = Some(task));
    }

    fn toggle(
        id: &str,
        state: &Entity<MapJourneyComposerControlsState>,
        session: &Entity<MapEditingSession>,
        cx: &mut App,
    ) {
        let in_candidates = session.read(cx).candidates.iter().any(|grid| grid.id == id);
        if state.read(cx).preparing || !in_candidates {
            return;
        }
        let mut hint = None;
        let mut proceeds = true;
        session.update(cx, |session, cx| {
            if session.selected.contains(id) {
                let mut selected = session.selected.clone();
                selected.remove(id);
                session.set_selected(selected);
            } else if i64::try_from(session.selected.len()).unwrap_or(i64::MAX) < Self::max_grids() {
                let count: i64 = session
                    .candidates
                    .iter()
                    .filter(|grid| session.selected.contains(&grid.id) || grid.id == id)
                    .map(|grid| grid.photo_count)
                    .sum();
                if count > Self::max_media() {
                    hint = Some(tr("map_journey_media_limit").replace("{{limit}}", &Self::max_media().to_string()));
                    proceeds = false;
                    return;
                }
                let mut selected = session.selected.clone();
                selected.insert(id.to_owned());
                session.set_selected(selected);
            } else {
                hint = Some(tr("map_journey_limit"));
            }
            cx.notify();
        });
        state.update(cx, |state, cx| {
            state.preparation_hint = hint;
            if proceeds {
                state.selection_id = new_selection_id();
            }
            cx.notify();
        });
    }

    fn prepare(
        state: &Entity<MapJourneyComposerControlsState>,
        session: &Entity<MapEditingSession>,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        if state.read(cx).preparing || session.read(cx).polygon.is_empty() {
            return;
        }
        let (from, until, request_id) = {
            let state = state.read(cx);
            (state.from, state.until, state.selection_id.clone())
        };
        let calendar = Calendar::current();
        let first_day = calendar.start_of_day(&from);
        let last_day = calendar.start_of_day(&until);
        let day_after = calendar.date_by_adding(CalendarComponent::Day, 1, &last_day).unwrap_or(until);
        let milliseconds = |date: &Date| (date.time_interval_since_1970() * 1000.0) as i64;
        let start = milliseconds(&first_day);
        let max_period_ms = Self::max_period_ms();
        let end = (start + max_period_ms).min(milliseconds(&RiseClock::now())).min(milliseconds(&day_after) - 1);
        if end <= start || end - start > max_period_ms {
            state.update(cx, |state, cx| {
                state.preparation_hint = Some(tr("map_journey_period_limit"));
                cx.notify();
            });
            return;
        }
        let selection = {
            let session = session.read(cx);
            let mut grid_ids: Vec<String> = session.selected.iter().cloned().collect();
            grid_ids.sort();
            MapJourneySelection {
                id: request_id.clone(),
                polygon: session.polygon.clone(),
                grid_ids,
                from_ms: start,
                until_ms: end,
            }
        };
        session.update(cx, |session, cx| {
            session.set_saving(true);
            cx.notify();
        });
        let preparing = MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.prepare_journey(selection, cx));
        let task_state = state.clone();
        let task_session = session.clone();
        let task = cx.spawn(async move |cx: &mut AsyncApp| {
            let result = preparing.await;
            cx.update(|cx| {
                let is_requested = task_state.read(cx).selection_id == request_id;
                if is_requested {
                    task_state.update(cx, |state, cx| {
                        state.preparing = false;
                        if let (Err(error), true) = (&result, task_session.read(cx).is_current()) {
                            state.preparation_hint = Some(RiseMapFailure::journey_text(error));
                        }
                        cx.notify();
                    });
                    task_session.update(cx, |session, cx| {
                        session.set_saving(false);
                        cx.notify();
                    });
                }
                let Ok(snapshot) = result else {
                    return;
                };
                if !task_session.read(cx).is_current() || !is_requested {
                    return;
                }
                if let Some(nav_store) = nav_store {
                    nav_store.update(cx, |nav_store, cx| {
                        nav_store.push(
                            RootRoute::CreatePostMedia {
                                intent: CreatePostIntent::Publication,
                                track_seed: None,
                                journey_id: Some(snapshot.id),
                            },
                            NavigationStoreSettlingPush::Drop,
                            cx,
                        );
                    });
                }
            });
        });
        state.update(cx, |state, cx| {
            state.preparing = true;
            state.preparation_hint = None;
            state.work = Some(task);
            cx.notify();
        });
    }

    /// compact date row
    /// the title and the date as a button and the calendar in a popover
    fn date_row(
        title: String,
        field: MapJourneyDateField,
        range: std::ops::RangeInclusive<Date>,
        state: &Entity<MapJourneyComposerControlsState>,
        tint: Hsla,
        theme: &AppTheme,
        viewport_height: Pixels,
        cx: &App,
    ) -> Div {
        let (date, is_open, preparing, chip_bounds) = {
            let state = state.read(cx);
            let date = if field == MapJourneyDateField::From { state.from } else { state.until };
            (date, state.open_date_picker == Some(field), state.preparing, state.date_chip_bounds[field as usize])
        };
        let toggle_state = state.clone();
        let close_state = state.clone();
        let chip_state = state.clone();
        let selection = Binding::entity(
            state.clone(),
            move |state: &MapJourneyComposerControlsState| {
                if field == MapJourneyDateField::From { state.from } else { state.until }
            },
            move |state: &mut MapJourneyComposerControlsState, value: Date, _| {
                if field == MapJourneyDateField::From {
                    state.from = value;
                } else {
                    state.until = value;
                }
            },
        );
        let id = if field == MapJourneyDateField::From { "map-journey-from" } else { "map-journey-until" };
        let (popover_background, popover_border, chip) = (theme.bg._100, theme.border._100, theme.bg._300);
        hstack(None)
            .w_full()
            .child(MainText::new(title))
            .child(spacer_min_w(0.0))
            .child(
                plain(id)
                    .disabled(preparing)
                    .on_press(move |_, cx| {
                        toggle_state.update(cx, |state, cx| {
                            state.open_date_picker = if state.open_date_picker == Some(field) { None } else { Some(field) };
                            cx.notify();
                        });
                    })
                    .px(px(10.0))
                    .py(px(6.0))
                    .bg(chip)
                    .rounded(px(8.0))
                    .relative()
                    .child(MainText::new(PlanDateFormatting::display(PlanDateFormatting::ms(date))).px(14.0))
                    .child(measure("map-journey-date-chip", move |bounds, _, cx| {
                        chip_state.update(cx, |state, cx| {
                            state.date_chip_bounds[field as usize] = Some(bounds);
                            if state.open_date_picker == Some(field) {
                                cx.notify();
                            }
                        });
                    })),
            )
            .when_some(chip_bounds.filter(|_| is_open), |row, chip_bounds| {
                let below = chip_bounds.bottom() + px(DATE_POPOVER_GAP + DATE_POPOVER_HEIGHT + DATE_POPOVER_MARGIN) <= viewport_height;
                let (anchor, position) = if below {
                    (Anchor::TopRight, point(chip_bounds.right(), chip_bounds.bottom() + px(DATE_POPOVER_GAP)))
                } else {
                    (Anchor::BottomRight, point(chip_bounds.right(), chip_bounds.top() - px(DATE_POPOVER_GAP)))
                };
                row.child(deferred(
                    anchored().anchor(anchor).position(position).snap_to_window_with_margin(px(DATE_POPOVER_MARGIN)).child(
                        div()
                            .occlude()
                            .w(px(DATE_POPOVER_WIDTH))
                            .p(px(12.0))
                            .bg(popover_background)
                            .border_1()
                            .border_color(popover_border)
                            .rounded(px(12.0))
                            .on_mouse_down_out(move |event: &MouseDownEvent, _, cx| {
                                let on_chip = close_state.read(cx).date_chip_bounds[field as usize]
                                    .is_some_and(|bounds| bounds.contains(&event.position));
                                if on_chip {
                                    return;
                                }
                                close_state.update(cx, |state, cx| {
                                    state.open_date_picker = None;
                                    cx.notify();
                                });
                            })
                            .child(
                                DatePicker::new((id, 1_usize), selection)
                                    .range(range)
                                    .displayed_components(DatePickerComponents::Date)
                                    .tint(tint),
                            ),
                    ),
                ))
            })
    }

    fn grid_cell(
        grid: &MapPhotoGrid,
        is_selected: bool,
        state: &Entity<MapJourneyComposerControlsState>,
        session: &Entity<MapEditingSession>,
    ) -> impl IntoElement {
        let id = grid.id.clone();
        let press_state = state.clone();
        let press_session = session.clone();
        let image = MapPhotoURL::absolute(&grid.cover_url).map(|source| {
            CleverImageUi::new(source)
                .content_mode(ContentMode::Fill)
                .corner_radius(12.0)
                .persist_to_disk(false)
                .maximum_decode_pixel_dimension(256_i64)
                .cache_policy(CleverImageCachePolicy::MemoryOnly)
        });
        let symbol = if is_selected { "checkmark.circle.fill" } else { "circle" };
        let name = SharedString::from(format!("map-journey-grid-{}", grid.id));
        map_control_hint(
            SharedString::from(format!("{name}-hint")),
            tr("map_journey_select_grid"),
            plain(name)
                .on_press(move |_, cx| Self::toggle(&id, &press_state, &press_session, cx))
                .child(
                    div()
                        .relative()
                        .size(px(64.0))
                        .overflow_hidden()
                        .children(image)
                        .child(
                            div()
                                .absolute()
                                .top(px(5.0))
                                .right(px(5.0))
                                .children(IconUi::sized(symbol, px(17.0), white(), false)),
                        ),
                ),
        )
    }
}

impl RenderOnce for MapJourneyComposerControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { session, state, on_close } = self;
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let grids = MapServicesStore::photos(cx).grids;
        let period = MapServicesStore::tracks(cx).period;
        let (polygon, selected, candidates, drawing) = {
            let session = session.read(cx);
            (session.polygon.clone(), session.selected.clone(), session.candidates.clone(), session.drawing())
        };
        let (from, until, preparing, preparation_hint, earliest_start, latest_end) = {
            let state = state.read(cx);
            (
                state.from,
                state.until,
                state.preparing,
                state.preparation_hint.clone(),
                state.earliest_start(),
                state.latest_end(),
            )
        };

        Self::watch_inputs(&state, &session, grids, polygon, period, &selected, from, until, window, cx);

        let mut column = vstack(10.0).w_full();
        if !drawing {
            let viewport_height = window.viewport_size().height;
            let dates = div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(Self::date_row(
                    tr("map_journey_from"),
                    MapJourneyDateField::From,
                    earliest_start..=until,
                    &state,
                    theme.primary._100,
                    &theme,
                    viewport_height,
                    cx,
                ))
                .child(Self::date_row(
                    tr("map_journey_until"),
                    MapJourneyDateField::Until,
                    from..=latest_end,
                    &state,
                    theme.primary._100,
                    &theme,
                    viewport_height,
                    cx,
                ))
                .guide_anchor(RiseGuideAnchor::MapJourneyDates);
            let grids = (!candidates.is_empty()).then(|| {
                let cells: Vec<AnyElement> = candidates
                    .iter()
                    .map(|grid| {
                        div()
                            .flex_shrink_0()
                            .child(Self::grid_cell(grid, selected.contains(&grid.id), &state, &session))
                            .into_any_element()
                    })
                    .collect();
                div().relative().w_full().h(px(68.0)).child(
                    div()
                        .id("map-journey-grids")
                        .absolute()
                        .inset_0()
                        .flex()
                        .gap(px(10.0))
                        .overflow_x_scroll()
                        .children(cells),
                )
                .guide_anchor(RiseGuideAnchor::MapJourneyGrids)
            });
            let create_state = state.clone();
            let create_session = session.clone();
            let create_nav = nav_store.clone();
            let mut create_label = hstack(None);
            if preparing {
                create_label = create_label.child(progress_view());
            }
            let create_label =
                create_label.child(MainText::new(tr("map_journey_create_post")).px(16.0).font_weight(FontWeight::SEMIBOLD));
            column = column.child(div().id("map-journey-card").w_full().occlude().child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .p(px(14.0))
                    .child(dates)
                    .children(grids)
                    .child(
                        MainText::new(format!("{} / {}", selected.len(), Self::max_grids()))
                            .px(12.0)
                            .color(theme.text.secondary),
                    )
                    .child(MainText::new(tr("map_journey_audience")).px(12.0).color(theme.text.secondary))
                    .children(preparation_hint.map(|hint| MainText::new(hint).px(13.0).color(theme.text.secondary)))
                    .child(
                        MapEditingActionButton::new(
                            "map-journey-create",
                            move |_, cx| Self::prepare(&create_state, &create_session, create_nav.clone(), cx),
                            create_label,
                        )
                        .primary(true)
                        .disabled(preparing)
                        .guide_anchor(RiseGuideAnchor::MapJourneyCreate),
                    )
                    .liquid_chrome_rounded_rectangle(24.0),
            ));
        }
        let reset_state = state.clone();
        let reset_session = session.clone();
        let disappear_state = state.clone();
        let disappear_session = session.clone();
        column
            .child(MapEditingToolbar::new(session.clone()))
            .child(
                div().w_full().child(MapEditingActionButton::new(
                    "map.edit.cancelOrReset",
                    move |window, cx| Self::cancel_or_reset(&reset_state, &reset_session, &on_close, window, cx),
                    MainText::new(tr(if drawing { "common_cancel" } else { "map_edit_reset" }))
                        .px(16.0)
                        .font_weight(FontWeight::SEMIBOLD),
                )),
            )
            .on_disappear("map-journey-composer-lifecycle", move |_, cx| {
                disappear_state.update(cx, |state, cx| {
                    state.work = None;
                    state.selection_id = new_selection_id();
                    state.preparing = false;
                    cx.notify();
                });
                disappear_session.update(cx, |session, cx| {
                    session.set_saving(false);
                    cx.notify();
                });
                GuideFacts::set(RiseGuideSignal::MapJourneyGridSelected, false, cx);
            })
    }
}

impl MapJourneyComposerControls {
    #[allow(clippy::too_many_arguments)]
    fn watch_inputs(
        state: &Entity<MapJourneyComposerControlsState>,
        session: &Entity<MapEditingSession>,
        grids: Vec<MapPhotoGrid>,
        polygon: Vec<MapJourneyPoint>,
        period: MapTrackPeriod,
        selected: &HashSet<String>,
        from: Date,
        until: Date,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (grids_state, grids_session) = (state.clone(), session.clone());
        on_change("map-journey-photos", grids, true, window, cx, move |_, _, _, cx| {
            Self::refresh_candidates(&grids_state, &grids_session, cx);
        });
        let (polygon_state, polygon_session) = (state.clone(), session.clone());
        on_change("map-journey-polygon", polygon, false, window, cx, move |_, _, _, cx| {
            polygon_state.update(cx, |state, cx| {
                state.work = None;
                state.preparing = false;
                state.preparation_hint = None;
                state.selection_id = new_selection_id();
                cx.notify();
            });
            Self::refresh_candidates(&polygon_state, &polygon_session, cx);
        });
        let period_state = state.clone();
        on_change("map-journey-period", period, true, window, cx, move |_, period, _, cx| {
            if period_state.read(cx).preparing {
                return;
            }
            let tracks = MapServicesStore::tracks(cx);
            let day = tracks.summary.as_ref().and_then(|summary| summary.stats(*period)).map_or(0, |stats| stats.since_day);
            period_state.update(cx, |state, cx| {
                let earliest = state.earliest_start();
                state.from = if day > 0 {
                    let offset = TimeZone::current().seconds_from_gmt(&RiseClock::now());
                    let start = Date::from_time_interval_since_1970(f64::from(day) * 86_400.0 - f64::from(offset));
                    earliest.max(state.until.min(start))
                } else {
                    earliest
                };
                cx.notify();
            });
        });
        let selected_state = state.clone();
        on_change("map-journey-selected", selected.clone(), false, window, cx, move |_, _, _, cx| {
            selected_state.update(cx, |state, cx| {
                state.selection_id = new_selection_id();
                state.preparation_hint = None;
                cx.notify();
            });
        });
        on_change("map-journey-selected-empty", selected.is_empty(), true, window, cx, |_, empty, _, cx| {
            GuideFacts::set(RiseGuideSignal::MapJourneyGridSelected, !*empty, cx);
        });
        for (id, value) in [("map-journey-from-change", from), ("map-journey-until-change", until)] {
            let dates_state = state.clone();
            on_change(id, value, false, window, cx, move |_, _, _, cx| {
                dates_state.update(cx, |state, cx| {
                    state.selection_id = new_selection_id();
                    state.preparation_hint = None;
                    cx.notify();
                });
            });
        }
    }
}

pub struct MapJourneyPage {
    post_id: i64,
    selected_grid_id: Option<String>,
    scope: Uuid,
    focus_handle: FocusHandle,
    _observations: Vec<Subscription>,
}

impl MapJourneyPage {
    pub fn new(post_id: i64, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let journey = MapServicesStore::journey(cx);
        let state = RiseMapPresentation::shared(cx).read(cx).state.clone();
        let observations = vec![
            cx.observe(&journey, |_, _, cx| cx.notify()),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        Self {
            post_id,
            selected_grid_id: None,
            scope: Uuid::new_v4(),
            focus_handle: cx.focus_handle(),
            _observations: observations,
        }
    }
}

impl Render for MapJourneyPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (post_id, scope) = (self.post_id, self.scope);
        let page = cx.entity();
        let nav_store = env::<NavigationStoreKey>(cx);
        let theme = rise_ui::theme(cx).clone();
        let is_member = MapServicesStore::is_member(cx);
        let journey = MapServicesStore::journey(cx);
        let (snapshot, grids, photos_by_grid, heated_lines, loading, error) = {
            let presentation = journey.read(cx);
            let snapshot = if presentation.scope == Some(scope) && presentation.post_id == Some(post_id) {
                presentation.snapshot.clone()
            } else {
                None
            };
            let grids = if snapshot.is_none() { Vec::new() } else { presentation.grids.clone() };
            (
                snapshot,
                grids,
                self.selected_grid_id
                    .as_ref()
                    .and_then(|id| presentation.photos_by_grid.get(id).map(|photos| (id.clone(), photos.clone())))
                    .into_iter()
                    .collect::<HashMap<_, _>>(),
                presentation.heated_lines.clone(),
                presentation.loading,
                presentation.error.clone(),
            )
        };
        let selected = grids.iter().find(|grid| Some(&grid.id) == self.selected_grid_id.as_ref()).cloned();
        let has_snapshot = snapshot.is_some();
        let is_owner = snapshot.as_ref().is_some_and(|snapshot| {
            AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().as_deref() == Some(snapshot.owner_id.as_str())
        });
        let snapshot_id = snapshot.as_ref().map(|snapshot| snapshot.id.clone());

        use_task("map-journey-page-open", post_id, window, cx, async move |cx| {
            let cx: &mut AsyncApp = cx;
            let opening = cx.update(|cx| {
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.open_journey(post_id, scope, cx))
            });
            opening.await;
        });

        let canvas = snapshot.map(|snapshot| {
            let grid_page = page.clone();
            let language_code = LanguageStore::shared(cx).code().to_owned();
            let appearance = ThemeStore::shared(cx).read(cx).resolved_appearance();
            let mut canvas = MapJourneyCanvas::new(
                "map-journey-canvas",
                appearance,
                language_code,
                journey_pin_palette(&theme, appearance),
                false,
                Vec::new(),
                Some(snapshot),
                grids.clone(),
                HashSet::new(),
                None,
                RiseMapTrackTilesTick::idle(),
                Rc::new(|_: Vec<MapJourneyPoint>, _: &mut Window, _: &mut App| {}),
                Rc::new(|_: &mut Window, _: &mut App| {}),
                Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                    grid_page.update(cx, |page, cx| {
                        page.selected_grid_id = Some(id);
                        cx.notify();
                    });
                }),
            );
            canvas.heated_lines = heated_lines;
            div().absolute().inset_0().child(canvas)
        });

        let back_nav = nav_store.clone();
        let mut header = hstack(None).w_full().p(px(16.0));
        if !env::<PageBesideSplitListKey>(cx) {
            header = header.child(PageHeaderBackButton::new("map-journey-back", move |_, cx| {
                if let Some(nav_store) = &back_nav {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
            }));
        }
        header = header.child(spacer());
        if is_owner {
            let revoke_nav = nav_store.clone();
            let revoke_id = snapshot_id.clone();
            header = header.child(
                PageHeaderIconButton::new("map-journey-revoke", "trash", move |_, cx| {
                    let revoke_nav = revoke_nav.clone();
                    let revoke_id = revoke_id.clone();
                    NativeConfirmAlert::present_delete(
                        &tr("map_journey_revoke"),
                        "",
                        Some(tr("map_journey_revoke")),
                        None,
                        false,
                        None,
                        None,
                        move |cx| {
                            let Some(id) = revoke_id else {
                                return;
                            };
                            let revoking = MapInteractionsStore::shared(cx)
                                .update(cx, |interactions, cx| interactions.revoke_journey(id, cx));
                            cx.spawn(async move |cx: &mut AsyncApp| {
                                let result = revoking.await;
                                cx.update(|cx| match result {
                                    Ok(()) => {
                                        if let Some(nav_store) = &revoke_nav {
                                            nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                                        }
                                    }
                                    Err(_) => {
                                        let options = NotifierSendOptions {
                                            content: Some(tr("map_journey_unavailable")),
                                            r#type: NotifierType::Error,
                                            ..NotifierSendOptions::default()
                                        };
                                        Notifier::send(tr("map_journey_title"), options, cx);
                                    }
                                });
                            })
                            .detach();
                        },
                        cx,
                    );
                })
                .font_size(18.0),
            );
        }

        let mut column = div().absolute().inset_0().flex().flex_col().items_center().child(header);
        if loading {
            column = column.child(spacer()).child(progress_view()).child(spacer());
        } else if let Some(error) = error {
            column = column
                .child(spacer())
                .child(MainText::new(error).px(16.0))
                .child(
                    plain("map-journey-retry")
                        .on_press(move |_, cx| {
                            MapInteractionsStore::shared(cx)
                                .update(cx, |interactions, cx| interactions.open_journey(post_id, scope, cx))
                                .detach();
                        })
                        .child(MainText::new(tr("map_journey_retry")).color(theme.primary._100)),
                )
                .child(spacer());
        } else {
            column = column.child(spacer());
        }
        if let (Some(selected), true) = (selected, has_snapshot) {
            let close_page = page.clone();
            let open = RiseMapOpenGridSnapshot {
                grid_id: selected.id.clone(),
                photos: photos_by_grid.get(&selected.id).cloned().unwrap_or_default(),
                has_more: false,
                load_state: RiseMapLoadState::Loaded,
            };
            column = column.child(div().w_full().p(px(12.0)).child(MapPhotoGridCard {
                grid: selected,
                open: Some(open),
                drafts: Vec::new(),
                publishing_drafts: HashSet::new(),
                uploads: Vec::new(),
                is_own: false,
                upload_progress: None,
                on_add: Rc::new(|_: &mut Window, _: &mut App| {}),
                on_delete_photo: Rc::new(|_: String, _: &mut App| {}),
                on_delete_grid: Rc::new(|_: &mut App| {}),
                on_publish_drafts: Rc::new(|_: Vec<MapGalleryDraft>, _: &mut App| {}),
                on_discard_drafts: Rc::new(|_: Vec<String>, _: &mut App| {}),
                on_discard_all_drafts: Rc::new(|_: &mut App| {}),
                on_cancel_upload: Rc::new(|_: String, _: &mut Window, _: &mut App| {}),
                on_load_more: Rc::new(|_: &mut Window, _: &mut App| {}),
                on_retry: Rc::new(|_: &mut Window, _: &mut App| {}),
                on_close: Rc::new(move |_: &mut Window, cx: &mut App| {
                    close_page.update(cx, |page, cx| {
                        page.selected_grid_id = None;
                        cx.notify();
                    });
                }),
                on_edit: None,
                docked: false,
            }));
        }
        if !is_member && has_snapshot {
            column = column.child(
                div().p(px(12.0)).child(
                    plain("map-journey-join")
                        .on_press(|_, cx| {
                            MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.present_onboarding(cx));
                        })
                        .child(
                            div()
                                .p(px(16.0))
                                .child(MainText::new(tr("map_journey_join")).color(theme.primary._100))
                                .liquid_chrome_rounded_rectangle(22.0),
                        ),
                ),
            );
        }

        let page_focus = PageFocus::keyed_with("map-journey-page-focus", self.focus_handle.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        let key_nav = nav_store.clone();
        let key_page = page.clone();
        div()
            .id("map-journey-page")
            .relative()
            .size_full()
            .bg(theme.bg._000)
            .track_focus(&self.focus_handle)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != "escape" {
                    return;
                }
                let has_grid = key_page.read(cx).selected_grid_id.is_some();
                if has_grid {
                    key_page.update(cx, |page, cx| {
                        page.selected_grid_id = None;
                        cx.notify();
                    });
                } else if let Some(nav_store) = &key_nav {
                    nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
                }
                cx.stop_propagation();
            })
            .children(canvas)
            .child(column)
            .on_appear("map-journey-page-lifecycle", move |window, cx| PageFocus::did_appear(&page_focus, window, cx))
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
                MapInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.close_journey(scope, cx)).detach();
            })
    }
}

pub(crate) fn journey_pin_palette(theme: &AppTheme, appearance: RiseThemeAppearance) -> MapPinPalette {
    let appearance = match appearance {
        RiseThemeAppearance::Light => Appearance::Light,
        RiseThemeAppearance::Dark => Appearance::Dark,
    };
    MapPinPalette {
        primary: theme.primary._100,
        ring: white(),
        stale: theme.text.secondary,
        plate: theme.bg._100.opacity(0.92),
        plate_text: theme.text.primary,
        letter: AvatarPalette::letter_color(),
        low_battery: UIColor::system_red().resolved_color(appearance),
    }
}
