use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::{Range, RangeInclusive};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyView, App, Bounds, Context, ElementId, Entity, FocusHandle, KeyDownEvent, PinchEvent, Pixels, Point,
    Render, RenderOnce, ScrollHandle, ScrollStrategy, ScrollWheelEvent, Size, StyleRefinement, Subscription, Task,
    TouchPhase, UniformListScrollHandle, WeakEntity, Window, point, size, uniform_list,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::AppDateFormatter;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_platform::haptics::selection_changed;
use rise_platform::photo_library::{ImageRequestId, LibraryAsset, LibraryAssetKind};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::MEDIA_PICKER_GRID_COLUMNS;
use rise_theme::ThemeStore;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::swiftui::prelude::*;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::media_viewer::media_viewer_types::MediaViewerFrameKeys;
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::modules::camera::capture::rise_camera_presenter::RiseCameraPresenter;
use crate::modules::media_picker::editor::editor_edit_preview_cache::MEDIA_PICKER_EDITS_DID_CHANGE;
use crate::modules::media_picker::library::photo_library_fetcher::{PhotoLibraryFetcher, PhotoLibraryFetcherDelegate};
use crate::modules::media_picker::selection::media_selection_state::MediaSelectionState;
use crate::modules::media_picker::shared::media_picker_performance_diagnostics::{
    MediaPickerPerformanceDiagnostics, MediaPickerScrollPerformanceSampler,
};
use crate::modules::media_picker::ui::media_picker_camera_cell::MediaPickerCameraCell;
use crate::modules::media_picker::ui::media_picker_date_scrubber::{
    MediaPickerDateScrubber, MediaPickerDateScrubberPan,
};
use crate::modules::media_picker::ui::media_picker_grid_cell::{
    MediaPickerGridCell, MediaPickerGridCellAction, MediaPickerGridCellConfiguration,
};

const GRID_SPACING: f32 = 1.0;
const LIBRARY_TOP_SLACK: f32 = 4.0;
const DATE_SCRUBBER_WIDTH: f32 = 210.0;
const SETTLE_DELAY_SECONDS: f64 = 0.18;
const VIEWPORT_RESIZE_SECONDS: f64 = 0.10;
const SCRUB_COMMIT_SECONDS: f64 = 0.05;
const DATE_SCRUBBER_UPDATE_SECONDS: f64 = 1.0 / 30.0;
const NORMAL_DECELERATION_RATE: f32 = 0.998;
const WHEEL_QUIET_SECONDS: f64 = 0.12;
const WHEEL_EVENTS_PER_SECOND: f32 = 60.0;
const FADED_HANDLE_PAN_HYSTERESIS: f32 = 10.0;

pub type MediaPickerGridAssetAction = Rc<dyn Fn(&LibraryAsset, &mut Window, &mut App)>;
pub type MediaPickerGridPreviewAction = Rc<dyn Fn(&LibraryAsset, i64, Option<Bounds<Pixels>>, &mut Window, &mut App)>;

pub const MEDIA_PICKER_REFRESH_CELL_FRAMES: RiseNotificationName =
    RiseNotificationName::new("mediaPickerRefreshCellFrames");
pub const MEDIA_PICKER_REVEAL_ASSET: RiseNotificationName = RiseNotificationName::new("mediaPickerRevealAsset");

pub enum MediaPickerRevealAssetKey {}

impl MediaPickerRevealAssetKey {
    pub const INDEX: &'static str = "index";
}

/// asset local id and reveal index of the reveal notification as one typed payload
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaPickerRevealAssetPayload {
    pub local_id: String,
    pub index: i64,
}

#[derive(IntoElement)]
pub struct MediaPickerGridView {
    id: ElementId,
    pub fetcher: Entity<PhotoLibraryFetcher>,
    pub selection: Entity<MediaSelectionState>,
    pub theme_store: Entity<ThemeStore>,
    pub on_pick_asset: MediaPickerGridAssetAction,
    pub on_toggle_selection: MediaPickerGridAssetAction,
    pub on_preview_asset: MediaPickerGridPreviewAction,
    pub refresh_trigger: Uuid,
    pub hero_lift_id: Option<String>,
    pub hero_cell_hidden: bool,
    pub shows_selection_controls: bool,
    pub keeps_stable_maximum_viewport: bool,
    pub top_content_inset: f32,
    pub bottom_content_inset: f32,
    pub on_camera_tile: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl MediaPickerGridView {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        fetcher: Entity<PhotoLibraryFetcher>,
        selection: Entity<MediaSelectionState>,
        theme_store: Entity<ThemeStore>,
        on_pick_asset: impl Fn(&LibraryAsset, &mut Window, &mut App) + 'static,
        on_toggle_selection: impl Fn(&LibraryAsset, &mut Window, &mut App) + 'static,
        on_preview_asset: impl Fn(&LibraryAsset, i64, Option<Bounds<Pixels>>, &mut Window, &mut App) + 'static,
        refresh_trigger: Uuid,
        hero_lift_id: Option<String>,
        hero_cell_hidden: bool,
    ) -> Self {
        Self {
            id: id.into(),
            fetcher,
            selection,
            theme_store,
            on_pick_asset: Rc::new(on_pick_asset),
            on_toggle_selection: Rc::new(on_toggle_selection),
            on_preview_asset: Rc::new(on_preview_asset),
            refresh_trigger,
            hero_lift_id,
            hero_cell_hidden,
            shows_selection_controls: true,
            keeps_stable_maximum_viewport: false,
            top_content_inset: 0.0,
            bottom_content_inset: 120.0,
            on_camera_tile: None,
        }
    }

    pub fn shows_selection_controls(mut self, shows: bool) -> Self {
        self.shows_selection_controls = shows;
        self
    }

    pub fn keeps_stable_maximum_viewport(mut self, keeps: bool) -> Self {
        self.keeps_stable_maximum_viewport = keeps;
        self
    }

    pub fn top_content_inset(mut self, inset: f32) -> Self {
        self.top_content_inset = inset;
        self
    }

    pub fn bottom_content_inset(mut self, inset: f32) -> Self {
        self.bottom_content_inset = inset;
        self
    }

    pub fn on_camera_tile(mut self, on_camera_tile: Option<Rc<dyn Fn(&mut Window, &mut App)>>) -> Self {
        self.on_camera_tile = on_camera_tile;
        self
    }

    fn make_coordinator(&self, signature: u64) -> MediaPickerGridViewCoordinator {
        MediaPickerGridViewCoordinator {
            last_refresh_token: self.refresh_trigger,
            last_selection_signature: signature,
            last_hero_lift_id: self.hero_lift_id.clone(),
            last_hero_hidden: self.hero_cell_hidden,
        }
    }

    fn make_ui_view_controller(&self, cx: &mut Context<MediaPickerGridViewHost>) -> MediaPickerGridViewHost {
        let controller = cx.new(|cx| {
            let mut controller = MediaPickerGridController::new(
                self.fetcher.clone(),
                self.selection.clone(),
                self.theme_store.clone(),
                self.keeps_stable_maximum_viewport,
                cx,
            );
            controller.on_pick_asset = Some(self.on_pick_asset.clone());
            controller.on_toggle_selection = Some(self.on_toggle_selection.clone());
            controller.on_preview_asset = Some(self.on_preview_asset.clone());
            controller.shows_selection_controls = self.shows_selection_controls;
            controller.top_content_inset = self.top_content_inset;
            controller.bottom_content_inset = self.bottom_content_inset;
            controller.on_camera_tile = self.on_camera_tile.clone();
            controller.apply_tile_slots(false, cx);
            controller
        });
        let signature = Self::selection_signature(self.selection.read(cx));
        MediaPickerGridViewHost { controller, coordinator: self.make_coordinator(signature) }
    }

    fn update_ui_view_controller(&self, host: &Entity<MediaPickerGridViewHost>, cx: &mut App) {
        let controller = host.read(cx).controller.clone();
        let signature = Self::selection_signature(self.selection.read(cx));
        let (refresh_changed, selection_changed, hero_changed) = host.update(cx, |host, _| {
            let coordinator = &mut host.coordinator;
            let refresh_changed = coordinator.last_refresh_token != self.refresh_trigger;
            coordinator.last_refresh_token = self.refresh_trigger;
            let selection_changed = coordinator.last_selection_signature != signature;
            coordinator.last_selection_signature = signature;
            let hero_changed = coordinator.last_hero_lift_id != self.hero_lift_id
                || coordinator.last_hero_hidden != self.hero_cell_hidden;
            coordinator.last_hero_lift_id = self.hero_lift_id.clone();
            coordinator.last_hero_hidden = self.hero_cell_hidden;
            (refresh_changed, selection_changed, hero_changed)
        });
        controller.update(cx, |controller, cx| {
            controller.on_pick_asset = Some(self.on_pick_asset.clone());
            controller.on_toggle_selection = Some(self.on_toggle_selection.clone());
            controller.on_preview_asset = Some(self.on_preview_asset.clone());
            controller.set_shows_selection_controls(self.shows_selection_controls, cx);
            controller.set_top_content_inset(self.top_content_inset, cx);
            controller.set_bottom_content_inset(self.bottom_content_inset, cx);
            controller.set_on_camera_tile(self.on_camera_tile.clone(), cx);
            if refresh_changed {
                controller.apply_refresh_trigger(self.refresh_trigger, cx);
            }
            if selection_changed {
                controller.refresh_selection_badges_if_needed(cx);
            }
            if hero_changed {
                controller.apply_hero_lift(self.hero_lift_id.clone(), self.hero_cell_hidden, cx);
            }
        });
    }

    fn selection_signature(selection: &MediaSelectionState) -> u64 {
        let mut hasher = DefaultHasher::new();
        selection.count().hash(&mut hasher);
        selection.draws_spoiler().hash(&mut hasher);
        for id in selection.ordered_identifiers() {
            id.hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl RenderOnce for MediaPickerGridView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = window.use_keyed_state(self.id.clone(), cx, |_, cx| self.make_ui_view_controller(cx));
        self.update_ui_view_controller(&host, cx);
        let controller = host.read(cx).controller.clone();
        div().size_full().child(AnyView::from(controller).cached(StyleRefinement::default().size_full()))
    }
}

/// what the grid view host keeps between passes
struct MediaPickerGridViewHost {
    controller: Entity<MediaPickerGridController>,
    coordinator: MediaPickerGridViewCoordinator,
}

pub struct MediaPickerGridViewCoordinator {
    pub last_refresh_token: Uuid,
    pub last_selection_signature: u64,
    pub last_hero_lift_id: Option<String>,
    pub last_hero_hidden: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScrollPhase {
    Idle,
    Dragging,
    Decelerating,
}

/// observers of view did load, hopped to the main thread
enum MediaPickerGridEvent {
    RefreshCellFrames,
    EditsDidChange,
    RevealAsset(MediaPickerRevealAssetPayload),
}

/// square grid whose first item the camera tile can span two slots
/// below it in portrait, beside it in landscape
/// covered slot lives only in the layout so an item keeps its asset when the column count changes
#[derive(Clone, Copy, PartialEq, Debug)]
struct MediaPickerGridLayout {
    spacing: f32,
    columns: usize,
    has_leading_tile: bool,
    tile_prefers_beside: bool,
}

impl MediaPickerGridLayout {
    fn geometry(&self, requested: usize, width: f32, item_count: usize) -> MediaPickerGridGeometry {
        if width <= 1.0 {
            return MediaPickerGridGeometry::EMPTY;
        }
        let available = width.max(1.0);
        let columns = requested.max(1);
        let side = ((available - self.spacing * (columns - 1) as f32) / columns as f32).floor().max(1.0);
        let pitch_x = if columns > 1 { (available - side) / (columns - 1) as f32 } else { 0.0 };
        let mut covered_slot = None;
        if self.has_leading_tile && columns >= 2 {
            let candidate = if self.tile_prefers_beside { 1 } else { columns };
            if item_count >= candidate {
                covered_slot = Some(candidate);
            }
        }
        let slots = item_count + usize::from(covered_slot.is_some());
        let rows = slots.div_ceil(columns);
        let content_height = if rows > 0 { rows as f32 * side + (rows - 1) as f32 * self.spacing } else { 0.0 };
        MediaPickerGridGeometry { columns, side, spacing: self.spacing, pitch_x, covered_slot, item_count, rows, content_height }
    }
}

/// one column count laid out over the grid items, tile first
#[derive(Clone, Copy, PartialEq, Debug)]
struct MediaPickerGridGeometry {
    columns: usize,
    side: f32,
    spacing: f32,
    pitch_x: f32,
    covered_slot: Option<usize>,
    item_count: usize,
    rows: usize,
    content_height: f32,
}

impl MediaPickerGridGeometry {
    const EMPTY: Self = Self {
        columns: 1,
        side: 1.0,
        spacing: 0.0,
        pitch_x: 0.0,
        covered_slot: None,
        item_count: 0,
        rows: 0,
        content_height: 0.0,
    };

    fn row_pitch(&self) -> f32 {
        self.side + self.spacing
    }

    fn slot(&self, item: usize) -> usize {
        match self.covered_slot {
            Some(covered) if item >= covered => item + 1,
            _ => item,
        }
    }

    /// the item a slot draws, the covered slot answers the tile
    fn item(&self, slot: usize) -> Option<usize> {
        let item = match self.covered_slot {
            Some(covered) if slot == covered => 0,
            Some(covered) if slot > covered => slot - 1,
            _ => slot,
        };
        (item < self.item_count).then_some(item)
    }

    fn row(&self, item: usize) -> usize {
        self.slot(item) / self.columns
    }

    fn slot_frame(&self, slot: usize) -> Bounds<f32> {
        let row = slot / self.columns;
        let column = slot % self.columns;
        Bounds {
            origin: point((column as f32 * self.pitch_x).round(), row as f32 * self.row_pitch()),
            size: size(self.side, self.side),
        }
    }

    fn frame(&self, item: usize) -> Bounds<f32> {
        let base = self.slot_frame(self.slot(item));
        match self.covered_slot {
            Some(covered) if item == 0 => base.union(&self.slot_frame(covered)),
            _ => base,
        }
    }

    /// items whose rows cross min_y..max_y in content space
    fn items(&self, min_y: f32, max_y: f32) -> Option<RangeInclusive<usize>> {
        if self.item_count == 0 {
            return None;
        }
        let pitch = self.row_pitch();
        let first_row = (min_y / pitch).floor().max(0.0) as i64;
        let last_row = (max_y / pitch).floor() as i64;
        if last_row < first_row {
            return None;
        }
        let first_slot = first_row as usize * self.columns;
        let last_slot = (last_row as usize + 1) * self.columns - 1;
        let first_item = match self.covered_slot {
            Some(covered) if first_slot > covered => first_slot - 1,
            _ => first_slot,
        };
        let last_item = match self.covered_slot {
            Some(covered) if last_slot >= covered => last_slot - 1,
            _ => last_slot,
        }
        .min(self.item_count - 1);
        (first_item <= last_item).then_some(first_item..=last_item)
    }

    /// item at a point, nearest item for a point between or past them
    fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        if self.item_count == 0 {
            return None;
        }
        let row = (y / self.row_pitch()).floor().max(0.0) as usize;
        let column = if self.pitch_x > 0.0 {
            ((x / self.pitch_x).floor().max(0.0) as usize).min(self.columns - 1)
        } else {
            0
        };
        Some(self.item(row * self.columns + column).unwrap_or(self.item_count - 1))
    }
}

/// the item under the fingers and where on screen it sits
#[derive(Clone, Copy, PartialEq, Debug)]
struct MediaPickerPinchAnchor {
    item: usize,
    fraction: f32,
    screen_x: f32,
    screen_y: f32,
}

/// one pinch step between two column counts
#[derive(Clone, Copy, PartialEq, Debug)]
struct MediaPickerPinchZoom {
    from_columns: usize,
    to_columns: usize,
    ratio: f32,
    progress: f32,
}

/// a dequeued cell and the asset it is bound to
struct MediaPickerVisibleCell {
    cell: Entity<MediaPickerGridCell>,
    asset_id: String,
}

pub struct MediaPickerGridController {
    fetcher: Entity<PhotoLibraryFetcher>,
    selection: Entity<MediaSelectionState>,
    theme_store: Entity<ThemeStore>,
    keeps_stable_maximum_viewport: bool,

    pub collection_view: UniformListScrollHandle,
    date_scrubber: Entity<MediaPickerDateScrubber>,
    faded_handle_route: MediaPickerFadedHandlePanRoute,
    last_selection_snapshot: Vec<String>,
    last_refresh_token: Option<Uuid>,

    pub on_pick_asset: Option<MediaPickerGridAssetAction>,
    pub on_toggle_selection: Option<MediaPickerGridAssetAction>,
    pub on_preview_asset: Option<MediaPickerGridPreviewAction>,
    pub shows_selection_controls: bool,
    pub top_content_inset: f32,
    pub bottom_content_inset: f32,
    pub on_camera_tile: Option<Rc<dyn Fn(&mut Window, &mut App)>>,

    layout: MediaPickerGridLayout,
    camera_cell: Option<Entity<MediaPickerCameraCell>>,
    camera_tile_frame: Rc<Cell<Option<Bounds<Pixels>>>>,
    is_camera_tile_displayed: bool,
    is_release_of_held_insertions_scheduled: bool,
    away_from_library_top: Rc<Cell<bool>>,
    grid_origin: Point<Pixels>,
    pinch_anchor: Option<MediaPickerPinchAnchor>,
    pinch_zoom: Option<MediaPickerPinchZoom>,
    is_pinching: bool,
    pinch_scale: f32,
    pinch_base_scale: f32,
    warming_thumbnail_ids: Rc<RefCell<HashSet<String>>>,

    current_hero_lift_id: Option<String>,
    current_hero_hidden: bool,
    fresh_render_cells: Rc<RefCell<HashSet<gpui::EntityId>>>,
    scroll_phase: ScrollPhase,
    scroll_velocity: f32,
    pending_native_drag: bool,
    native_drag_began_away_from_top: bool,
    pending_diagnostic_scroll_callbacks: i64,
    settle_work_item: Option<Task<()>>,
    date_scrubber_work_item: Option<Task<()>>,
    pending_date_scrubber_active: bool,
    hero_prefetch_request: Option<ImageRequestId>,
    is_reloading_library: bool,
    last_layout_width: f32,
    last_layout_height: f32,
    is_viewport_resizing: bool,
    viewport_resize_work_item: Option<Task<()>>,
    scrub_initial_content_offset: f32,
    is_date_scrubbing: bool,
    pending_scrub_commit: Option<f32>,
    scrub_commit_work_item: Option<Task<()>>,
    last_date_index: Option<i64>,
    last_date_month_key: Option<i64>,
    last_date_text: String,
    diagnostics_id: String,
    scroll_performance_sampler: MediaPickerScrollPerformanceSampler,
    month_formatter: DateFormatter,

    visible_cells: HashMap<usize, MediaPickerVisibleCell>,
    reusable_cells: Vec<Entity<MediaPickerGridCell>>,
    observed_content_offset: f32,
    wheel_quiet_work_item: Option<Task<()>>,
    needs_visible_cell_frames_refresh: bool,

    focus_handle: FocusHandle,
    keyboard_cursor: Option<usize>,
    range_anchor: Option<usize>,

    _frame_refresh_observer: RiseSubscription,
    _edits_observer: RiseSubscription,
    _reveal_observer: RiseSubscription,
    _event_listener: Task<()>,
    _selection_observation: Subscription,
    _theme_observation: Subscription,
}

impl MediaPickerGridController {
    pub fn new(
        fetcher: Entity<PhotoLibraryFetcher>,
        selection: Entity<MediaSelectionState>,
        theme_store: Entity<ThemeStore>,
        keeps_stable_maximum_viewport: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let date_scrubber = cx.new(MediaPickerDateScrubber::new);
        let controller = cx.weak_entity();
        date_scrubber.update(cx, |scrubber, _| {
            let began = controller.clone();
            scrubber.on_scrub_began = Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                let controller = began.clone();
                cx.defer(move |cx| {
                    let _ = controller.update(cx, |controller, cx| controller.begin_date_scrubbing(cx));
                });
            }));
            let delta = controller.clone();
            scrubber.on_scrub_delta = Some(Rc::new(move |translation: f32, _: &mut Window, cx: &mut App| {
                let controller = delta.clone();
                cx.defer(move |cx| {
                    let _ = controller.update(cx, |controller, cx| controller.scrub_date(translation, cx));
                });
            }));
            let ended = controller.clone();
            scrubber.on_scrub_ended = Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                let controller = ended.clone();
                cx.defer(move |cx| {
                    let _ = controller.update(cx, |controller, cx| controller.end_date_scrubbing(cx));
                });
            }));
        });
        let faded_handle_route = MediaPickerFadedHandlePanRoute::new(date_scrubber.downgrade());

        let away_from_library_top = Rc::new(Cell::new(false));
        let holder = controller.clone();
        let away = away_from_library_top.clone();
        fetcher.update(cx, |fetcher, _| {
            fetcher.set_delegate(Some(controller));
            fetcher.should_hold_insertions = Some(Rc::new(move |_: &App| holder.upgrade().is_some() && away.get()));
        });
        let diagnostics_id: String = Uuid::new_v4().uuid_string().chars().take(6).collect();
        MediaPickerPerformanceDiagnostics::shared().grid_opened(&diagnostics_id, fetcher.read(cx).asset_count());

        let (events, mut received) = mpsc::unbounded::<MediaPickerGridEvent>();
        let frame_events = events.clone();
        let frame_refresh_observer =
            RiseAppEvents::default_center().observe(MEDIA_PICKER_REFRESH_CELL_FRAMES, move |_| {
                let _ = frame_events.unbounded_send(MediaPickerGridEvent::RefreshCellFrames);
            });
        let edit_events = events.clone();
        let edits_observer = RiseAppEvents::default_center().observe(MEDIA_PICKER_EDITS_DID_CHANGE, move |_| {
            let _ = edit_events.unbounded_send(MediaPickerGridEvent::EditsDidChange);
        });
        let reveal_observer = RiseAppEvents::default_center().observe(MEDIA_PICKER_REVEAL_ASSET, move |note| {
            if let Some(payload) = note.payload::<MediaPickerRevealAssetPayload>() {
                let _ = events.unbounded_send(MediaPickerGridEvent::RevealAsset(payload.clone()));
            }
        });
        let event_listener = cx.spawn(async move |this, cx| {
            while let Some(event) = received.next().await {
                if this.update(cx, |this, cx| this.handle_event(event, cx)).is_err() {
                    break;
                }
            }
        });

        let selection_observation =
            cx.observe(&selection, |this, _, cx| this.refresh_selection_badges_if_needed(cx));
        let theme_observation = cx.observe(&theme_store, |_, _, cx| cx.notify());
        cx.on_release(|this, cx| this.deinit(cx)).detach();

        let mut month_formatter = AppDateFormatter::make(None, Some(Calendar::current()), None);
        month_formatter.set_localized_date_format_from_template("LLLL yyyy");

        Self {
            fetcher,
            selection,
            theme_store,
            keeps_stable_maximum_viewport,
            collection_view: UniformListScrollHandle::new(),
            date_scrubber,
            faded_handle_route,
            last_selection_snapshot: Vec::new(),
            last_refresh_token: None,
            on_pick_asset: None,
            on_toggle_selection: None,
            on_preview_asset: None,
            shows_selection_controls: true,
            top_content_inset: 0.0,
            bottom_content_inset: 120.0,
            on_camera_tile: None,
            layout: MediaPickerGridLayout {
                spacing: GRID_SPACING,
                columns: 3,
                has_leading_tile: false,
                tile_prefers_beside: false,
            },
            camera_cell: None,
            camera_tile_frame: Rc::new(Cell::new(None)),
            is_camera_tile_displayed: false,
            is_release_of_held_insertions_scheduled: false,
            away_from_library_top,
            grid_origin: Point::default(),
            pinch_anchor: None,
            pinch_zoom: None,
            is_pinching: false,
            pinch_scale: 1.0,
            pinch_base_scale: 1.0,
            warming_thumbnail_ids: Rc::new(RefCell::new(HashSet::new())),
            current_hero_lift_id: None,
            current_hero_hidden: false,
            fresh_render_cells: Rc::new(RefCell::new(HashSet::new())),
            scroll_phase: ScrollPhase::Idle,
            scroll_velocity: 0.0,
            pending_native_drag: false,
            native_drag_began_away_from_top: false,
            pending_diagnostic_scroll_callbacks: 0,
            settle_work_item: None,
            date_scrubber_work_item: None,
            pending_date_scrubber_active: false,
            hero_prefetch_request: None,
            is_reloading_library: false,
            last_layout_width: 0.0,
            last_layout_height: 0.0,
            is_viewport_resizing: false,
            viewport_resize_work_item: None,
            scrub_initial_content_offset: 0.0,
            is_date_scrubbing: false,
            pending_scrub_commit: None,
            scrub_commit_work_item: None,
            last_date_index: None,
            last_date_month_key: None,
            last_date_text: String::new(),
            diagnostics_id,
            scroll_performance_sampler: MediaPickerScrollPerformanceSampler::new(),
            month_formatter,
            visible_cells: HashMap::new(),
            reusable_cells: Vec::new(),
            observed_content_offset: 0.0,
            wheel_quiet_work_item: None,
            needs_visible_cell_frames_refresh: false,
            focus_handle: cx.focus_handle(),
            keyboard_cursor: None,
            range_anchor: None,
            _frame_refresh_observer: frame_refresh_observer,
            _edits_observer: edits_observer,
            _reveal_observer: reveal_observer,
            _event_listener: event_listener,
            _selection_observation: selection_observation,
            _theme_observation: theme_observation,
        }
    }

    fn deinit(&mut self, cx: &mut App) {
        self.release_camera_cell(cx);
        self.scroll_performance_sampler.stop("grid_deinit");
        self.flush_diagnostic_scroll_callbacks();
        MediaPickerPerformanceDiagnostics::shared().grid_closed(&self.diagnostics_id);
    }

    fn handle_event(&mut self, event: MediaPickerGridEvent, cx: &mut Context<Self>) {
        match event {
            MediaPickerGridEvent::RefreshCellFrames => self.refresh_visible_cell_frames(cx),
            MediaPickerGridEvent::EditsDidChange => self.reload_visible_edited_cells(cx),
            MediaPickerGridEvent::RevealAsset(payload) => {
                self.reveal_asset(payload.index, &payload.local_id, cx);
            }
        }
    }

    pub fn set_shows_selection_controls(&mut self, shows: bool, cx: &mut Context<Self>) {
        if shows == self.shows_selection_controls {
            return;
        }
        self.shows_selection_controls = shows;
        self.rebind_visible_cells(cx);
    }

    pub fn set_top_content_inset(&mut self, inset: f32, cx: &mut Context<Self>) {
        if (inset - self.top_content_inset).abs() <= 0.5 {
            return;
        }
        self.apply_content_insets(inset, cx);
    }

    pub fn set_bottom_content_inset(&mut self, inset: f32, cx: &mut Context<Self>) {
        if (inset - self.bottom_content_inset).abs() > 0.5 {
            self.bottom_content_inset = inset;
            cx.notify();
        }
    }

    fn view_did_layout_subviews(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if width <= 1.0 {
            return;
        }
        self.grid_origin = bounds.origin;
        let width_changed = (width - self.last_layout_width).abs() > 0.5;
        if width_changed {
            self.last_layout_width = width;
            self.finish_pinch(cx);
            self.layout.columns = Self::preferred_columns(width);
            self.apply_tile_slots(false, cx);
        }
        if self.last_layout_height > 1.0 && (height - self.last_layout_height).abs() > 0.5 {
            self.note_viewport_resize(cx);
        }
        self.last_layout_height = height;
        self.faded_handle_route.scrubber_origin_x.set(width - DATE_SCRUBBER_WIDTH);
        MediaPickerPerformanceDiagnostics::shared().record_grid_layout(self.is_viewport_resizing);
        self.schedule_date_scrubber_update(false, cx);
        self.release_held_insertions_if_at_top(cx);
        cx.notify();
    }

    fn stable_maximum_viewport_height(window: &Window) -> f32 {
        let window_height = f32::from(window.viewport_size().height);
        (window_height * 0.92 - BottomSheetMetrics::native_sheet_grabber_clearance()).max(1.0)
    }

    fn note_viewport_resize(&mut self, cx: &mut Context<Self>) {
        self.is_viewport_resizing = true;
        self.viewport_resize_work_item = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(VIEWPORT_RESIZE_SECONDS))
                .await;
            let _ = this.update(cx, |this, _| {
                this.viewport_resize_work_item = None;
                if this.scroll_phase == ScrollPhase::Idle {
                    this.is_viewport_resizing = false;
                }
            });
        }));
    }

    fn geometry_for(&self, columns: usize, cx: &App) -> Option<MediaPickerGridGeometry> {
        let width = self.last_layout_width;
        if width <= 1.0 {
            return None;
        }
        let item_count = self.fetcher.read(cx).asset_count() + self.leading_items();
        Some(self.layout.geometry(columns, width, item_count))
    }

    fn grid_geometry(&self, cx: &App) -> Option<MediaPickerGridGeometry> {
        self.geometry_for(self.layout.columns, cx)
    }

    fn list_row_count(&self, cx: &App) -> usize {
        let Some(geometry) = self.grid_geometry(cx) else {
            return 0;
        };
        if self.fetcher.read(cx).asset_count() == 0 || geometry.rows == 0 {
            return 0;
        }
        geometry.rows + (self.bottom_content_inset / geometry.row_pitch()).ceil() as usize
    }

    fn has_camera_tile(&self) -> bool {
        self.on_camera_tile.is_some()
    }

    /// items before the first asset
    /// the slot the tile covers belongs to the layout only so the mapping doesnt depend on column count
    fn leading_items(&self) -> usize {
        usize::from(self.has_camera_tile())
    }

    pub fn set_on_camera_tile(&mut self, on_camera_tile: Option<Rc<dyn Fn(&mut Window, &mut App)>>, cx: &mut Context<Self>) {
        let changed = self.on_camera_tile.is_some() != on_camera_tile.is_some();
        self.on_camera_tile = on_camera_tile;
        if changed {
            self.apply_tile_slots(true, cx);
        }
    }

    fn apply_tile_slots(&mut self, reload: bool, cx: &mut Context<Self>) {
        self.layout.has_leading_tile = self.has_camera_tile();
        if !self.layout.has_leading_tile {
            self.release_camera_cell(cx);
        }
        if reload {
            self.reload_data(cx);
        }
    }

    fn grid_item(&self, asset_index: usize) -> usize {
        asset_index + self.leading_items()
    }

    /// asset index of a grid item, none for the tile
    fn grid_asset_index(&self, item: usize) -> Option<usize> {
        item.checked_sub(self.leading_items())
    }

    fn camera_tile_cell(&mut self, cx: &mut Context<Self>) -> Entity<MediaPickerCameraCell> {
        if let Some(cell) = self.camera_cell.clone() {
            return cell;
        }
        let controller = cx.weak_entity();
        let frame = self.camera_tile_frame.clone();
        let cell = cx.new(MediaPickerCameraCell::new);
        cell.update(cx, |tile, cx| {
            tile.on_tap = Some(Box::new(move |window: &mut Window, cx: &mut App| {
                if let Some(frame) = frame.get() {
                    RiseCameraPresenter::shared(cx).update(cx, |presenter, _| presenter.note_origin_tile(frame));
                }
                let on_camera_tile = controller
                    .upgrade()
                    .and_then(|controller| controller.read(cx).on_camera_tile.clone());
                if let Some(on_camera_tile) = on_camera_tile {
                    on_camera_tile(window, cx);
                }
            }));
            tile.preview_view.update(cx, |preview, cx| preview.did_move_to_window(true, cx));
        });
        self.camera_cell = Some(cell.clone());
        cell
    }

    fn set_camera_tile_displayed(&mut self, displayed: bool, cx: &mut Context<Self>) {
        if self.is_camera_tile_displayed == displayed {
            return;
        }
        self.is_camera_tile_displayed = displayed;
        if let Some(cell) = self.camera_cell.clone() {
            let preview = cell.read(cx).preview_view.clone();
            preview.update(cx, |preview, cx| preview.set_is_displayed(displayed, cx));
        }
    }

    fn release_camera_cell(&mut self, cx: &mut App) {
        self.is_camera_tile_displayed = false;
        self.camera_tile_frame.set(None);
        let Some(cell) = self.camera_cell.take() else {
            return;
        };
        let preview = cell.read(cx).preview_view.clone();
        preview.update(cx, |preview, cx| {
            preview.set_is_displayed(false, cx);
            preview.did_move_to_window(false, cx);
        });
    }

    fn scroll_view(&self) -> ScrollHandle {
        self.collection_view.0.borrow().base_handle.clone()
    }

    /// content offset y, negative top inset at the top like a content inset
    fn content_offset_y(&self) -> f32 {
        -f32::from(self.scroll_view().offset().y) - self.top_content_inset
    }

    fn set_content_offset_y(&mut self, offset_y: f32, cx: &mut Context<Self>) {
        let scroll_view = self.scroll_view();
        let x = scroll_view.offset().x;
        scroll_view.set_offset(point(x, px(-(offset_y + self.top_content_inset))));
        cx.notify();
    }

    fn collection_view_height(&self) -> f32 {
        f32::from(self.scroll_view().bounds().size.height)
    }

    fn apply_content_insets(&mut self, inset: f32, cx: &mut Context<Self>) {
        let old_offset = self.content_offset_y();
        let was_at_top = old_offset <= -self.top_content_inset + 1.0;
        self.top_content_inset = inset;
        if was_at_top {
            self.set_content_offset_y(-inset, cx);
        } else {
            self.set_content_offset_y(old_offset, cx);
        }
        self.release_held_insertions_if_at_top(cx);
    }

    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(geometry) = self.grid_geometry(cx) else {
            return range.map(|_| div().into_any_element()).collect();
        };
        let pitch = geometry.row_pitch();
        let may_be_measuring = range.start == 0 && range.len() == 1;
        if may_be_measuring && !self.is_first_row_on_screen(geometry) {
            return vec![div().w_full().h(px(pitch)).into_any_element()];
        }
        let count = self.fetcher.read(cx).asset_count();
        let slots: Vec<(usize, usize)> = (range.start * geometry.columns..range.end * geometry.columns)
            .filter_map(|slot| {
                let item = geometry.item(slot).filter(|item| geometry.slot(*item) == slot)?;
                let asset_index = self.grid_asset_index(item).filter(|index| *index < count)?;
                Some((slot, asset_index))
            })
            .collect();
        let tile_last_row = geometry.covered_slot.map_or(0, |covered| covered / geometry.columns);
        let tile_row = (self.has_camera_tile() && range.start <= tile_last_row).then_some(range.start);
        if !may_be_measuring {
            let drawn: HashSet<usize> = slots.iter().map(|(_, index)| *index).collect();
            self.release_cells_outside(&drawn, cx);
            self.set_camera_tile_displayed(tile_row.is_some(), cx);
        }
        let tile = tile_row.map(|_| self.camera_tile_cell(cx));
        let cursor = self.keyboard_cursor.filter(|_| self.focus_handle.is_focused(window));
        let primary = self.theme_store.read(cx).current_theme().primary._100;
        let mut rows: Vec<Vec<AnyElement>> = range.clone().map(|_| Vec::with_capacity(geometry.columns + 1)).collect();
        if let (Some(row), Some(tile)) = (tile_row, tile) {
            let frame = geometry.frame(0);
            let tile_frame = self.camera_tile_frame.clone();
            rows[row - range.start].push(
                div()
                    .absolute()
                    .left(px(frame.origin.x))
                    .top(px(frame.origin.y - row as f32 * pitch))
                    .w(px(frame.size.width))
                    .h(px(frame.size.height))
                    .child(tile)
                    .child(measure("media-picker-camera-tile-frame", move |bounds, _, _| {
                        tile_frame.set(Some(bounds));
                    }))
                    .into_any_element(),
            );
        }
        let mut rendered_fresh: Vec<gpui::EntityId> = Vec::new();
        for (slot, asset_index) in slots {
            let Some(cell) = self.cell_for_item(asset_index, window, cx) else {
                continue;
            };
            let frame = geometry.slot_frame(slot);
            let cell_id = cell.entity_id();
            let cell_view = if self.fresh_render_cells.borrow().contains(&cell_id) {
                rendered_fresh.push(cell_id);
                AnyView::from(cell).into_any_element()
            } else {
                AnyView::from(cell).cached(StyleRefinement::default().size_full()).into_any_element()
            };
            let wrapped = div()
                .absolute()
                .left(px(frame.origin.x))
                .top_0()
                .size(px(geometry.side))
                .child(cell_view)
                .when(cursor == Some(asset_index), |wrapped| {
                    wrapped.child(div().absolute().inset_0().border_2().border_color(primary))
                })
                .guide_anchor(if asset_index == 0 { Some(RiseGuideAnchor::MediaPickerFirstThumbnail) } else { None });
            rows[slot / geometry.columns - range.start].push(wrapped.into_any_element());
        }
        if !rendered_fresh.is_empty() {
            let fresh_render_cells = self.fresh_render_cells.clone();
            window.defer(cx, move |_, _| {
                let mut fresh = fresh_render_cells.borrow_mut();
                for id in rendered_fresh {
                    fresh.remove(&id);
                }
            });
        }
        rows.into_iter()
            .map(|cells| div().relative().w_full().h(px(pitch)).children(cells).into_any_element())
            .collect()
    }

    fn is_first_row_on_screen(&self, geometry: MediaPickerGridGeometry) -> bool {
        self.top_content_inset + f32::from(self.scroll_view().offset().y) + geometry.row_pitch() > 0.0
    }

    fn release_cells_outside(&mut self, drawn: &HashSet<usize>, cx: &mut Context<Self>) {
        let released: Vec<usize> = self
            .visible_cells
            .keys()
            .copied()
            .filter(|item| !drawn.contains(item))
            .collect();
        for item in released {
            if let Some(visible) = self.visible_cells.remove(&item) {
                self.enqueue_reusable_cell(visible.cell, cx);
            }
        }
    }

    fn enqueue_reusable_cell(&mut self, cell: Entity<MediaPickerGridCell>, cx: &mut Context<Self>) {
        cell.update(cx, |cell, cx| cell.prepare_for_reuse(cx));
        self.reusable_cells.push(cell);
    }

    fn cell_for_item(
        &mut self,
        item: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<MediaPickerGridCell>> {
        let asset = {
            let fetcher = self.fetcher.read(cx);
            let asset = fetcher.asset(item as i64)?;
            if let Some(visible) = self.visible_cells.get(&item)
                && visible.asset_id == asset.id
            {
                return Some(visible.cell.clone());
            }
            asset.clone()
        };
        if let Some(stale) = self.visible_cells.remove(&item) {
            self.enqueue_reusable_cell(stale.cell, cx);
        }
        let cell = self
            .reusable_cells
            .pop()
            .unwrap_or_else(|| cx.new(MediaPickerGridCell::new));
        self.bind(&cell, &asset, item, cx);
        self.visible_cells.insert(item, MediaPickerVisibleCell { cell: cell.clone(), asset_id: asset.id });
        self.will_display(&cell, window, cx);
        Some(cell)
    }

    fn will_display(&self, cell: &Entity<MediaPickerGridCell>, window: &mut Window, cx: &mut Context<Self>) {
        if self.scroll_phase != ScrollPhase::Idle {
            return;
        }
        let cell = cell.downgrade();
        window.defer(cx, move |_, cx| {
            if let Some(cell) = cell.upgrade() {
                cell.read(cx).refresh_screen_frame();
            }
        });
    }

    fn bind(&mut self, cell: &Entity<MediaPickerGridCell>, asset: &LibraryAsset, item: usize, cx: &mut Context<Self>) {
        let (selection_index, has_edits, is_selectable) = {
            let selection = self.selection.read(cx);
            (
                selection.index(&asset.id).map(|index| index as i64),
                selection.edit(&asset.id).is_some_and(|edit| edit.payload.was_edited()),
                selection.can_select(asset),
            )
        };
        let registry_id = MediaViewerFrameKeys::picker_asset(&asset.id);
        let alpha = if self.current_hero_lift_id.as_deref() == Some(registry_id.as_str()) && self.current_hero_hidden {
            0.0
        } else {
            1.0
        };
        if asset.kind == LibraryAssetKind::Video {
            tracing::warn!(target: "riseonly::X2230", item, alpha, asset = %asset.id, "video cell bound");
        }
        MediaPickerPerformanceDiagnostics::shared().record_cell_bind();

        let tapped_thumbnail = cx.weak_entity();
        let thumbnail_asset = asset.clone();
        let on_tap_thumbnail: MediaPickerGridCellAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            let _ = tapped_thumbnail.update(cx, |this, cx| {
                this.handle_tap_thumbnail(&thumbnail_asset, item, window, cx);
            });
        });
        let tapped_selection = cx.weak_entity();
        let selection_asset = asset.clone();
        let on_tap_selection: MediaPickerGridCellAction = Rc::new(move |window: &mut Window, cx: &mut App| {
            let _ = tapped_selection.update(cx, |this, cx| {
                this.range_anchor = Some(item);
                this.toggle_selection(&selection_asset, window, cx);
                if !this.shows_selection_controls {
                    this.reload_visible_edited_cells(cx);
                }
            });
        });

        let fetcher = self.fetcher.clone();
        let placeholder = self.theme_store.read(cx).current_theme().bg._300;
        let marks_edited_pick = !self.shows_selection_controls && selection_index.is_some() && has_edits;
        let shows_selection_control = self.shows_selection_controls || marks_edited_pick;
        let draws_spoiler = self.selection.read(cx).draws_spoiler() && selection_index.is_some();
        cell.update(cx, |cell, cx| {
            cell.set_alpha(alpha, cx);
            cell.set_placeholder_color(placeholder, cx);
            cell.marks_with_check = marks_edited_pick;
            cell.configure(
                MediaPickerGridCellConfiguration {
                    asset,
                    fetcher: &fetcher,
                    selection_index,
                    shows_selection_control,
                    is_selectable,
                    has_edits,
                    loads_thumbnail: true,
                    frame_registry_id: registry_id,
                    on_tap_thumbnail,
                    on_tap_selection,
                },
                cx,
            );
            cell.set_spoiler(draws_spoiler, cx);
        });
    }

    fn rebind_visible_cells(&mut self, cx: &mut Context<Self>) {
        if self.is_reloading_library || self.last_layout_width <= 1.0 {
            return;
        }
        for (item, cell, asset) in self.safe_visible_items(cx) {
            self.bind(&cell, &asset, item, cx);
        }
    }

    /// visible items still in the data, with their cells and assets
    fn safe_visible_items(&self, cx: &App) -> Vec<(usize, Entity<MediaPickerGridCell>, LibraryAsset)> {
        let fetcher = self.fetcher.read(cx);
        self.visible_cells
            .iter()
            .filter_map(|(item, visible)| {
                let asset = fetcher.asset(*item as i64)?;
                Some((*item, visible.cell.clone(), asset.clone()))
            })
            .collect()
    }

    pub fn fresh_screen_frame_at(&self, item: usize, cx: &App) -> Option<Bounds<Pixels>> {
        if let Some(visible) = self.visible_cells.get(&item) {
            return visible.cell.read(cx).capture_screen_frame();
        }
        let fetcher = self.fetcher.read(cx);
        let asset = fetcher.asset(item as i64)?;
        MediaCellScreenFrameRegistry::frame(&MediaViewerFrameKeys::picker_asset(&asset.id))
    }

    pub fn fresh_screen_frame_for_local_id(&self, local_id: &str, cx: &App) -> Option<Bounds<Pixels>> {
        let registry_id = MediaViewerFrameKeys::picker_asset(local_id);
        for visible in self.visible_cells.values() {
            let cell = visible.cell.read(cx);
            if cell.frame_registry_id() == Some(registry_id.as_str()) {
                return cell.capture_screen_frame();
            }
        }
        MediaCellScreenFrameRegistry::frame(&registry_id)
    }

    fn prefetch_hero_if_needed(&mut self, asset: &LibraryAsset, window: &Window, cx: &mut Context<Self>) {
        let cache_key = format!("ph://{}", asset.id);
        if MediaViewerImageMemoryCache::image(
            &cache_key,
            SaiAccountCacheScope::active_user_id().as_deref(),
            CleverImageCachePolicy::Standard,
        )
        .is_some()
        {
            return;
        }
        let previous = self.hero_prefetch_request.take();
        let this = cx.weak_entity();
        let request = self.fetcher.update(cx, |fetcher, cx| {
            fetcher.cancel_image_request(previous);
            fetcher.request_hero_thumbnail(
                asset,
                window,
                move |_, cx| {
                    let _ = this.update(cx, |this, _| this.hero_prefetch_request = None);
                },
                cx,
            )
        });
        self.hero_prefetch_request = request;
    }

    fn handle_tap_thumbnail(&mut self, asset: &LibraryAsset, item: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.keyboard_cursor = Some(item);
        cx.notify();
        if self.shows_selection_controls {
            let modifiers = window.modifiers();
            if modifiers.shift {
                self.select_range(item, window, cx);
                return;
            }
            if modifiers.platform {
                self.range_anchor = Some(item);
                self.toggle_selection(asset, window, cx);
                return;
            }
        }
        self.preview_asset(asset, item, window, cx);
    }

    fn toggle_selection(&mut self, asset: &LibraryAsset, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_toggle_selection) = self.on_toggle_selection.clone() {
            on_toggle_selection(asset, window, cx);
        }
        self.refresh_selection_badges_if_needed(cx);
    }

    fn select_range(&mut self, item: usize, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.range_anchor.unwrap_or(item);
        let Some(on_toggle_selection) = self.on_toggle_selection.clone() else {
            return;
        };
        for index in anchor.min(item)..=anchor.max(item) {
            let Some(asset) = self.fetcher.read(cx).asset(index as i64).cloned() else {
                continue;
            };
            let selection = self.selection.read(cx);
            if selection.max_reached() {
                break;
            }
            if selection.contains(&asset.id) || !selection.can_select(&asset) {
                continue;
            }
            on_toggle_selection(&asset, window, cx);
        }
        self.refresh_selection_badges_if_needed(cx);
    }

    fn preview_asset(&mut self, asset: &LibraryAsset, item: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.prefetch_hero_if_needed(asset, window, cx);
        let frame = self.fresh_screen_frame_at(item, cx);
        if let Some(on_preview_asset) = self.on_preview_asset.clone() {
            on_preview_asset(asset, item as i64, frame, window, cx);
        }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt {
            return;
        }
        let count = self.fetcher.read(cx).asset_count();
        let Some(geometry) = self.grid_geometry(cx).filter(|_| count > 0) else {
            return;
        };
        let cursor = self.keyboard_cursor.unwrap_or_else(|| self.estimated_first_visible_index(cx).max(0) as usize);
        match event.keystroke.key.as_str() {
            "left" => self.move_keyboard_cursor(cursor.saturating_sub(1), geometry, cx),
            "right" => self.move_keyboard_cursor((cursor + 1).min(count - 1), geometry, cx),
            "up" => self.move_keyboard_cursor(cursor.saturating_sub(geometry.columns), geometry, cx),
            "down" => self.move_keyboard_cursor((cursor + geometry.columns).min(count - 1), geometry, cx),
            "space" => {
                let Some(asset) = self.fetcher.read(cx).asset(cursor as i64).cloned() else {
                    return;
                };
                self.keyboard_cursor = Some(cursor);
                self.range_anchor = Some(cursor);
                self.toggle_selection(&asset, window, cx);
            }
            "enter" => {
                let Some(asset) = self.fetcher.read(cx).asset(cursor as i64).cloned() else {
                    return;
                };
                self.keyboard_cursor = Some(cursor);
                self.preview_asset(&asset, cursor, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn move_keyboard_cursor(&mut self, cursor: usize, geometry: MediaPickerGridGeometry, cx: &mut Context<Self>) {
        self.keyboard_cursor = Some(cursor);
        self.collection_view.scroll_to_item(geometry.row(self.grid_item(cursor)), ScrollStrategy::Nearest);
        cx.notify();
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_date_scrubbing {
            return;
        }
        let delta = event.delta.pixel_delta(window.line_height());
        self.scroll_velocity = f32::from(delta.y).abs() * WHEEL_EVENTS_PER_SECOND;
        match event.touch_phase {
            TouchPhase::Started => self.scroll_view_will_begin_dragging(window, cx),
            TouchPhase::Moved => {
                if self.scroll_phase == ScrollPhase::Idle && !self.pending_native_drag {
                    self.scroll_view_will_begin_dragging(window, cx);
                }
            }
            TouchPhase::Ended => self.scroll_view_did_end_dragging(true, cx),
            TouchPhase::Cancelled => self.scroll_view_did_end_dragging(false, cx),
        }
        self.wheel_quiet_work_item = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(WHEEL_QUIET_SECONDS))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.wheel_quiet_work_item = None;
                match this.scroll_phase {
                    ScrollPhase::Decelerating => this.scroll_view_did_end_decelerating(cx),
                    ScrollPhase::Dragging => {
                        if !this.is_date_scrubbing {
                            this.scroll_view_did_end_dragging(false, cx);
                        }
                    }
                    ScrollPhase::Idle => {
                        if this.pending_native_drag {
                            this.scroll_view_did_end_dragging(false, cx);
                        }
                    }
                }
            });
        }));
    }

    fn scroll_view_will_begin_dragging(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settle_work_item = None;
        self.flush_diagnostic_scroll_callbacks();
        self.pending_native_drag = true;
        let top = -self.top_content_inset;
        self.native_drag_began_away_from_top = self.content_offset_y() > top + BottomSheetMetrics::SCROLL_TOP_SLACK;
        if self.native_drag_began_away_from_top {
            self.activate_native_scroll_session(window, cx);
        }
    }

    fn scroll_view_did_end_dragging(&mut self, decelerate: bool, cx: &mut Context<Self>) {
        if !decelerate {
            self.release_held_insertions_if_at_top(cx);
        }
        self.pending_native_drag = false;
        self.native_drag_began_away_from_top = false;
        if self.scroll_phase == ScrollPhase::Idle {
            self.flush_diagnostic_scroll_callbacks();
            self.date_scrubber.update(cx, |scrubber, cx| scrubber.hide_soon(cx));
            return;
        }
        if decelerate {
            self.scroll_phase = ScrollPhase::Decelerating;
            MediaPickerPerformanceDiagnostics::shared().update_scroll_context(
                "decelerating",
                self.scroll_velocity,
                self.visible_cells.len(),
            );
        } else {
            self.scroll_phase = ScrollPhase::Idle;
            self.scroll_performance_sampler.stop("drag_end");
            self.flush_diagnostic_scroll_callbacks();
            self.schedule_settle(SETTLE_DELAY_SECONDS, cx);
        }
        self.date_scrubber.update(cx, |scrubber, cx| scrubber.hide_soon(cx));
    }

    fn scroll_view_did_end_decelerating(&mut self, cx: &mut Context<Self>) {
        self.release_held_insertions_if_at_top(cx);
        if self.scroll_phase != ScrollPhase::Decelerating {
            return;
        }
        self.scroll_phase = ScrollPhase::Idle;
        self.scroll_performance_sampler.stop("deceleration_end");
        self.flush_diagnostic_scroll_callbacks();
        self.schedule_settle(SETTLE_DELAY_SECONDS, cx);
        self.date_scrubber.update(cx, |scrubber, cx| scrubber.hide_soon(cx));
    }

    fn scroll_view_did_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hold_library_insertions_away_from_top(cx);
        if self.scroll_phase == ScrollPhase::Idle && self.pending_native_drag {
            let top = -self.top_content_inset;
            if self.native_drag_began_away_from_top || self.content_offset_y() > top + 4.0 {
                self.activate_native_scroll_session(window, cx);
            }
        }
        if self.scroll_phase == ScrollPhase::Idle && !self.is_date_scrubbing {
            self.schedule_date_scrubber_update(false, cx);
            return;
        }
        self.pending_diagnostic_scroll_callbacks += 1;
        self.schedule_date_scrubber_update(true, cx);
    }

    fn activate_native_scroll_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scroll_phase != ScrollPhase::Idle {
            return;
        }
        self.scroll_phase = ScrollPhase::Dragging;
        self.scroll_performance_sampler
            .start(&self.diagnostics_id, NORMAL_DECELERATION_RATE, window, cx);
        MediaPickerPerformanceDiagnostics::shared().update_scroll_context(
            "drag",
            self.scroll_velocity,
            self.visible_cells.len(),
        );
    }

    fn flush_diagnostic_scroll_callbacks(&mut self) {
        if self.pending_diagnostic_scroll_callbacks <= 0 {
            return;
        }
        MediaPickerPerformanceDiagnostics::shared().record_scroll_callbacks(self.pending_diagnostic_scroll_callbacks);
        self.pending_diagnostic_scroll_callbacks = 0;
    }

    fn schedule_settle(&mut self, delay: f64, cx: &mut Context<Self>) {
        self.settle_work_item = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            let _ = this.update(cx, |this, cx| this.settle_after_scroll(cx));
        }));
    }

    fn settle_after_scroll(&mut self, cx: &mut Context<Self>) {
        if self.is_reloading_library || self.scroll_phase != ScrollPhase::Idle {
            return;
        }
        self.is_viewport_resizing = false;
        self.refresh_visible_cell_frames(cx);
        self.update_date_scrubber(false, cx);
        let diagnostics = MediaPickerPerformanceDiagnostics::shared();
        diagnostics.update_scroll_context("idle", 0.0, self.visible_cells.len());
        diagnostics.emit("settled");
        self.warm_denser_thumbnails(None, cx);
    }

    fn begin_date_scrubbing(&mut self, cx: &mut Context<Self>) {
        if self.scrollable_bounds().is_none() {
            return;
        }
        self.scrub_initial_content_offset = self.content_offset_y();
        self.is_date_scrubbing = true;
        self.pending_scrub_commit = None;
        self.scrub_commit_work_item = None;
        self.scroll_phase = ScrollPhase::Dragging;
        self.pending_native_drag = false;
        self.native_drag_began_away_from_top = false;
        self.scroll_velocity = 0.0;
        self.settle_work_item = None;
    }

    fn scrub_date(&mut self, relative_translation: f32, cx: &mut Context<Self>) {
        if !self.is_date_scrubbing {
            return;
        }
        let Some((minimum, maximum)) = self.scrollable_bounds() else {
            return;
        };
        let top_rail = self.top_content_inset + 8.0;
        let bottom_rail = self.bottom_content_inset + 8.0;
        let rail_distance = (self.last_layout_height - top_rail - bottom_rail - 44.0).max(44.0);
        let content_per_point = (maximum - minimum) / rail_distance;
        let target = maximum.min(minimum.max(self.scrub_initial_content_offset + relative_translation * content_per_point));
        self.pending_scrub_commit = Some(target);
        self.schedule_latest_scrub_commit(cx);
    }

    fn end_date_scrubbing(&mut self, cx: &mut Context<Self>) {
        if !self.is_date_scrubbing {
            return;
        }
        self.scrub_commit_work_item = None;
        self.commit_latest_scrub_target(cx);
        self.is_date_scrubbing = false;
        self.scroll_phase = ScrollPhase::Idle;
        self.flush_diagnostic_scroll_callbacks();
        self.rebind_visible_cells(cx);
        self.schedule_settle(SETTLE_DELAY_SECONDS, cx);
    }

    fn schedule_latest_scrub_commit(&mut self, cx: &mut Context<Self>) {
        if self.scrub_commit_work_item.is_some() {
            return;
        }
        self.scrub_commit_work_item = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(SCRUB_COMMIT_SECONDS))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.scrub_commit_work_item = None;
                this.commit_latest_scrub_target(cx);
                if this.pending_scrub_commit.is_some() && this.is_date_scrubbing {
                    this.schedule_latest_scrub_commit(cx);
                }
            });
        }));
    }

    fn commit_latest_scrub_target(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.pending_scrub_commit.take() else {
            return;
        };
        self.set_content_offset_y(target, cx);
        self.update_date_scrubber(true, cx);
    }

    fn schedule_date_scrubber_update(&mut self, is_active: bool, cx: &mut Context<Self>) {
        self.pending_date_scrubber_active = self.pending_date_scrubber_active || is_active;
        if self.date_scrubber_work_item.is_some() {
            return;
        }
        self.date_scrubber_work_item = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(DATE_SCRUBBER_UPDATE_SECONDS))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.date_scrubber_work_item = None;
                let active = this.pending_date_scrubber_active;
                this.pending_date_scrubber_active = false;
                this.update_date_scrubber(active, cx);
            });
        }));
    }

    fn update_date_scrubber(&mut self, is_active: bool, cx: &mut Context<Self>) {
        let Some(geometry) = self.grid_geometry(cx) else {
            return;
        };
        let (top_inset, bottom_inset) = (self.top_content_inset, self.bottom_content_inset);
        let content_height = geometry.rows as f32 * geometry.row_pitch() + top_inset + bottom_inset;
        let viewport_height = self.collection_view_height();
        let is_available = self.fetcher.read(cx).asset_count() > 0
            && content_height > viewport_height
            && viewport_height / content_height.max(1.0) < 0.55;

        let bounds = self.scrollable_bounds().filter(|_| is_available);
        let Some((minimum, maximum)) = bounds else {
            self.date_scrubber.update(cx, |scrubber, cx| {
                scrubber.update(0.0, String::new(), top_inset, bottom_inset, false, false, cx);
            });
            return;
        };

        let fraction = ((self.content_offset_y() - minimum) / (maximum - minimum).max(1.0)).max(0.0).min(1.0);
        let visible_index = self.estimated_first_visible_index(cx);
        let date_text = if self.last_date_index == Some(visible_index) {
            self.last_date_text.clone()
        } else if let Some(date) = self.fetcher.read(cx).creation_date(visible_index) {
            let components =
                Calendar::current().date_components(&[CalendarComponent::Year, CalendarComponent::Month], &date);
            let month_key = components.year.unwrap_or(0) * 100 + components.month.unwrap_or(0);
            if self.last_date_month_key != Some(month_key) {
                self.last_date_month_key = Some(month_key);
                self.last_date_text = self.formatted_month(&date);
            }
            self.last_date_index = Some(visible_index);
            self.last_date_text.clone()
        } else {
            self.last_date_index = Some(visible_index);
            self.last_date_month_key = None;
            self.last_date_text = String::new();
            String::new()
        };
        self.date_scrubber.update(cx, |scrubber, cx| {
            scrubber.update(fraction, date_text, top_inset, bottom_inset, true, is_active, cx);
        });
    }

    fn estimated_first_visible_index(&self, cx: &App) -> i64 {
        let count = self.fetcher.read(cx).asset_count() as i64;
        let Some(geometry) = self.grid_geometry(cx).filter(|_| count > 0) else {
            return 0;
        };
        let row_height = geometry.row_pitch().max(1.0);
        let visible_y = (self.content_offset_y() + self.top_content_inset).max(0.0);
        let row = (visible_y / row_height).floor().max(0.0) as i64;
        let first_slot = row * geometry.columns as i64;
        let tile_cover = i64::from(geometry.covered_slot.is_some_and(|covered| first_slot > covered as i64));
        (count - 1).min((first_slot - self.leading_items() as i64 - tile_cover).max(0))
    }

    fn formatted_month(&self, date: &Date) -> String {
        let value = self.month_formatter.string_from(date);
        let mut characters = value.chars();
        match characters.next() {
            Some(first) => first.to_uppercase().chain(characters).collect(),
            None => value,
        }
    }

    /// min and max of the content offset y
    fn scrollable_bounds(&self) -> Option<(f32, f32)> {
        let minimum = -self.top_content_inset;
        let maximum = minimum.max(minimum + f32::from(self.scroll_view().max_offset().y));
        (maximum - minimum > 1.0).then_some((minimum, maximum))
    }

    fn refresh_visible_cell_frames(&self, cx: &App) {
        for visible in self.visible_cells.values() {
            visible.cell.read(cx).refresh_screen_frame();
        }
    }

    pub fn apply_refresh_trigger(&mut self, token: Uuid, cx: &mut Context<Self>) {
        if self.last_refresh_token == Some(token) {
            return;
        }
        self.last_refresh_token = Some(token);
        self.refresh_selection_badges_if_needed(cx);
    }

    pub fn reload_visible_edited_cells(&mut self, cx: &mut Context<Self>) {
        if self.scroll_phase != ScrollPhase::Idle {
            return;
        }
        for (item, cell, asset) in self.safe_visible_items(cx) {
            self.bind(&cell, &asset, item, cx);
        }
    }

    pub fn refresh_selection_badges_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.shows_selection_controls {
            return;
        }
        let current: Vec<String> = self.selection.read(cx).ordered_identifiers().to_vec();
        {
            let current_set: HashSet<&str> = current.iter().map(String::as_str).collect();
            let snapshot_set: HashSet<&str> = self.last_selection_snapshot.iter().map(String::as_str).collect();
            let visible = self.safe_visible_items(cx);
            for (_, cell, asset) in &visible {
                if !current_set.contains(asset.id.as_str()) && !snapshot_set.contains(asset.id.as_str()) {
                    continue;
                }
                let (index, draws_spoiler) = {
                    let selection = self.selection.read(cx);
                    (selection.index(&asset.id).map(|index| index as i64), selection.draws_spoiler())
                };
                cell.update(cx, |cell, cx| {
                    cell.set_selection_index(index, true, cx);
                    cell.set_spoiler(draws_spoiler && index.is_some(), cx);
                });
            }
            if current_set != snapshot_set {
                for (_, cell, asset) in &visible {
                    let (index, is_selectable) = {
                        let selection = self.selection.read(cx);
                        (selection.index(&asset.id).map(|index| index as i64), selection.can_select(asset))
                    };
                    cell.update(cx, |cell, cx| {
                        cell.set_selection_index(index, false, cx);
                        cell.set_selectable(is_selectable, cx);
                    });
                }
            }
        }
        self.last_selection_snapshot = current;
    }

    pub fn apply_hero_lift(&mut self, id: Option<String>, hidden: bool, cx: &mut Context<Self>) {
        let old_id = self.current_hero_lift_id.take();
        self.current_hero_lift_id = id.clone();
        self.current_hero_hidden = hidden;
        if id.is_none() && old_id.is_none() {
            return;
        }
        let mut restored = 0usize;
        for visible in self.visible_cells.values() {
            let registry_id = MediaViewerFrameKeys::picker_asset(&visible.asset_id);
            let alpha = if id.as_deref() == Some(registry_id.as_str()) && hidden { 0.0 } else { 1.0 };
            let changed = visible.cell.update(cx, |cell, cx| {
                if alpha > 0.0 && cell.alpha() == 0.0 {
                    restored += 1;
                }
                let changed = cell.alpha() != alpha;
                cell.set_alpha(alpha, cx);
                changed
            });
            if changed {
                self.fresh_render_cells.borrow_mut().insert(visible.cell.entity_id());
            }
        }
        if !self.fresh_render_cells.borrow().is_empty() {
            cx.notify();
        }
        tracing::warn!(target: "riseonly::X2230", lift = ?id, hidden, restored, visible = self.visible_cells.len(), listed = self.fetcher.read(cx).asset_count(), "hero lift applied");
    }

    fn reveal_asset(&mut self, index: i64, local_id: &str, cx: &mut Context<Self>) {
        let is_current = self.fetcher.read(cx).asset(index).is_some_and(|asset| asset.id == local_id);
        let Some(geometry) = self.grid_geometry(cx).filter(|_| is_current && index >= 0) else {
            return;
        };
        let item = self.grid_item(index as usize);
        if item >= geometry.item_count {
            return;
        }
        let row = geometry.row(item);
        let cell_top = geometry.frame(item).origin.y;
        let viewport_top = self.content_offset_y() + self.top_content_inset;
        let viewport_bottom = self.content_offset_y() + self.collection_view_height() - self.bottom_content_inset;
        if cell_top >= viewport_top && cell_top + geometry.side <= viewport_bottom {
            return;
        }
        self.collection_view.scroll_to_item(row, ScrollStrategy::Center);
        self.needs_visible_cell_frames_refresh = true;
        cx.notify();
    }

    /// reloads data, every cell goes back to the queue
    fn reload_data(&mut self, cx: &mut Context<Self>) {
        let cells: Vec<Entity<MediaPickerGridCell>> = self.visible_cells.drain().map(|(_, visible)| visible.cell).collect();
        for cell in cells {
            self.enqueue_reusable_cell(cell, cx);
        }
        cx.notify();
    }

    fn default_columns(width: f32) -> usize {
        if width >= 900.0 {
            6
        } else if width >= 600.0 {
            5
        } else if width >= 400.0 {
            4
        } else {
            3
        }
    }

    fn column_range(width: f32) -> RangeInclusive<usize> {
        let base = Self::default_columns(width);
        3.max(base - 1)..=(base + 3)
    }

    fn preferred_columns(width: f32) -> usize {
        let range = Self::column_range(width);
        let stored = UserDefaults::standard().integer(MEDIA_PICKER_GRID_COLUMNS);
        if stored <= 0 {
            return Self::default_columns(width);
        }
        (stored as usize).clamp(*range.start(), *range.end())
    }

    fn gesture_recognizer_should_begin(&self, cx: &App) -> bool {
        !self.is_date_scrubbing && self.fetcher.read(cx).asset_count() > 0 && self.last_layout_width > 1.0
    }

    fn handle_pinch(&mut self, event: &PinchEvent, cx: &mut Context<Self>) {
        match event.phase {
            TouchPhase::Started => self.begin_pinch(event.position, cx),
            TouchPhase::Moved => {
                if !self.is_pinching {
                    self.begin_pinch(event.position, cx);
                }
                if !self.is_pinching {
                    return;
                }
                self.pinch_scale *= 1.0 + event.delta;
                self.update_pinch(event.position, cx);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if self.is_pinching {
                    self.end_pinch(cx);
                }
            }
        }
        cx.stop_propagation();
    }

    fn begin_pinch(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.gesture_recognizer_should_begin(cx) {
            return;
        }
        self.finish_pinch(cx);
        self.is_pinching = true;
        self.pinch_scale = 1.0;
        self.pinch_base_scale = 1.0;
        self.capture_pinch_anchor(position, cx);
        self.warm_denser_thumbnails(self.pinch_anchor, cx);
    }

    fn capture_pinch_anchor(&mut self, position: Point<Pixels>, cx: &App) {
        let screen_x = f32::from(position.x - self.grid_origin.x);
        let screen_y = f32::from(position.y - self.grid_origin.y);
        let content_y = screen_y + self.content_offset_y();
        let anchor = self.grid_geometry(cx).and_then(|geometry| {
            let item = geometry.item_at(screen_x, content_y)?;
            let frame = geometry.frame(item);
            (frame.size.height > 0.0).then(|| MediaPickerPinchAnchor {
                item,
                fraction: ((content_y - frame.origin.y) / frame.size.height).clamp(0.0, 1.0),
                screen_x,
                screen_y,
            })
        });
        self.pinch_anchor = anchor;
    }

    fn update_pinch(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.pinch_anchor.is_none() {
            return;
        }
        let relative = self.pinch_scale / self.pinch_base_scale.max(0.01);
        let base = self.pinch_zoom.map_or(self.layout.columns, |zoom| zoom.from_columns);
        let target = if relative >= 1.0 { base.saturating_sub(1) } else { base + 1 };
        if !Self::column_range(self.last_layout_width).contains(&target) {
            self.pinch_zoom = None;
            return;
        }
        if self.pinch_zoom.map(|zoom| zoom.to_columns) != Some(target) {
            self.pinch_zoom = None;
            self.begin_pinch_zoom(target, cx);
        }
        let Some(zoom) = self.pinch_zoom.filter(|zoom| (zoom.ratio - 1.0).abs() > 0.001) else {
            return;
        };
        let progress = (relative - 1.0) / (zoom.ratio - 1.0);
        if progress >= 1.0 {
            self.commit_pinch_level(cx);
            self.pinch_base_scale = self.pinch_scale;
            self.capture_pinch_anchor(position, cx);
            return;
        }
        if let Some(zoom) = self.pinch_zoom.as_mut() {
            zoom.progress = progress.clamp(0.0, 1.0);
        }
    }

    fn begin_pinch_zoom(&mut self, target: usize, cx: &App) {
        let from_columns = self.layout.columns;
        let (Some(from), Some(to)) = (self.geometry_for(from_columns, cx), self.geometry_for(target, cx)) else {
            return;
        };
        if to.item_count == 0 || from.side <= 0.0 {
            return;
        }
        self.pinch_zoom =
            Some(MediaPickerPinchZoom { from_columns, to_columns: target, ratio: to.side / from.side, progress: 0.0 });
    }

    /// lands the step, the new count with the anchor item back under the fingers
    fn commit_pinch_level(&mut self, cx: &mut Context<Self>) {
        let Some(zoom) = self.pinch_zoom.take() else {
            return;
        };
        let Some(geometry) = self.geometry_for(zoom.to_columns, cx) else {
            return;
        };
        let target_offset = self.pinch_anchor.filter(|_| geometry.item_count > 0).map(|anchor| {
            let frame = geometry.frame(anchor.item.min(geometry.item_count - 1));
            let point = frame.origin.y + anchor.fraction * frame.size.height;
            let minimum = -self.top_content_inset;
            let maximum = minimum.max(geometry.content_height + self.bottom_content_inset - self.collection_view_height());
            (point - anchor.screen_y).clamp(minimum, maximum)
        });
        self.layout.columns = zoom.to_columns;
        if let Some(offset) = target_offset {
            self.set_content_offset_y(offset, cx);
        }
        let _ = selection_changed();
        UserDefaults::standard().set(MEDIA_PICKER_GRID_COLUMNS, zoom.to_columns as i64);
        self.needs_visible_cell_frames_refresh = true;
        cx.notify();
    }

    fn end_pinch(&mut self, cx: &mut Context<Self>) {
        if self.pinch_zoom.is_some_and(|zoom| zoom.progress >= 0.5) {
            self.commit_pinch_level(cx);
        }
        self.finish_pinch(cx);
    }

    fn finish_pinch(&mut self, cx: &mut Context<Self>) {
        if !self.is_pinching && self.pinch_zoom.is_none() {
            return;
        }
        self.pinch_zoom = None;
        self.pinch_anchor = None;
        self.is_pinching = false;
        self.refresh_visible_cell_frames(cx);
        self.warm_denser_thumbnails(None, cx);
    }

    /// why: a closing pinch brings in rows never bound and their cells only read the memory cache
    /// so they stood blank
    /// rows the densest level would show around the fingers load into the cache first
    fn warm_denser_thumbnails(&mut self, anchor: Option<MediaPickerPinchAnchor>, cx: &mut Context<Self>) {
        if self.is_date_scrubbing {
            return;
        }
        let height = self.collection_view_height();
        let width = self.last_layout_width;
        let densest = *Self::column_range(width).end();
        if height <= 0.0 || densest <= self.layout.columns {
            return;
        }
        let Some(geometry) = self.geometry_for(densest, cx).filter(|geometry| geometry.item_count > 0) else {
            return;
        };
        let (item, fraction, screen_y) = match anchor {
            Some(anchor) => (anchor.item, anchor.fraction, anchor.screen_y),
            None => {
                let Some(current) = self.grid_geometry(cx) else {
                    return;
                };
                let Some(item) = current.item_at(width / 2.0, self.content_offset_y() + height / 2.0) else {
                    return;
                };
                (item, 0.5, height / 2.0)
            }
        };
        let frame = geometry.frame(item.min(geometry.item_count - 1));
        let top = frame.origin.y + fraction * frame.size.height - screen_y;
        let Some(items) = geometry.items(top - height / 4.0, top - height / 4.0 + height * 1.5) else {
            return;
        };
        let user_id = SaiAccountCacheScope::active_user_id();
        for item in items {
            let Some(asset) = self
                .grid_asset_index(item)
                .and_then(|index| self.fetcher.read(cx).asset(index as i64).cloned())
            else {
                continue;
            };
            let asset_id = asset.id.clone();
            let key = format!("ph://{asset_id}");
            if self.warming_thumbnail_ids.borrow().contains(&asset_id)
                || MediaViewerImageMemoryCache::memory_only(&key, user_id.as_deref()).is_some()
            {
                continue;
            }
            self.warming_thumbnail_ids.borrow_mut().insert(asset_id.clone());
            let warming = self.warming_thumbnail_ids.clone();
            let user_id = user_id.clone();
            self.fetcher.update(cx, |fetcher, cx| {
                fetcher.request_grid_thumbnail(
                    &asset,
                    move |delivery, _| {
                        let (is_cancelled, is_degraded) = (delivery.is_cancelled, delivery.is_degraded);
                        if let Some(image) = delivery.image
                            && !is_cancelled
                            && (MediaViewerImageMemoryCache::memory_only(&key, user_id.as_deref()).is_none()
                                || !is_degraded)
                        {
                            MediaViewerImageMemoryCache::store(image, &key, user_id.as_deref());
                        }
                        if is_cancelled || !is_degraded {
                            warming.borrow_mut().remove(&asset_id);
                        }
                    },
                    cx,
                );
            });
        }
    }

    /// away from the top a new photo would push every cell by one
    /// it waits until the grid is back at the top and then shows as the first cell
    fn hold_library_insertions_away_from_top(&mut self, cx: &mut Context<Self>) {
        let away_from_top = self.is_away_from_library_top(cx);
        self.away_from_library_top.set(away_from_top);
        let (holds, has_held) = {
            let fetcher = self.fetcher.read(cx);
            (fetcher.holds_insertions, fetcher.has_held_insertions())
        };
        if holds != away_from_top {
            self.fetcher.update(cx, |fetcher, _| fetcher.holds_insertions = away_from_top);
        }
        if away_from_top || !has_held || self.is_release_of_held_insertions_scheduled {
            return;
        }
        self.is_release_of_held_insertions_scheduled = true;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                this.is_release_of_held_insertions_scheduled = false;
                if this.fetcher.read(cx).holds_insertions {
                    return;
                }
                this.fetcher.update(cx, |fetcher, cx| fetcher.release_held_insertions(cx));
            });
        })
        .detach();
    }

    fn is_away_from_library_top(&self, cx: &App) -> bool {
        let pitch = self.grid_geometry(cx).map_or(0.0, |geometry| geometry.row_pitch());
        let slack = LIBRARY_TOP_SLACK.max(pitch * 0.5);
        self.content_offset_y() > -self.top_content_inset + slack
    }

    /// scroll ends, layout passes and inset changes re-check the top
    /// a rest at the top with no scroll callback still shows a held photo
    fn release_held_insertions_if_at_top(&mut self, cx: &mut Context<Self>) {
        let fetcher = self.fetcher.read(cx);
        if !(fetcher.has_held_insertions() || fetcher.holds_insertions) {
            return;
        }
        self.hold_library_insertions_away_from_top(cx);
    }

    fn observe_content_offset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.content_offset_y();
        if current == self.observed_content_offset {
            return;
        }
        self.observed_content_offset = current;
        let this = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = this.update(cx, |this, cx| this.scroll_view_did_scroll(window, cx));
        });
    }
}

impl PhotoLibraryFetcherDelegate for MediaPickerGridController {
    fn photo_library_did_change(&mut self, _fetcher: &Entity<PhotoLibraryFetcher>, cx: &mut Context<Self>) {
        self.settle_work_item = None;
        self.finish_pinch(cx);
        self.is_reloading_library = true;
        self.apply_tile_slots(false, cx);
        self.reload_data(cx);
        let count = self.fetcher.read(cx).asset_count();
        self.keyboard_cursor = self.keyboard_cursor.filter(|cursor| *cursor < count);
        self.range_anchor = self.range_anchor.filter(|anchor| *anchor < count);
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                this.is_reloading_library = false;
                this.last_date_index = None;
                this.last_date_month_key = None;
                this.update_date_scrubber(false, cx);
            });
        })
        .detach();
    }

    fn photo_library_filter_did_change(&mut self, fetcher: &Entity<PhotoLibraryFetcher>, cx: &mut Context<Self>) {
        self.photo_library_did_change(fetcher, cx);
        let top = -self.top_content_inset;
        self.set_content_offset_y(top, cx);
    }
}

impl Render for MediaPickerGridController {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.observe_content_offset(window, cx);
        if self.needs_visible_cell_frames_refresh {
            self.needs_visible_cell_frames_refresh = false;
            let this = cx.weak_entity();
            window.defer(cx, move |_, cx| {
                let _ = this.update(cx, |this, cx| this.refresh_visible_cell_frames(cx));
            });
        }

        let viewport = window.viewport_size();
        self.layout.tile_prefers_beside = viewport.width > viewport.height;
        let width = self.last_layout_width;
        let height = if self.keeps_stable_maximum_viewport {
            Self::stable_maximum_viewport_height(window)
        } else {
            self.last_layout_height
        };

        let grid = (width > 1.0).then(|| {
            let list = uniform_list(
                "media-picker-grid-rows",
                self.list_row_count(cx),
                cx.processor(|this, range: Range<usize>, window, cx| this.render_rows(range, window, cx)),
            )
            .track_scroll(&self.collection_view)
            .w_full()
            .h(px(height))
            .pt(px(self.top_content_inset))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                this.handle_scroll_wheel(event, window, cx);
            }));
            let changed_route = self.faded_handle_route.clone();
            let ended_route = self.faded_handle_route.clone();
            let gesture = DragGesture::new(FADED_HANDLE_PAN_HYSTERESIS)
                .on_changed(move |value, window, cx| changed_route.handle_pan_changed(value, window, cx))
                .on_ended(move |_, window, cx| ended_route.handle_pan_ended(window, cx));
            div().w(px(width)).h(px(height)).child(list).simultaneous_gesture(
                ElementId::NamedChild(Arc::new(ElementId::View(cx.entity_id())), "faded-handle-pan".into()),
                gesture,
                GestureMask::All,
            )
        });

        let this = cx.weak_entity();
        div()
            .id("media-picker-grid")
            .key_context("MediaPickerGrid")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_key_down(event, window, cx);
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| this.handle_pinch(event, cx)))
            .relative()
            .size_full()
            .overflow_hidden()
            .children(grid)
            .child(measure("media-picker-grid-bounds", move |bounds, _, cx| {
                let _ = this.update(cx, |this, cx| this.view_did_layout_subviews(bounds, cx));
            }))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(DATE_SCRUBBER_WIDTH))
                    .child(self.date_scrubber.clone()),
            )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MediaPickerFadedHandlePanPhase {
    Possible,
    Tracking,
    Failed,
}

/// while the date handle is faded it takes no pointer so a drag from its resting place reaches the grid
/// this pan claims only such drags, clicks and other drags never meet it
#[derive(Clone)]
struct MediaPickerFadedHandlePanRoute {
    scrubber: WeakEntity<MediaPickerDateScrubber>,
    scrubber_origin_x: Rc<Cell<f32>>,
    phase: Rc<Cell<MediaPickerFadedHandlePanPhase>>,
}

impl MediaPickerFadedHandlePanRoute {
    fn new(scrubber: WeakEntity<MediaPickerDateScrubber>) -> Self {
        Self {
            scrubber,
            scrubber_origin_x: Rc::new(Cell::new(0.0)),
            phase: Rc::new(Cell::new(MediaPickerFadedHandlePanPhase::Possible)),
        }
    }

    fn handle_pan(&self, pan: MediaPickerDateScrubberPan, window: &mut Window, cx: &mut App) {
        let Some(scrubber) = self.scrubber.upgrade() else {
            return;
        };
        scrubber.update(cx, |scrubber, cx| scrubber.handle_scrub_pan(pan, window, cx));
    }

    fn handle_pan_changed(&self, value: &DragGestureValue, window: &mut Window, cx: &mut App) {
        match self.phase.get() {
            MediaPickerFadedHandlePanPhase::Failed => {}
            MediaPickerFadedHandlePanPhase::Tracking => {
                self.handle_pan(MediaPickerDateScrubberPan::Changed(f32::from(value.translation.height)), window, cx);
            }
            MediaPickerFadedHandlePanPhase::Possible => {
                if !self.gesture_recognizer_should_receive(value, cx)
                    || !Self::gesture_recognizer_should_begin(value.translation)
                {
                    self.phase.set(MediaPickerFadedHandlePanPhase::Failed);
                    return;
                }
                self.phase.set(MediaPickerFadedHandlePanPhase::Tracking);
                self.handle_pan(MediaPickerDateScrubberPan::Began, window, cx);
                self.handle_pan(MediaPickerDateScrubberPan::Changed(f32::from(value.translation.height)), window, cx);
            }
        }
    }

    fn handle_pan_ended(&self, window: &mut Window, cx: &mut App) {
        let was_tracking = self.phase.get() == MediaPickerFadedHandlePanPhase::Tracking;
        self.phase.set(MediaPickerFadedHandlePanPhase::Possible);
        if was_tracking {
            self.handle_pan(MediaPickerDateScrubberPan::Ended, window, cx);
        }
    }

    fn gesture_recognizer_should_receive(&self, value: &DragGestureValue, cx: &App) -> bool {
        let Some(scrubber) = self.scrubber.upgrade() else {
            return false;
        };
        let location = point(value.start_location.x - px(self.scrubber_origin_x.get()), value.start_location.y);
        scrubber.read(cx).is_on_faded_handle(location)
    }

    fn gesture_recognizer_should_begin(translation: Size<Pixels>) -> bool {
        translation.height.abs() >= translation.width.abs()
    }
}
