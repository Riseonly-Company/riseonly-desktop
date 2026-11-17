use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Context, ElementId, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render,
    ScrollWheelEvent, StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::wheel_gesture::WHEEL_SETTLE_DELAY;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::environment::LayoutDirection;
use rise_ui::swiftui::motion::ease_out;

use crate::core::media_viewer::media_viewer_presentation_state::MediaViewerPresentationState;
use crate::core::media_viewer::media_viewer_types::{MediaViewerFilmstripMetrics, MediaViewerGalleryItem};
use crate::core::ui::clever_image_ui::CleverImageUi;

pub type MediaViewerFilmstripSelect = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// thumbnail rail under the photo, its own view
/// host redraws on every video tick so the rail only takes its run and open page
/// middle item is the open page, scrubs both ways
pub struct MediaViewerFilmstrip {
    state: Entity<MediaViewerPresentationState>,
    width: Pixels,
    on_select: MediaViewerFilmstripSelect,
    run_range: Range<usize>,
    run: Vec<MediaViewerGalleryItem>,
    current_index: usize,
    centered_item_id: Option<String>,
    scroller: MediaViewerFilmstripScroller,
    cell_heights: Vec<Animated<f32>>,
    cell_opacities: Vec<Animated<f32>>,
    drag_suppresses_click: bool,
}

impl MediaViewerFilmstrip {
    const SELECTION_DURATION: f64 = 0.24;
    const MINIMUM_SIDE_INSET: f32 = 18.0;
    const RESTING_OPACITY: f32 = 0.55;
    const DRAG_MINIMUM_DISTANCE: f32 = 4.0;

    pub fn new(
        state: Entity<MediaViewerPresentationState>,
        width: Pixels,
        on_select: MediaViewerFilmstripSelect,
        cx: &mut Context<Self>,
    ) -> Self {
        let (run_range, run, current_index) = Self::snapshot(&state, cx);
        let centered_item_id = {
            let snapshot = state.read(cx);
            snapshot.items().get(snapshot.current_index()).map(|item| item.id.clone())
        };
        let mut rail = Self {
            state,
            width,
            on_select,
            run_range,
            run,
            current_index,
            centered_item_id,
            scroller: MediaViewerFilmstripScroller::new(),
            cell_heights: Vec::new(),
            cell_opacities: Vec::new(),
            drag_suppresses_click: false,
        };
        rail.rebuild_cells();
        rail.recenter(cx);
        rail
    }

    fn snapshot(
        state: &Entity<MediaViewerPresentationState>,
        cx: &App,
    ) -> (Range<usize>, Vec<MediaViewerGalleryItem>, usize) {
        let snapshot = state.read(cx);
        let range = snapshot.filmstrip_range();
        let run = snapshot.items()[range.clone()].to_vec();
        (range, run, snapshot.current_index())
    }

    pub fn host_did_change(&mut self, width: Pixels, cx: &mut Context<Self>) {
        let (run_range, run, current_index) = Self::snapshot(&self.state, cx);
        let count_changed = run.len() != self.run.len();
        let index_changed = current_index != self.current_index;
        self.width = width;
        self.run_range = run_range;
        self.run = run;
        self.current_index = current_index;
        if count_changed {
            self.rebuild_cells();
            self.recenter(cx);
        } else if index_changed {
            self.current_index_did_change(cx);
        }
        self.sync_cells(true, cx);
        cx.notify();
    }

    fn rebuild_cells(&mut self) {
        self.cell_heights = (0..self.run.len())
            .map(|offset| Animated::new(self.resting_height_for(offset)))
            .collect();
        self.cell_opacities = (0..self.run.len())
            .map(|offset| Animated::new(self.opacity_for(offset)))
            .collect();
    }

    fn is_current(&self, offset: usize) -> bool {
        self.run_range.start + offset == self.current_index
    }

    fn resting_height_for(&self, offset: usize) -> f32 {
        if self.is_current(offset) {
            f32::from(MediaViewerFilmstripMetrics::HEIGHT)
        } else {
            f32::from(MediaViewerFilmstripMetrics::RESTING_HEIGHT)
        }
    }

    fn opacity_for(&self, offset: usize) -> f32 {
        if self.is_current(offset) { 1.0 } else { Self::RESTING_OPACITY }
    }

    fn sync_cells(&mut self, animated: bool, cx: &App) {
        let now = RiseClock::monotonic();
        let spec = animated.then(|| ease_out(Self::SELECTION_DURATION));
        for offset in 0..self.run.len() {
            let height = self.resting_height_for(offset);
            let opacity = self.opacity_for(offset);
            if let Some(cell) = self.cell_heights.get_mut(offset) {
                cell.set_in(height, spec, now, cx);
            }
            if let Some(cell) = self.cell_opacities.get_mut(offset) {
                cell.set_in(opacity, spec, now, cx);
            }
        }
    }

    fn current_item_id(&self) -> Option<String> {
        let offset = self.current_index.checked_sub(self.run_range.start)?;
        self.run.get(offset).map(|item| item.id.clone())
    }

    fn side_inset(&self) -> f32 {
        Self::MINIMUM_SIDE_INSET
            .max((f32::from(self.width) - f32::from(MediaViewerFilmstripMetrics::SLOT_WIDTH)) / 2.0)
    }

    fn geometry(&self) -> MediaViewerFilmstripGeometry {
        MediaViewerFilmstripGeometry {
            count: self.run.len(),
            width: f32::from(self.width),
            side_inset: self.side_inset(),
        }
    }

    fn current_index_did_change(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current_item_id() else { return };
        if self.centered_item_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.centered_item_id = Some(id);
        let offset = self.current_index.saturating_sub(self.run_range.start);
        let geometry = self.geometry();
        self.scroller.center(offset, true, geometry, cx);
    }

    /// a fresh rail rests on the open page from its first frame
    fn recenter(&mut self, cx: &App) {
        let Some(id) = self.current_item_id() else { return };
        self.centered_item_id = Some(id);
        let offset = self.current_index.saturating_sub(self.run_range.start);
        let geometry = self.geometry();
        self.scroller.center(offset, false, geometry, cx);
    }

    /// rail was scrolled or clicked, moves the pager to the centered item
    fn centered_item_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.centered_item_id.clone() else { return };
        if self.current_item_id().as_deref() == Some(id.as_str()) {
            return;
        }
        let index = {
            let snapshot = self.state.read(cx);
            let range = snapshot.filmstrip_range();
            snapshot.items()[range.clone()]
                .iter()
                .position(|item| item.id == id)
                .map(|position| range.start + position)
        };
        let Some(index) = index else { return };
        (self.on_select)(index, window, cx);
    }

    fn select_offset(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.run.get(offset) else { return };
        self.centered_item_id = Some(item.id.clone());
        let geometry = self.geometry();
        self.scroller.center(offset, true, geometry, cx);
        self.centered_item_did_change(window, cx);
        cx.notify();
    }

    fn drag_changed(&mut self, value: &DragGestureValue, window: &mut Window, cx: &mut Context<Self>) {
        let geometry = self.geometry();
        let base = match self.scroller.drag_base {
            Some(base) => base,
            None => {
                let base = self.scroller.current_x();
                self.scroller.drag_base = Some(base);
                base
            }
        };
        self.scroller.drag_to(base - forward_sign() * f32::from(value.translation.width), geometry);
        self.read_center(window, cx);
        cx.notify();
    }

    fn drag_ended(&mut self, value: &DragGestureValue, window: &mut Window, cx: &mut Context<Self>) {
        let Some(base) = self.scroller.drag_base.take() else { return };
        self.drag_suppresses_click = true;
        let geometry = self.geometry();
        let projected = geometry.clamped(base - forward_sign() * f32::from(value.predicted_end_translation.width));
        if let Some(offset) = geometry.centered_offset(projected) {
            self.scroller.center(offset, true, geometry, cx);
            self.read_center(window, cx);
        }
        cx.notify();
    }

    fn drag_gesture(&self, cx: &Context<Self>) -> DragGesture {
        let changed = cx.entity().downgrade();
        let ended = changed.clone();
        DragGesture::new(Self::DRAG_MINIMUM_DISTANCE)
            .on_changed(move |value: &DragGestureValue, window, cx| {
                changed.update(cx, |rail, cx| rail.drag_changed(value, window, cx)).ok();
            })
            .on_ended(move |value: &DragGestureValue, window, cx| {
                ended.update(cx, |rail, cx| rail.drag_ended(value, window, cx)).ok();
            })
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(window.line_height());
        let along = if delta.x.abs() > delta.y.abs() { forward_sign() * f32::from(delta.x) } else { f32::from(delta.y) };
        if along == 0.0 {
            return;
        }
        cx.stop_propagation();
        let geometry = self.geometry();
        self.scroller.scroll_by(-along, geometry);
        self.read_center(window, cx);
        let settle = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(WHEEL_SETTLE_DELAY).await;
            this.update_in(cx, |rail: &mut Self, window, cx| {
                let geometry = rail.geometry();
                if let Some(offset) = rail.scroller.nearest_offset(geometry) {
                    rail.scroller.center(offset, true, geometry, cx);
                    rail.read_center(window, cx);
                }
                cx.notify();
            })
            .ok();
        });
        self.scroller.settle_task = Some(settle);
        cx.notify();
    }

    fn read_center(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let geometry = self.geometry();
        let Some(offset) = self.scroller.nearest_offset(geometry) else { return };
        let Some(id) = self.run.get(offset).map(|item| item.id.clone()) else { return };
        if self.centered_item_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.centered_item_id = Some(id);
        self.centered_item_did_change(window, cx);
    }

    /// the slot is fixed and only what is drawn inside it changes with the selection
    fn cell(&self, offset: usize, item: &MediaViewerGalleryItem, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let height = self
            .cell_heights
            .get(offset)
            .map(|cell| cell.frame_value(now, window))
            .unwrap_or_else(|| self.resting_height_for(offset));
        let opacity = self
            .cell_opacities
            .get(offset)
            .map(|cell| cell.frame_value(now, window))
            .unwrap_or_else(|| self.opacity_for(offset));
        let slot_width = MediaViewerFilmstripMetrics::SLOT_WIDTH;
        let image = CleverImageUi::new(item.display_source())
            .id(ElementId::Name(format!("media-viewer-filmstrip-image-{}", item.id).into()))
            .content_mode(ContentMode::Fill)
            .corner_radius(f32::from(MediaViewerFilmstripMetrics::CORNER_RADIUS))
            .persist_to_disk(item.persist_media_to_disk)
            .shows_progress_indicator(false)
            .maximum_decode_pixel_dimension(MediaViewerFilmstripMetrics::DECODE_PIXEL_DIMENSION)
            .cache_policy(item.image_cache_policy)
            .shows_failure_icon(false);
        div()
            .id(ElementId::Name(format!("media-viewer-filmstrip-cell-{}", item.id).into()))
            .w(slot_width)
            .h(MediaViewerFilmstripMetrics::HEIGHT)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(
                div()
                    .w(slot_width)
                    .h(px(height))
                    .rounded(MediaViewerFilmstripMetrics::CORNER_RADIUS)
                    .overflow_hidden()
                    .opacity(opacity)
                    .child(image),
            )
            .on_click(cx.listener(move |rail: &mut Self, _, window, cx| {
                if std::mem::take(&mut rail.drag_suppresses_click) {
                    return;
                }
                rail.select_offset(offset, window, cx)
            }))
    }
}

impl Render for MediaViewerFilmstrip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let geometry = self.geometry();
        let now = RiseClock::monotonic();
        let scroll_x = self.scroller.content_offset_x.frame_value(now, window);
        let stride = MediaViewerFilmstripGeometry::stride();
        let visible = geometry.visible_offsets(scroll_x);
        let drag = self.drag_gesture(cx);
        let is_rtl = LayoutDirection::current().is_right_to_left();
        let slot_width = f32::from(MediaViewerFilmstripMetrics::SLOT_WIDTH);
        let cells: Vec<_> = visible
            .filter_map(|offset| self.run.get(offset).map(|item| (offset, item.clone())))
            .map(|(offset, item)| {
                let leading = geometry.side_inset + offset as f32 * stride - scroll_x;
                let left = if is_rtl { geometry.width - leading - slot_width } else { leading };
                div()
                    .absolute()
                    .top_0()
                    .left(px(left))
                    .child(self.cell(offset, &item, window, cx))
            })
            .collect();
        div()
            .id("media-viewer-filmstrip")
            .relative()
            .w(self.width)
            .h(MediaViewerFilmstripMetrics::HEIGHT)
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(|rail: &mut Self, event: &ScrollWheelEvent, window, cx| {
                rail.handle_scroll_wheel(event, window, cx)
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|rail: &mut Self, _, _, _| {
                rail.drag_suppresses_click = false;
            }))
            .child(
                div()
                    .relative()
                    .w(self.width)
                    .h(MediaViewerFilmstripMetrics::HEIGHT)
                    .children(cells)
                    .gesture("media-viewer-filmstrip-drag", drag),
            )
    }
}

/// 1 when the rail runs left to right and -1 in rtl where a right swipe moves forward
fn forward_sign() -> f32 {
    if LayoutDirection::current().is_right_to_left() { -1.0 } else { 1.0 }
}

/// where the rail stands in rail content coordinates
#[derive(Clone, Copy, Debug)]
struct MediaViewerFilmstripGeometry {
    count: usize,
    width: f32,
    side_inset: f32,
}

impl MediaViewerFilmstripGeometry {
    fn stride() -> f32 {
        f32::from(MediaViewerFilmstripMetrics::SLOT_WIDTH) + f32::from(MediaViewerFilmstripMetrics::SPACING)
    }

    fn content_width(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        self.side_inset * 2.0 + self.count as f32 * Self::stride() - f32::from(MediaViewerFilmstripMetrics::SPACING)
    }

    fn maximum_offset(&self) -> f32 {
        (self.content_width() - self.width).max(0.0)
    }

    fn clamped(&self, x: f32) -> f32 {
        x.clamp(0.0, self.maximum_offset())
    }

    fn offset_x(&self, index: usize) -> f32 {
        let target = self.side_inset
            + index as f32 * Self::stride()
            + f32::from(MediaViewerFilmstripMetrics::SLOT_WIDTH) / 2.0;
        self.clamped(target - self.width / 2.0)
    }

    fn centered_offset(&self, x: f32) -> Option<usize> {
        if self.count == 0 || self.width <= 0.0 {
            return None;
        }
        let center = x + self.width / 2.0 - self.side_inset;
        let offset = ((center - f32::from(MediaViewerFilmstripMetrics::SLOT_WIDTH) / 2.0) / Self::stride()).round();
        Some((offset.max(0.0) as usize).min(self.count - 1))
    }

    fn visible_offsets(&self, x: f32) -> Range<usize> {
        if self.count == 0 {
            return 0..0;
        }
        let stride = Self::stride();
        let first = ((x - self.side_inset) / stride).floor().max(0.0) as usize;
        let last = ((x + self.width - self.side_inset) / stride).ceil().max(0.0) as usize + 1;
        first.min(self.count)..last.min(self.count)
    }
}

/// centers the rail on a cell by moving its content offset, like a swipe does
pub struct MediaViewerFilmstripScroller {
    content_offset_x: Animated<f32>,
    settle_task: Option<Task<()>>,
    drag_base: Option<f32>,
}

impl MediaViewerFilmstripScroller {
    fn new() -> Self {
        Self {
            content_offset_x: Animated::new(0.0),
            settle_task: None,
            drag_base: None,
        }
    }

    fn center(&mut self, offset: usize, animated: bool, geometry: MediaViewerFilmstripGeometry, cx: &App) {
        if geometry.count == 0 {
            return;
        }
        let index = offset.min(geometry.count - 1);
        let x = geometry.offset_x(index);
        let spec = animated.then(|| ease_out(MediaViewerFilmstrip::SELECTION_DURATION));
        self.content_offset_x.set_in(x, spec, RiseClock::monotonic(), cx);
    }

    fn scroll_by(&mut self, dx: f32, geometry: MediaViewerFilmstripGeometry) {
        self.settle_task = None;
        let current = self.content_offset_x.value(RiseClock::monotonic());
        self.content_offset_x.jump(geometry.clamped(current + dx));
    }

    fn current_x(&self) -> f32 {
        self.content_offset_x.value(RiseClock::monotonic())
    }

    fn drag_to(&mut self, x: f32, geometry: MediaViewerFilmstripGeometry) {
        self.settle_task = None;
        self.content_offset_x.jump(geometry.clamped(x));
    }

    fn nearest_offset(&self, geometry: MediaViewerFilmstripGeometry) -> Option<usize> {
        geometry.centered_offset(*self.content_offset_x.target())
    }
}
