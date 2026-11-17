use std::ops::{Range, RangeInclusive};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, Context, Div, ElementId, Entity, IntoElement, ParentElement, Pixels, RenderOnce,
    Styled, Window, canvas, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::swiftui::environment::{EnvKey, env};

use crate::modules::gifts::components::gift_cosmetic_playback::GiftFramePacer;

pub struct GiftShopCatalogIsActiveKey;

impl EnvKey for GiftShopCatalogIsActiveKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        true
    }
}

pub fn gift_shop_catalog_is_active(cx: &App) -> bool {
    env::<GiftShopCatalogIsActiveKey>(cx)
}

type GiftShopShelfCell<Item> = Rc<dyn Fn(&Item, &mut Window, &mut App) -> AnyElement>;

/// shop shelf grid that builds only the rows near the sheet viewport
/// shelves are pager pages sized to content and a lazy grid there measured every row, so the
/// shop built every gift at once
#[derive(IntoElement)]
pub struct GiftShopShelfGrid<Item: 'static> {
    id: ElementId,
    pub items: Vec<Item>,
    pub columns: i64,
    pub spacing: f32,
    pub initial_rows: i64,
    template: Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>,
    cell: GiftShopShelfCell<Item>,
    fixed_cells: Option<GiftShopShelfFixedCells>,
}

#[derive(Default)]
pub struct GiftShopShelfGridState {
    mounted: Option<GiftShopShelfRows>,
}

impl<Item: 'static> GiftShopShelfGrid<Item> {
    pub fn new<Template: IntoElement, Cell: IntoElement>(
        id: impl Into<ElementId>,
        items: Vec<Item>,
        columns: i64,
        template: impl FnOnce(&mut Window, &mut App) -> Template + 'static,
        cell: impl Fn(&Item, &mut Window, &mut App) -> Cell + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            items,
            columns,
            spacing: 10.0,
            initial_rows: 1,
            fixed_cells: None,
            template: Box::new(move |window, cx| template(window, cx).into_any_element()),
            cell: Rc::new(move |item, window, cx| cell(item, window, cx).into_any_element()),
        }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// a grid whose cells all have one known size needs no template and no track sizing
    /// the items tab renders every scroll frame and min content rows laid out every built cell again
    pub fn fixed_cells(mut self, fixed_cells: GiftShopShelfFixedCells) -> Self {
        self.fixed_cells = Some(fixed_cells);
        self
    }

    pub fn initial_rows(mut self, initial_rows: i64) -> Self {
        self.initial_rows = initial_rows;
        self
    }

    fn follow(
        state: &Entity<GiftShopShelfGridState>,
        band: Option<RangeInclusive<f32>>,
        height: f32,
        row_count: i64,
        spacing: f32,
        initial_rows: i64,
        cx: &mut App,
    ) {
        if !gift_shop_catalog_is_active(cx) {
            return;
        }
        let Some(band) = band else { return };
        if height <= 0.0 {
            return;
        }
        state.update(cx, |state, cx| {
            let next = GiftShopShelfRows::following(
                &band,
                height,
                row_count,
                spacing,
                state.mounted.unwrap_or(GiftShopShelfRows::leading(initial_rows)),
            );
            if Some(next) != state.mounted {
                state.mounted = Some(next);
                cx.notify();
            }
        });
    }

    /// one step beside the pager, true once the rows are where follow would put them
    fn catch_up(
        state: &Entity<GiftShopShelfGridState>,
        band: Option<RangeInclusive<f32>>,
        height: f32,
        row_count: i64,
        spacing: f32,
        initial_rows: i64,
        cx: &mut App,
    ) -> bool {
        if !gift_shop_catalog_is_active(cx) {
            return true;
        }
        let Some(band) = band else { return true };
        if height <= 0.0 {
            return true;
        }
        state.update(cx, |state, cx| {
            let (rows, reached) = GiftShopShelfRows::stepping(
                &band,
                height,
                row_count,
                spacing,
                state.mounted.unwrap_or(GiftShopShelfRows::leading(initial_rows)),
            );
            if Some(rows) != state.mounted {
                state.mounted = Some(rows);
                cx.notify();
            }
            reached
        })
    }
}

impl<Item: 'static> RenderOnce for GiftShopShelfGrid<Item> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _: &mut Context<GiftShopShelfGridState>| {
            GiftShopShelfGridState::default()
        });
        let item_count = self.items.len() as i64;
        let row_count = GiftShopShelfRows::count(item_count, self.columns);
        let is_catalog_active = gift_shop_catalog_is_active(cx);
        let rows = if is_catalog_active {
            state
                .read(cx)
                .mounted
                .unwrap_or(GiftShopShelfRows::leading(self.initial_rows))
                .clamped(row_count)
        } else {
            GiftShopShelfRows::NONE
        };
        let range = rows.items(self.columns, item_count);
        let spacing = self.spacing;
        let initial_rows = self.initial_rows;

        let cells: Vec<AnyElement> = self.items[range.start as usize..range.end as usize]
            .iter()
            .map(|item| (self.cell)(item, window, cx))
            .collect();
        let grid = match self.fixed_cells {
            Some(fixed_cells) => fixed_cells.place_subviews(self.columns, row_count, spacing, range.start, cells),
            None => {
                let layout = GiftShopShelfGridLayout {
                    columns: self.columns,
                    spacing,
                    item_count,
                    first_index: range.start,
                };
                let template = (self.template)(window, cx);
                layout.place_subviews(layout.size_that_fits(), template, cells)
            }
        };

        let follow_state = state.clone();
        let catch_up_state = state;
        let probe = GiftScrollWindowProbe::new(
            self.id.clone(),
            move |band, height, cx| {
                Self::follow(&follow_state, band, height, row_count, spacing, initial_rows, cx);
            },
        )
        .on_catch_up(move |band, height, cx| {
            Self::catch_up(&catch_up_state, band, height, row_count, spacing, initial_rows, cx)
        });
        grid.child(div().absolute().inset_0().child(probe))
    }
}

pub struct GiftShopShelfGridLayout {
    pub columns: i64,
    pub spacing: f32,
    pub item_count: i64,
    pub first_index: i64,
}

impl GiftShopShelfGridLayout {
    pub fn size_that_fits(&self) -> Div {
        let grid = div().relative().w_full().grid().gap(px(self.spacing));
        let rows = GiftShopShelfRows::count(self.item_count, self.columns);
        if rows <= 0 {
            return grid;
        }
        grid.grid_cols(u16::try_from(self.columns.max(1)).unwrap_or(u16::MAX))
            .grid_rows_min_content(u16::try_from(rows).unwrap_or(u16::MAX))
    }

    pub fn place_subviews(&self, grid: Div, template: AnyElement, cells: Vec<AnyElement>) -> Div {
        let columns = self.columns.max(1);
        if GiftShopShelfRows::count(self.item_count, self.columns) <= 0 {
            return grid;
        }
        let mut grid = grid.child(div().row_start(1).col_start(1).invisible().child(template));
        for (offset, cell) in cells.into_iter().enumerate() {
            let index = self.first_index + offset as i64;
            let row = i16::try_from(index / columns + 1).unwrap_or(i16::MAX);
            let column = i16::try_from(index % columns + 1).unwrap_or(i16::MAX);
            grid = grid.child(div().row_start(row).col_start(column).min_w_0().child(cell));
        }
        grid
    }
}

/// a shelf whose every cell is column_width by row_height
/// grid is as tall as its rows and gaps and each cell sits absolutely at its row and column
/// so no tracks get sized
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GiftShopShelfFixedCells {
    pub column_width: f32,
    pub row_height: f32,
}

impl GiftShopShelfFixedCells {
    fn place_subviews(&self, columns: i64, row_count: i64, spacing: f32, first_index: i64, cells: Vec<AnyElement>) -> Div {
        let columns = columns.max(1);
        let height = if row_count <= 0 {
            0.0
        } else {
            row_count as f32 * self.row_height + (row_count - 1) as f32 * spacing
        };
        let mut grid = div().relative().w_full().h(px(height));
        for (offset, cell) in cells.into_iter().enumerate() {
            let index = first_index + offset as i64;
            let (row, column) = (index / columns, index % columns);
            grid = grid.child(
                div()
                    .absolute()
                    .left(px(column as f32 * (self.column_width + spacing)))
                    .top(px(row as f32 * (self.row_height + spacing)))
                    .w(px(self.column_width))
                    .h(px(self.row_height))
                    .child(cell),
            );
        }
        grid
    }
}

/// the built rows of a shelf both ends included, first greater than last builds none
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GiftShopShelfRows {
    pub first: i64,
    pub last: i64,
}

impl GiftShopShelfRows {
    pub const NONE: GiftShopShelfRows = GiftShopShelfRows { first: 0, last: -1 };

    const DROP_STEP: i64 = 2;

    pub fn leading(count: i64) -> GiftShopShelfRows {
        GiftShopShelfRows {
            first: 0,
            last: count - 1,
        }
    }

    pub fn count(items: i64, columns: i64) -> i64 {
        if items <= 0 || columns <= 0 {
            return 0;
        }
        (items + columns - 1) / columns
    }

    pub fn is_empty(&self) -> bool {
        self.first > self.last
    }

    pub fn clamped(&self, row_count: i64) -> GiftShopShelfRows {
        let upper = self.last.min(row_count - 1);
        if self.first <= upper {
            GiftShopShelfRows {
                first: self.first,
                last: upper,
            }
        } else {
            Self::NONE
        }
    }

    pub fn items(&self, columns: i64, count: i64) -> Range<i64> {
        if self.is_empty() {
            return 0..0;
        }
        let lower = count.min(self.first * columns);
        lower..lower.max(count.min((self.last + 1) * columns))
    }

    /// rows within two of the visible band are built so a fling meets decoded posters
    /// a built row stays until it is four rows out so a scroll resting on an edge doesnt build
    /// and drop the same row
    pub fn following(
        band: &RangeInclusive<f32>,
        height: f32,
        row_count: i64,
        spacing: f32,
        current: GiftShopShelfRows,
    ) -> GiftShopShelfRows {
        if row_count <= 0 || height <= 0.0 {
            return Self::NONE;
        }
        let stride = (height + spacing) / row_count as f32;
        if stride <= 0.0 {
            return Self::NONE;
        }
        let rows = |margin: f32| -> GiftShopShelfRows {
            let top = (band.start() - margin).max(0.0);
            let bottom = height.min(band.end() + margin);
            if top >= bottom {
                return Self::NONE;
            }
            GiftShopShelfRows {
                first: (row_count - 1).min((top / stride) as i64),
                last: (row_count - 1).min((bottom / stride) as i64),
            }
        };
        let needed = rows(2.0 * stride);
        let kept = current.clamped(row_count).intersection(rows(4.0 * stride));
        if kept.is_empty() {
            return needed;
        }
        if needed.is_empty() {
            return kept;
        }
        GiftShopShelfRows {
            first: needed.first.min(kept.first),
            last: needed.last.max(kept.last),
        }
    }

    /// one step from current towards what following wants, for a shelf beside the pager
    /// builds one row per step nearest the band middle first and drops up to DROP_STEP each end
    pub fn stepping(
        band: &RangeInclusive<f32>,
        height: f32,
        row_count: i64,
        spacing: f32,
        current: GiftShopShelfRows,
    ) -> (GiftShopShelfRows, bool) {
        let now = current.clamped(row_count);
        let target = Self::following(band, height, row_count, spacing, now);
        if now == target {
            return (target, true);
        }
        if target.is_empty() || row_count <= 0 {
            let next = now.trimmed(Self::DROP_STEP);
            return (next, next.is_empty());
        }
        if now.is_empty() {
            let start = Self::middle_row(band, height, row_count, spacing, target);
            return (start, start == target);
        }
        if now.last < target.first || now.first > target.last {
            if now.last - now.first + 1 <= 2 * Self::DROP_STEP {
                let start = Self::middle_row(band, height, row_count, spacing, target);
                return (start, start == target);
            }
            return (now.trimmed(Self::DROP_STEP), false);
        }
        let grows_up = now.first > target.first;
        let grows_down = now.last < target.last;
        if grows_up || grows_down {
            let middle = Self::middle_row(band, height, row_count, spacing, target).first;
            let up = now.first - 1;
            let down = now.last + 1;
            let takes_up = grows_up && (!grows_down || (up - middle).abs() < (down - middle).abs());
            let next = if takes_up {
                GiftShopShelfRows {
                    first: up,
                    last: now.last,
                }
            } else {
                GiftShopShelfRows {
                    first: now.first,
                    last: down,
                }
            };
            return (next, next == target);
        }
        let next = GiftShopShelfRows {
            first: target.first.min(now.first + Self::DROP_STEP),
            last: target.last.max(now.last - Self::DROP_STEP),
        };
        (next, next == target)
    }

    /// row at the middle of the band, within target as a window of its own
    fn middle_row(
        band: &RangeInclusive<f32>,
        height: f32,
        row_count: i64,
        spacing: f32,
        target: GiftShopShelfRows,
    ) -> GiftShopShelfRows {
        let stride = (height + spacing) / row_count.max(1) as f32;
        let middle = if stride > 0.0 {
            (((band.start() + band.end()) / 2.0).max(0.0) / stride) as i64
        } else {
            target.first
        };
        let row = middle.max(target.first).min(target.last);
        GiftShopShelfRows { first: row, last: row }
    }

    /// without count rows at each end, none once nothing is left between them
    fn trimmed(&self, count: i64) -> GiftShopShelfRows {
        let lower = self.first + count;
        let upper = self.last - count;
        if lower <= upper {
            GiftShopShelfRows {
                first: lower,
                last: upper,
            }
        } else {
            Self::NONE
        }
    }

    fn intersection(&self, other: GiftShopShelfRows) -> GiftShopShelfRows {
        if self.is_empty() || other.is_empty() {
            return Self::NONE;
        }
        let lower = self.first.max(other.first);
        let upper = self.last.min(other.last);
        if lower <= upper {
            GiftShopShelfRows {
                first: lower,
                last: upper,
            }
        } else {
            Self::NONE
        }
    }
}

type GiftScrollWindowProbeOnChange = Rc<dyn Fn(Option<RangeInclusive<f32>>, f32, &mut App)>;
type GiftScrollWindowProbeOnCatchUp = Rc<dyn Fn(Option<RangeInclusive<f32>>, f32, &mut App) -> bool>;

/// reports which vertical band of this view the scroll views around show, in its own coordinates
/// nil out of a window, the band may lie outside so a caller can build what comes in
/// horizontal position is ignored, on_catch_up answers once scroll stops
#[derive(IntoElement)]
pub struct GiftScrollWindowProbe {
    id: ElementId,
    pub on_catch_up: Option<GiftScrollWindowProbeOnCatchUp>,
    pub on_change: GiftScrollWindowProbeOnChange,
}

impl GiftScrollWindowProbe {
    pub fn new(
        id: impl Into<ElementId>,
        on_change: impl Fn(Option<RangeInclusive<f32>>, f32, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            on_catch_up: None,
            on_change: Rc::new(on_change),
        }
    }

    pub fn on_catch_up(
        mut self,
        on_catch_up: impl Fn(Option<RangeInclusive<f32>>, f32, &mut App) -> bool + 'static,
    ) -> Self {
        self.on_catch_up = Some(Rc::new(on_catch_up));
        self
    }
}

impl GiftScrollWindowProbe {
    fn make_ui_view() -> GiftScrollWindowProbeView {
        GiftScrollWindowProbeView::default()
    }

    fn update_ui_view(&self, view: &mut GiftScrollWindowProbeView) {
        view.on_catch_up = self.on_catch_up.clone();
        view.on_change = Some(self.on_change.clone());
    }
}

impl RenderOnce for GiftScrollWindowProbe {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let probe = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(self.id.clone()), "gift-scroll-window-probe".into()),
            cx,
            |_, _: &mut Context<GiftScrollWindowProbeView>| Self::make_ui_view(),
        );
        probe.update(cx, |view, _| self.update_ui_view(view));
        canvas(
            move |bounds, window, cx| {
                let measure = GiftScrollWindowProbeMeasure::new(bounds, window.content_mask().bounds);
                let changed = probe.update(cx, |view, _| {
                    if view.measure.as_ref() == Some(&measure) {
                        return false;
                    }
                    view.measure = Some(measure);
                    true
                });
                if changed {
                    cx.defer(move |cx| probe.update(cx, |view, cx| view.layout_subviews(cx)));
                }
            },
            |_, _, _, _| {},
        )
        .size_full()
    }
}

/// where the probe sits against its clip after one layout, in its own coordinates
#[derive(Clone, Debug, PartialEq)]
struct GiftScrollWindowProbeMeasure {
    band: Option<RangeInclusive<f32>>,
    height: f32,
    pages_beside: Option<i64>,
}

impl GiftScrollWindowProbeMeasure {
    fn new(bounds: Bounds<Pixels>, shown: Bounds<Pixels>) -> Self {
        let top = f32::from(shown.top() - bounds.top());
        let bottom = f32::from(shown.bottom() - bounds.top());
        let band = (top.is_finite() && bottom.is_finite() && top < bottom).then_some(top..=bottom);
        let is_beside = shown.right() <= bounds.left() + px(1.0) || shown.left() >= bounds.right() - px(1.0);
        let pages_beside = is_beside.then(|| {
            let distance = f32::from((shown.center().x - bounds.center().x).abs());
            let width = f32::from(shown.size.width).max(1.0);
            ((distance / width).round() as i64).max(1)
        });
        Self {
            band,
            height: f32::from(bounds.size.height),
            pages_beside,
        }
    }
}

#[derive(Clone, PartialEq)]
struct GiftScrollWindowProbeViewReport {
    band: Option<RangeInclusive<f32>>,
    height: f32,
}

#[derive(Default)]
pub struct GiftScrollWindowProbeView {
    on_change: Option<GiftScrollWindowProbeOnChange>,
    on_catch_up: Option<GiftScrollWindowProbeOnCatchUp>,
    measure: Option<GiftScrollWindowProbeMeasure>,
    last_report: Option<GiftScrollWindowProbeViewReport>,
    last_side_tick: Duration,
    pages_away: i64,
    is_catching_up: bool,
    was_beside: bool,
}

impl GiftScrollWindowProbeView {
    const QUIET_INTERVAL: Duration = Duration::from_millis(120);

    fn layout_subviews(&mut self, cx: &mut Context<Self>) {
        self.report_now_or_when_quiet(cx);
    }

    fn visible_band(&self) -> Option<RangeInclusive<f32>> {
        self.measure.as_ref().and_then(|measure| measure.band.clone())
    }

    fn height(&self) -> f32 {
        self.measure.as_ref().map_or(0.0, |measure| measure.height)
    }

    fn report_now_or_when_quiet(&mut self, cx: &mut Context<Self>) {
        let away = self.measure.as_ref().and_then(|measure| measure.pages_beside);
        let Some(away) = away.filter(|_| self.on_catch_up.is_some()) else {
            if self.was_beside {
                self.was_beside = false;
                self.last_report = None;
            }
            self.report(cx);
            return;
        };
        self.was_beside = true;
        self.pages_away = away;
        self.last_side_tick = RiseClock::monotonic();
        if self.is_catching_up {
            return;
        }
        self.is_catching_up = true;
        self.await_quiet(cx);
    }

    fn await_quiet(&mut self, cx: &mut Context<Self>) {
        let elapsed = RiseClock::monotonic().saturating_sub(self.last_side_tick);
        let wait = Self::QUIET_INTERVAL.saturating_sub(elapsed).max(Duration::from_millis(10));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |view, cx| view.queue_catch_up_step(cx));
        })
        .detach();
    }

    fn is_quiet(&self) -> bool {
        RiseClock::monotonic().saturating_sub(self.last_side_tick) >= Self::QUIET_INTERVAL
    }

    fn queue_catch_up_step(&mut self, cx: &mut Context<Self>) {
        if self.on_catch_up.is_none() {
            self.is_catching_up = false;
            return;
        }
        if !self.is_quiet() {
            self.await_quiet(cx);
            return;
        }
        let this = cx.weak_entity();
        let distance = self.pages_away;
        GiftFramePacer::shared(cx).update(cx, |pacer, cx| {
            pacer.enqueue_catch_up(
                distance,
                move |cx: &mut App| {
                    let _ = this.update(cx, |view, cx| view.catch_up_step(cx));
                },
                cx,
            );
        });
    }

    fn catch_up_step(&mut self, cx: &mut Context<Self>) {
        let Some(on_catch_up) = self.on_catch_up.clone() else {
            self.is_catching_up = false;
            return;
        };
        if !self.is_quiet() {
            self.await_quiet(cx);
            return;
        }
        let band = self.visible_band();
        let height = self.height();
        if on_catch_up(band, height, cx) {
            self.is_catching_up = false;
        } else {
            self.queue_catch_up_step(cx);
        }
    }

    fn report(&mut self, cx: &mut Context<Self>) {
        let Some(on_change) = self.on_change.clone() else { return };
        let next = GiftScrollWindowProbeViewReport {
            band: self.visible_band(),
            height: self.height(),
        };
        if self.last_report.as_ref() == Some(&next) {
            return;
        }
        self.last_report = Some(next.clone());
        on_change(next.band, next.height, cx);
    }
}
