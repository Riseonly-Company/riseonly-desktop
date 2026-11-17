use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyView, App, Context, ElementId, Entity, IntoElement, Pixels, Point, Render, RenderOnce, ScrollHandle,
    Size, StyleRefinement, Task, WeakEntity, Window, div, px,
};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::media_picker::ui::media_picker_date_scrubber::{
    MediaPickerDateScrubber, MediaPickerDateScrubberPan,
};
use crate::modules::user::components::profile_content::profile_scroll_coordinator::ProfileScrollCoordinator;

const RETRY_AND_COMMIT_DELAY: Duration = Duration::from_millis(50);

const PAN_HYSTERESIS: f32 = 10.0;

#[derive(IntoElement)]
pub struct ProfileGridDateScrubberLayer {
    pub id: ElementId,
    pub timestamps: Vec<TimeInterval>,
    pub columns: i64,
    pub cell_aspect: f32,
    pub interitem_spacing: f32,
    pub horizontal_padding: f32,
    pub tab_key: i64,
    pub scroll_coordinator: Entity<ProfileScrollCoordinator>,
    pub top_inset: f32,
    pub bottom_inset: f32,
    pub columns_for_width: Option<fn(f32) -> i64>,
}

impl ProfileGridDateScrubberLayer {
    pub const LAYER_WIDTH: f32 = 210.0;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        timestamps: Vec<TimeInterval>,
        columns: i64,
        cell_aspect: f32,
        interitem_spacing: f32,
        horizontal_padding: f32,
        tab_key: i64,
        scroll_coordinator: Entity<ProfileScrollCoordinator>,
        top_inset: f32,
        bottom_inset: f32,
    ) -> Self {
        Self {
            id: id.into(),
            timestamps,
            columns,
            cell_aspect,
            interitem_spacing,
            horizontal_padding,
            tab_key,
            scroll_coordinator,
            top_inset,
            bottom_inset,
            columns_for_width: None,
        }
    }

    pub fn columns_for_width(mut self, columns_for_width: fn(f32) -> i64) -> Self {
        self.columns_for_width = Some(columns_for_width);
        self
    }
}

impl RenderOnce for ProfileGridDateScrubberLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let representable = ProfileGridDateScrubberRepresentable {
            timestamps: self.timestamps,
            columns: self.columns.max(1),
            cell_aspect: if self.cell_aspect > 0.0 { self.cell_aspect } else { 1.0 },
            interitem_spacing: self.interitem_spacing,
            horizontal_padding: self.horizontal_padding,
            tab_key: self.tab_key,
            scroll_coordinator: self.scroll_coordinator,
            top_inset: self.top_inset,
            bottom_inset: self.bottom_inset,
            columns_for_width: self.columns_for_width,
        };
        let ui_view = window.use_keyed_state(self.id, cx, |_, cx| {
            ProfileGridDateScrubberRepresentable::make_ui_view(cx)
        });
        representable.update_ui_view(&ui_view, cx);
        div().size_full().child(AnyView::from(ui_view).cached(StyleRefinement::default().size_full()))
    }
}

struct ProfileGridDateScrubberRepresentable {
    timestamps: Vec<TimeInterval>,
    columns: i64,
    cell_aspect: f32,
    interitem_spacing: f32,
    horizontal_padding: f32,
    tab_key: i64,
    scroll_coordinator: Entity<ProfileScrollCoordinator>,
    top_inset: f32,
    bottom_inset: f32,
    columns_for_width: Option<fn(f32) -> i64>,
}

impl ProfileGridDateScrubberRepresentable {
    fn make_ui_view(cx: &mut Context<ProfileGridDateScrubberHostView>) -> ProfileGridDateScrubberHostView {
        ProfileGridDateScrubberHostView::new(cx)
    }

    fn update_ui_view(self, ui_view: &Entity<ProfileGridDateScrubberHostView>, cx: &mut App) {
        ui_view.update(cx, |ui_view, cx| {
            ui_view.columns_for_width = self.columns_for_width;
            ui_view.configure(
                self.timestamps,
                self.columns,
                self.cell_aspect,
                self.interitem_spacing,
                self.horizontal_padding,
                self.tab_key,
                self.scroll_coordinator,
                self.top_inset,
                self.bottom_inset,
                cx,
            );
        });
    }

    fn dismantle_ui_view(ui_view: &mut ProfileGridDateScrubberHostView) {
        ui_view.invalidate();
    }
}

/// what the scrubber read from the scroll view last
/// gpui has no kvo so the host compares each frame values with these and reacts to the ones that moved
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct ObservedScrollGeometry {
    content_offset_y: Pixels,
    max_offset_y: Pixels,
    bounds_size: Size<Pixels>,
}

impl ObservedScrollGeometry {
    fn read(scroll_view: &ScrollHandle) -> Self {
        Self {
            content_offset_y: scroll_view.offset().y,
            max_offset_y: scroll_view.max_offset().y,
            bounds_size: scroll_view.bounds().size,
        }
    }
}

pub struct ProfileGridDateScrubberHostView {
    scrubber: Entity<MediaPickerDateScrubber>,
    scroll_view_scrub_route: ProfileGridScrubPanRoute,
    scroll_coordinator: Option<WeakEntity<ProfileScrollCoordinator>>,
    scroll_view: Option<ScrollHandle>,
    observed: Option<ObservedScrollGeometry>,
    tab_key: i64,
    timestamps: Vec<TimeInterval>,
    columns: i64,
    columns_for_width: Option<fn(f32) -> i64>,
    cell_aspect: f32,
    interitem_spacing: f32,
    horizontal_padding: f32,
    top_inset: f32,
    bottom_inset: f32,
    bounds_height: Rc<Cell<f32>>,
    bounds_width: Rc<Cell<f32>>,
    bind_work_item: Option<Task<()>>,
    pending_scrub_commit: Option<f32>,
    scrub_commit_work_item: Option<Task<()>>,
    scrub_initial_offset: f32,
    scrub_range: f32,
    is_scrubbing: bool,
    last_month_key: Option<i64>,
    last_date_text: String,
}

impl ProfileGridDateScrubberHostView {
    pub const TAKES_TOUCHES_DIRECTLY: bool = true;

    pub fn new(cx: &mut Context<Self>) -> Self {
        let scrubber = cx.new(MediaPickerDateScrubber::new);
        let host = cx.weak_entity();
        scrubber.update(cx, |scrubber, _| {
            let began = host.clone();
            scrubber.on_scrub_began = Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                let host = began.clone();
                cx.defer(move |cx| {
                    let _ = host.update(cx, |host, cx| host.begin_scrubbing(cx));
                });
            }));
            let delta = host.clone();
            scrubber.on_scrub_delta = Some(Rc::new(move |translation: f32, _: &mut Window, cx: &mut App| {
                let host = delta.clone();
                cx.defer(move |cx| {
                    let _ = host.update(cx, |host, cx| host.scrub_by(translation, cx));
                });
            }));
            let ended = host;
            scrubber.on_scrub_ended = Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                let host = ended.clone();
                cx.defer(move |cx| {
                    let _ = host.update(cx, |host, cx| host.end_scrubbing(cx));
                });
            }));
        });
        cx.on_release(|host, _| ProfileGridDateScrubberRepresentable::dismantle_ui_view(host))
            .detach();
        let scroll_view_scrub_route =
            ProfileGridScrubPanRoute::new(scrubber.downgrade(), !Self::TAKES_TOUCHES_DIRECTLY);
        Self {
            scrubber,
            scroll_view_scrub_route,
            scroll_coordinator: None,
            scroll_view: None,
            observed: None,
            tab_key: 0,
            timestamps: Vec::new(),
            columns: 3,
            columns_for_width: None,
            cell_aspect: 1.0,
            interitem_spacing: 2.0,
            horizontal_padding: 0.0,
            top_inset: 0.0,
            bottom_inset: 0.0,
            bounds_height: Rc::new(Cell::new(0.0)),
            bounds_width: Rc::new(Cell::new(0.0)),
            bind_work_item: None,
            pending_scrub_commit: None,
            scrub_commit_work_item: None,
            scrub_initial_offset: 0.0,
            scrub_range: 0.0,
            is_scrubbing: false,
            last_month_key: None,
            last_date_text: String::new(),
        }
    }

    /// only the live area of the scrubber answers the pointer, the rest of the strip stays transparent
    /// or the pager loses a swipe that starts near the right edge
    pub fn point_inside(&self, point: Point<Pixels>, cx: &App) -> bool {
        self.scrubber.read(cx).point_inside(point)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        timestamps: Vec<TimeInterval>,
        columns: i64,
        cell_aspect: f32,
        interitem_spacing: f32,
        horizontal_padding: f32,
        tab_key: i64,
        scroll_coordinator: Entity<ProfileScrollCoordinator>,
        top_inset: f32,
        bottom_inset: f32,
        cx: &mut Context<Self>,
    ) {
        let identity_changed = self.tab_key != tab_key
            || self
                .scroll_coordinator
                .as_ref()
                .is_none_or(|current| current.entity_id() != scroll_coordinator.entity_id());
        let inputs_changed = identity_changed
            || self.timestamps != timestamps
            || self.columns != columns
            || self.cell_aspect != cell_aspect
            || self.interitem_spacing != interitem_spacing
            || self.horizontal_padding != horizontal_padding
            || self.top_inset != top_inset
            || self.bottom_inset != bottom_inset;
        self.timestamps = timestamps;
        self.columns = columns;
        self.cell_aspect = cell_aspect;
        self.interitem_spacing = interitem_spacing;
        self.horizontal_padding = horizontal_padding;
        self.tab_key = tab_key;
        self.scroll_coordinator = Some(scroll_coordinator.downgrade());
        self.top_inset = top_inset;
        self.bottom_inset = bottom_inset;
        if identity_changed {
            self.unbind_scroll_view();
            cx.notify();
        }
        if !inputs_changed {
            self.observe_scroll_view(cx);
            return;
        }
        self.bind_scroll_view_if_needed(cx);
        self.update_scrubber(false, cx);
    }

    pub fn invalidate(&mut self) {
        self.unbind_scroll_view();
        self.scrub_commit_work_item = None;
        self.scroll_coordinator = None;
    }

    fn bind_scroll_view_if_needed(&mut self, cx: &mut Context<Self>) {
        let candidate = self
            .scroll_coordinator
            .as_ref()
            .and_then(|coordinator| coordinator.upgrade())
            .and_then(|coordinator| coordinator.read(cx).inner_scroll_view(self.tab_key));
        let Some(candidate) = candidate else {
            self.schedule_binding_retry(cx);
            return;
        };
        if self.scroll_view.is_some() {
            return;
        }
        self.unbind_scroll_view();
        self.scroll_view_scrub_route.attach(&candidate);
        self.observed = Some(ObservedScrollGeometry::read(&candidate));
        self.scroll_view = Some(candidate);
        self.update_scrubber(false, cx);
        cx.notify();
    }

    fn schedule_binding_retry(&mut self, cx: &mut Context<Self>) {
        if self.bind_work_item.is_some() {
            return;
        }
        self.bind_work_item = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RETRY_AND_COMMIT_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.bind_work_item = None;
                this.bind_scroll_view_if_needed(cx);
            });
        }));
    }

    fn unbind_scroll_view(&mut self) {
        self.bind_work_item = None;
        self.observed = None;
        self.scroll_view_scrub_route.detach();
        self.scroll_view = None;
    }

    /// watches content offset, content size and bounds
    /// an offset move counts as activity and a size change doesnt
    fn observe_scroll_view(&mut self, cx: &mut Context<Self>) {
        let Some(scroll_view) = self.scroll_view.as_ref() else {
            return;
        };
        let current = ObservedScrollGeometry::read(scroll_view);
        let Some(previous) = self.observed.replace(current) else {
            return;
        };
        if previous == current {
            return;
        }
        let offset_moved = previous.content_offset_y != current.content_offset_y;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| this.update_scrubber(offset_moved, cx));
        })
        .detach();
    }

    fn begin_scrubbing(&mut self, cx: &mut Context<Self>) {
        let tab_key = self.tab_key;
        let metrics = self
            .scroll_coordinator
            .as_ref()
            .and_then(|coordinator| coordinator.upgrade())
            .and_then(|coordinator| {
                coordinator.update(cx, |coordinator, cx| coordinator.begin_inner_date_scrub(tab_key, cx))
            });
        let Some((offset, range)) = metrics else {
            return;
        };
        if range <= 0.0 {
            return;
        }
        self.scrub_initial_offset = offset;
        self.scrub_range = range;
        self.is_scrubbing = true;
        self.pending_scrub_commit = None;
        self.scrub_commit_work_item = None;
    }

    fn scrub_by(&mut self, relative_translation: f32, cx: &mut Context<Self>) {
        if !self.is_scrubbing {
            return;
        }
        let rail_distance = (self.bounds_height.get() - self.top_inset - self.bottom_inset - 44.0).max(44.0);
        let content_per_point = self.scrub_range / rail_distance;
        self.pending_scrub_commit = Some(
            self.scrub_range
                .min((self.scrub_initial_offset + relative_translation * content_per_point).max(0.0)),
        );
        self.schedule_latest_scrub_commit(cx);
    }

    fn end_scrubbing(&mut self, cx: &mut Context<Self>) {
        if !self.is_scrubbing {
            return;
        }
        self.scrub_commit_work_item = None;
        self.commit_latest_scrub_target(cx);
        self.is_scrubbing = false;
        let tab_key = self.tab_key;
        if let Some(coordinator) = self.scroll_coordinator.as_ref().and_then(|coordinator| coordinator.upgrade()) {
            coordinator.update(cx, |coordinator, cx| coordinator.end_inner_date_scrub(tab_key, cx));
        }
        self.update_scrubber(false, cx);
        self.scrubber.update(cx, |scrubber, cx| scrubber.hide_soon(cx));
    }

    /// one commit per 50 ms with the latest target
    /// a pointer makes far more events than a grid can re lay out
    /// and every commit moves the content offset of a live scroll view
    fn schedule_latest_scrub_commit(&mut self, cx: &mut Context<Self>) {
        if self.scrub_commit_work_item.is_some() {
            return;
        }
        self.scrub_commit_work_item = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RETRY_AND_COMMIT_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.scrub_commit_work_item = None;
                this.commit_latest_scrub_target(cx);
                if this.pending_scrub_commit.is_some() && this.is_scrubbing {
                    this.schedule_latest_scrub_commit(cx);
                }
            });
        }));
    }

    fn commit_latest_scrub_target(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.pending_scrub_commit.take() else {
            return;
        };
        let tab_key = self.tab_key;
        if let Some(coordinator) = self.scroll_coordinator.as_ref().and_then(|coordinator| coordinator.upgrade()) {
            coordinator.update(cx, |coordinator, cx| {
                coordinator.update_inner_date_scrub(target, tab_key, cx);
            });
        }
        self.update_scrubber(true, cx);
    }

    fn update_scrubber(&mut self, is_active: bool, cx: &mut Context<Self>) {
        let Some(scroll_view) = self.scroll_view.clone() else {
            self.hide_scrubber(cx);
            return;
        };

        let minimum = 0.0_f32;
        let bounds = scroll_view.bounds().size;
        let bounds_height = f32::from(bounds.height);
        let max_offset = f32::from(scroll_view.max_offset().y).max(0.0);
        let content_height = bounds_height + max_offset;
        let maximum = minimum.max(max_offset);
        let range = (maximum - minimum).max(0.0);
        let content_offset_y = -f32::from(scroll_view.offset().y);
        let logical_offset = range.min((content_offset_y - minimum).max(0.0));
        let is_available =
            self.timestamps.len() > 3 && range > 44.0 && bounds_height / content_height.max(1.0) < 0.72;

        if !is_available {
            self.hide_scrubber(cx);
            return;
        }

        let column_count = self
            .columns_for_width
            .map_or(self.columns, |columns_for_width| columns_for_width(f32::from(bounds.width)))
            .max(1);
        let columns = column_count as f32;
        let usable_width = (f32::from(bounds.width) - self.horizontal_padding * 2.0).max(1.0);
        let cell_side = ((usable_width - self.interitem_spacing * (columns - 1.0)) / columns).max(1.0);
        let row_height = (cell_side / self.cell_aspect + self.interitem_spacing).max(1.0);
        let row = ((logical_offset / row_height).floor() as i64).max(0);
        let index = (self.timestamps.len() as i64 - 1).min(row * column_count);
        let date_text = self.formatted_month(index);
        let (top_inset, bottom_inset, is_scrubbing) = (self.top_inset, self.bottom_inset, self.is_scrubbing);
        self.scrubber.update(cx, |scrubber, cx| {
            scrubber.update(
                logical_offset / range.max(1.0),
                date_text,
                top_inset,
                bottom_inset,
                true,
                is_active || is_scrubbing,
                cx,
            );
        });
    }

    fn hide_scrubber(&mut self, cx: &mut Context<Self>) {
        let (top_inset, bottom_inset) = (self.top_inset, self.bottom_inset);
        self.scrubber.update(cx, |scrubber, cx| {
            scrubber.update(0.0, String::new(), top_inset, bottom_inset, false, false, cx);
        });
    }

    fn formatted_month(&mut self, index: i64) -> String {
        let timestamp = usize::try_from(index)
            .ok()
            .and_then(|index| self.timestamps.get(index).copied())
            .filter(|timestamp| *timestamp > 0.0);
        let Some(timestamp) = timestamp else {
            self.last_month_key = None;
            self.last_date_text = String::new();
            return String::new();
        };
        let date = Date::from_time_interval_since_1970(timestamp);
        let calendar = Calendar::current();
        let components = calendar.date_components(&[CalendarComponent::Year, CalendarComponent::Month], &date);
        let month_key = components.year.unwrap_or(0) * 100 + components.month.unwrap_or(0);
        if self.last_month_key == Some(month_key) {
            return self.last_date_text.clone();
        }
        self.last_month_key = Some(month_key);
        let locale = RiseFormat::app_locale();
        let raw = RiseFormat::date_formatter(&tr("date_month_year_format"), &locale, &calendar, None).string_from(&date);
        self.last_date_text = uppercased_first(&raw);
        self.last_date_text.clone()
    }
}

fn uppercased_first(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl Render for ProfileGridDateScrubberHostView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.observe_scroll_view(cx);
        let bounds_height = self.bounds_height.clone();
        let bounds_width = self.bounds_width.clone();
        let this = cx.entity().downgrade();
        let view_id = ElementId::View(cx.entity_id());
        let measured = measure(
            ElementId::NamedChild(std::sync::Arc::new(view_id.clone()), "bounds".into()),
            move |bounds, _, cx| {
                let (height, width) = (f32::from(bounds.size.height), f32::from(bounds.size.width));
                if bounds_height.get() == height && bounds_width.get() == width {
                    return;
                }
                bounds_height.set(height);
                bounds_width.set(width);
                let _ = this.update(cx, |_, cx| cx.notify());
            },
        );
        let content = div()
            .relative()
            .size_full()
            .child(measured)
            .child(div().absolute().inset_0().child(self.scrubber.clone()));
        if !self.scroll_view_scrub_route.is_attached() {
            return content.into_any_element();
        }
        let route = self.scroll_view_scrub_route.clone();
        let changed_route = route.clone();
        let gesture = DragGesture::new(PAN_HYSTERESIS)
            .on_changed(move |value, window, cx| changed_route.handle_pan_changed(value, window, cx))
            .on_ended(move |_, window, cx| route.handle_pan_ended(window, cx));
        content
            .child(
                div().absolute().inset_0().child(
                    div()
                        .w(px(self.bounds_width.get()))
                        .h(px(self.bounds_height.get()))
                        .simultaneous_gesture(
                            ElementId::NamedChild(std::sync::Arc::new(view_id), "scrub-pan".into()),
                            gesture,
                            GestureMask::All,
                        ),
                ),
            )
            .into_any_element()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProfileGridScrubPanPhase {
    Possible,
    Tracking,
    Failed,
}

/// drags the handle from a pan over the grid
/// accepts only drags that start on the faded handle so clicks and scrolls elsewhere never meet it
/// a click on a photo under the handle still lands, pan claims nothing until it begins
#[derive(Clone)]
struct ProfileGridScrubPanRoute {
    scrubber: WeakEntity<MediaPickerDateScrubber>,
    includes_visible_handle: bool,
    is_attached: Rc<Cell<bool>>,
    phase: Rc<Cell<ProfileGridScrubPanPhase>>,
}

impl ProfileGridScrubPanRoute {
    fn new(scrubber: WeakEntity<MediaPickerDateScrubber>, includes_visible_handle: bool) -> Self {
        Self {
            scrubber,
            includes_visible_handle,
            is_attached: Rc::new(Cell::new(false)),
            phase: Rc::new(Cell::new(ProfileGridScrubPanPhase::Possible)),
        }
    }

    fn attach(&self, _scroll_view: &ScrollHandle) {
        if self.is_attached.get() {
            return;
        }
        self.detach();
        self.is_attached.set(true);
    }

    fn detach(&self) {
        self.is_attached.set(false);
        self.phase.set(ProfileGridScrubPanPhase::Possible);
    }

    fn is_attached(&self) -> bool {
        self.is_attached.get()
    }

    fn handle_pan(&self, pan: MediaPickerDateScrubberPan, window: &mut Window, cx: &mut App) {
        let Some(scrubber) = self.scrubber.upgrade() else {
            return;
        };
        scrubber.update(cx, |scrubber, cx| scrubber.handle_scrub_pan(pan, window, cx));
    }

    fn handle_pan_changed(&self, value: &DragGestureValue, window: &mut Window, cx: &mut App) {
        match self.phase.get() {
            ProfileGridScrubPanPhase::Failed => {}
            ProfileGridScrubPanPhase::Tracking => {
                self.handle_pan(MediaPickerDateScrubberPan::Changed(f32::from(value.translation.height)), window, cx);
            }
            ProfileGridScrubPanPhase::Possible => {
                if !self.gesture_recognizer_should_receive(value.start_location, cx)
                    || !Self::gesture_recognizer_should_begin(value.translation)
                {
                    self.phase.set(ProfileGridScrubPanPhase::Failed);
                    return;
                }
                self.phase.set(ProfileGridScrubPanPhase::Tracking);
                self.handle_pan(MediaPickerDateScrubberPan::Began, window, cx);
                self.handle_pan(MediaPickerDateScrubberPan::Changed(f32::from(value.translation.height)), window, cx);
            }
        }
    }

    fn handle_pan_ended(&self, window: &mut Window, cx: &mut App) {
        let was_tracking = self.phase.get() == ProfileGridScrubPanPhase::Tracking;
        self.phase.set(ProfileGridScrubPanPhase::Possible);
        if was_tracking {
            self.handle_pan(MediaPickerDateScrubberPan::Ended, window, cx);
        }
    }

    fn gesture_recognizer_should_receive(&self, point: Point<Pixels>, cx: &App) -> bool {
        let Some(scrubber) = self.scrubber.upgrade() else {
            return false;
        };
        let scrubber = scrubber.read(cx);
        if scrubber.is_on_faded_handle(point) {
            return true;
        }
        self.includes_visible_handle && scrubber.point_inside(point)
    }

    fn gesture_recognizer_should_begin(translation: Size<Pixels>) -> bool {
        translation.height.abs() >= translation.width.abs()
    }
}
