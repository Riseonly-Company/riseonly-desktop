use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AnyElement, App, Context, ElementId, Entity, EntityId, Global, Render, SharedString, Subscription, Window};
use rise_ui::swiftui::prelude::*;

use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerPresentationState, media_viewer_presentation,
};
use crate::core::media_viewer::media_viewer_types::MediaViewerFrameKeys;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseConversationPresentation, RiseConversationPresentationChange,
};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::gifts::components::gift_shop_shelf_grid::{GiftScrollWindowProbe, GiftShopShelfRows};

type ChatProfileTabBody = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// says if the data a tab shows changed since the last answer
type ChatProfileTabGate = Box<dyn FnMut(&mut App) -> bool>;

/// view a tab body renders in
/// store changes rebuild this tab and not the pager around it
pub struct ChatProfileTabHost {
    body: ChatProfileTabBody,
    gate: Option<ChatProfileTabGate>,
    _subscriptions: Vec<Subscription>,
}

impl ChatProfileTabHost {
    /// id must carry every property the body captures, the body is kept from the first render
    pub fn hosted(
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
        observe: impl FnOnce(&mut Context<Self>) -> Vec<Subscription> + 'static,
        body: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Entity<Self> {
        Self::hosted_with(id, window, cx, observe, None::<fn(&mut App) -> ()>, body)
    }

    pub fn hosted_gated<G: PartialEq + 'static>(
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
        observe: impl FnOnce(&mut Context<Self>) -> Vec<Subscription> + 'static,
        shown: impl Fn(&mut App) -> G + 'static,
        body: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Entity<Self> {
        Self::hosted_with(id, window, cx, observe, Some(shown), body)
    }

    fn hosted_with<G: PartialEq + 'static>(
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
        observe: impl FnOnce(&mut Context<Self>) -> Vec<Subscription> + 'static,
        shown: Option<impl Fn(&mut App) -> G + 'static>,
        body: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Entity<Self> {
        window.use_keyed_state(id, cx, move |_, cx| {
            let presentation = RiseConversationPresentation::shared(cx);
            let messages = MessageServicesStore::shared(cx);
            let chats = ChatServicesStore::shared(cx);
            let gate = shown.map(|shown| {
                let mut last = shown(&mut **cx);
                Box::new(move |cx: &mut App| {
                    let next = shown(cx);
                    if next == last {
                        return false;
                    }
                    last = next;
                    true
                }) as ChatProfileTabGate
            });
            let mut subscriptions = vec![
                cx.subscribe(&presentation, |host: &mut ChatProfileTabHost, _, change: &RiseConversationPresentationChange, cx: &mut Context<ChatProfileTabHost>| {
                    if !matches!(
                        change,
                        RiseConversationPresentationChange::UnreadMarks(_)
                            | RiseConversationPresentationChange::GlobalMessageSearch
                            | RiseConversationPresentationChange::MessageSearch
                            | RiseConversationPresentationChange::ChatOverlaySearch
                            | RiseConversationPresentationChange::ChatSearchRecents
                    ) && host.shown_changed(cx)
                    {
                        cx.notify();
                    }
                }),
                cx.observe(&messages, |host: &mut ChatProfileTabHost, _, cx: &mut Context<ChatProfileTabHost>| {
                    if host.shown_changed(cx) {
                        cx.notify();
                    }
                }),
                cx.observe(&chats, |host: &mut ChatProfileTabHost, _, cx: &mut Context<ChatProfileTabHost>| {
                    if host.shown_changed(cx) {
                        cx.notify();
                    }
                }),
                observe_grid_lift(cx),
            ];
            subscriptions.extend(observe(cx));
            Self { body: Rc::new(body), gate, _subscriptions: subscriptions }
        })
    }

    /// ungated hosts rebuild on every publish they hear
    fn shown_changed(&mut self, cx: &mut App) -> bool {
        self.gate.as_mut().is_none_or(|gate| gate(cx))
    }
}

/// profile grid cell hidden under each viewer, written by the tab host observers
/// reading it here keeps the viewer out of the cached page
#[derive(Default)]
struct ChatProfileGridLifts(HashMap<EntityId, Option<String>>);

impl Global for ChatProfileGridLifts {}

fn grid_lift(viewer: &MediaViewerPresentationState) -> Option<String> {
    if !viewer.is_presented() || !viewer.bubble_cell_hidden() {
        return None;
    }
    let prefix = MediaViewerFrameKeys::profile_grid("");
    viewer.hero_lift_id().filter(|id| id.starts_with(prefix.as_str())).map(str::to_owned)
}

fn observe_grid_lift(cx: &mut Context<ChatProfileTabHost>) -> Subscription {
    let viewer = media_viewer_presentation(cx);
    let viewer_id = viewer.entity_id();
    let mut last = grid_lift(viewer.read(cx));
    cx.default_global::<ChatProfileGridLifts>().0.insert(viewer_id, last.clone());
    cx.observe(&viewer, move |_, viewer, cx| {
        let next = grid_lift(viewer.read(cx));
        if next != last {
            last = next.clone();
            cx.default_global::<ChatProfileGridLifts>().0.insert(viewer_id, next);
            cx.notify();
        }
    })
}

/// true when this grid cell is hidden because the media viewer has it open
pub fn chat_profile_grid_cell_lifted(registry_id: &str, cx: &mut App) -> bool {
    let viewer_id = media_viewer_presentation(cx).entity_id();
    cx.try_global::<ChatProfileGridLifts>()
        .and_then(|lifts| lifts.0.get(&viewer_id))
        .and_then(Option::as_deref)
        == Some(registry_id)
}

impl Render for ChatProfileTabHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui::hear_entity_through_events_only(RiseConversationPresentation::shared(cx).entity_id());
        if self.gate.is_some() {
            gpui::hear_entity_through_events_only(MessageServicesStore::shared(cx).entity_id());
            gpui::hear_entity_through_events_only(ChatServicesStore::shared(cx).entity_id());
        }
        let body = self.body.clone();
        body(window, cx)
    }
}

pub(crate) type ChatProfileSquareGridCell = Rc<dyn Fn(usize, f32, &mut Window, &mut App) -> AnyElement>;

/// layout probe under the grid, gets the width inside the horizontal padding
pub(crate) type ChatProfileSquareGridProbe = Rc<dyn Fn(f32, &mut Window, &mut App) -> AnyElement>;

#[derive(Default)]
struct ChatProfileSquareGridState {
    width: f32,
    mounted: Option<GiftShopShelfRows>,
}

/// grid of square cells, height comes from the count
/// so the outer scroll sees every row without building it
#[derive(IntoElement)]
pub struct ChatProfileSquareGrid {
    id: ElementId,
    item_count: usize,
    columns: i64,
    spacing: f32,
    horizontal_padding: f32,
    top_padding: f32,
    cell: ChatProfileSquareGridCell,
    layout_probe: Option<ChatProfileSquareGridProbe>,
}

impl ChatProfileSquareGrid {
    const INITIAL_ROWS: i64 = 8;

    pub fn new(
        id: impl Into<ElementId>,
        item_count: usize,
        columns: i64,
        spacing: f32,
        horizontal_padding: f32,
        cell: ChatProfileSquareGridCell,
    ) -> Self {
        Self {
            id: id.into(),
            item_count,
            columns,
            spacing,
            horizontal_padding,
            top_padding: 0.0,
            cell,
            layout_probe: None,
        }
    }

    pub fn top_padding(mut self, top_padding: f32) -> Self {
        self.top_padding = top_padding;
        self
    }

    pub fn layout_probe(mut self, layout_probe: ChatProfileSquareGridProbe) -> Self {
        self.layout_probe = Some(layout_probe);
        self
    }
}

impl RenderOnce for ChatProfileSquareGrid {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _: &mut Context<ChatProfileSquareGridState>| {
            ChatProfileSquareGridState::default()
        });
        let (width, mounted) = {
            let current = state.read(cx);
            (current.width, current.mounted)
        };
        let columns = self.columns.max(1);
        let spacing = self.spacing;
        let item_count = i64::try_from(self.item_count).unwrap_or(i64::MAX);
        let row_count = GiftShopShelfRows::count(item_count, columns);
        let side = if width > 0.0 { ((width - spacing * (columns - 1) as f32) / columns as f32).max(1.0) } else { 0.0 };
        let height = row_count as f32 * side + (row_count - 1).max(0) as f32 * spacing;
        let range = if side > 0.0 {
            mounted
                .unwrap_or(GiftShopShelfRows::leading(Self::INITIAL_ROWS))
                .clamped(row_count)
                .items(columns, item_count)
        } else {
            0..0
        };

        let mut cells: Vec<AnyElement> = Vec::with_capacity((range.end - range.start).max(0) as usize);
        for index in range {
            let column = index % columns;
            let row = index / columns;
            cells.push(
                div()
                    .absolute()
                    .left(px(column as f32 * (side + spacing)))
                    .top(px(row as f32 * (side + spacing)))
                    .w(px(side))
                    .h(px(side))
                    .child((self.cell)(index as usize, side, window, cx))
                    .into_any_element(),
            );
        }

        let layout_probe = self.layout_probe.map(|probe| probe(width, window, cx));
        let width_state = state.clone();
        let rows_state = state;
        let width_reader = measure(
            ElementId::NamedChild(Arc::new(self.id.clone()), "width-reader".into()),
            move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                width_state.update(cx, |current, cx| {
                    if current.width != width {
                        current.width = width;
                        cx.notify();
                    }
                });
            },
        );
        let window_probe = GiftScrollWindowProbe::new(
            ElementId::NamedChild(Arc::new(self.id), "window-probe".into()),
            move |band, probe_height, cx| {
                let Some(band) = band else { return };
                if probe_height <= 0.0 {
                    return;
                }
                rows_state.update(cx, |current, cx| {
                    let next = GiftShopShelfRows::following(
                        &band,
                        probe_height,
                        row_count,
                        spacing,
                        current.mounted.unwrap_or(GiftShopShelfRows::leading(Self::INITIAL_ROWS)),
                    );
                    if Some(next) != current.mounted {
                        current.mounted = Some(next);
                        cx.notify();
                    }
                });
            },
        );

        div().w_full().px(px(self.horizontal_padding)).pt(px(self.top_padding)).child(
            div()
                .relative()
                .w_full()
                .h(px(height))
                .child(width_reader)
                .children(layout_probe)
                .children(cells)
                .child(div().absolute().inset_0().child(window_probe)),
        )
    }
}

pub struct ChatProfileWindowedRow {
    id: String,
    pinned: bool,
    build: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>,
}

impl ChatProfileWindowedRow {
    /// id is unique in the window and keys the row height too
    pub fn new<E: IntoElement>(
        id: impl Into<String>,
        build: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            pinned: false,
            build: Rc::new(move |window, cx| build(window, cx).into_any_element()),
        }
    }

    /// row stays at the top until the next pinned row
    pub(crate) fn pinned(mut self) -> Self {
        self.pinned = true;
        self
    }
}

#[derive(Default)]
struct ChatProfileWindowedRowsState {
    heights: HashMap<String, f32>,
    band: Option<(f32, f32)>,
}

/// lazy list, rows near the viewport are built and measured
/// the rest are spacers
#[derive(IntoElement)]
pub struct ChatProfileWindowedRows {
    id: ElementId,
    estimated_row_height: f32,
    rows: Vec<ChatProfileWindowedRow>,
}

impl ChatProfileWindowedRows {
    const OVERDRAW: f32 = 480.0;
    const INITIAL_EXTENT: f32 = 1200.0;
    const BAND_QUANTUM: f32 = 64.0;
    const HEIGHT_TOLERANCE: f32 = 0.25;

    pub fn new(
        id: impl Into<ElementId>,
        estimated_row_height: f32,
        rows: Vec<ChatProfileWindowedRow>,
    ) -> Self {
        Self { id: id.into(), estimated_row_height, rows }
    }

    fn note_band(
        current: &mut Option<(f32, f32)>,
        band: &RangeInclusive<f32>,
        follows_every_move: bool,
    ) -> bool {
        let next = (*band.start(), *band.end());
        let changed = current.is_none_or(|(start, end)| {
            let threshold = if follows_every_move { 0.0 } else { Self::BAND_QUANTUM };
            (start - next.0).abs() > threshold || (end - next.1).abs() > threshold
        });
        if changed {
            *current = Some(next);
        }
        changed
    }
}

impl RenderOnce for ChatProfileWindowedRows {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _: &mut Context<ChatProfileWindowedRowsState>| {
            ChatProfileWindowedRowsState::default()
        });
        let estimated = self.estimated_row_height;
        let (band, frames) = {
            let current = state.read(cx);
            let mut y = 0.0;
            let frames: Vec<(f32, f32)> = self
                .rows
                .iter()
                .map(|row| {
                    let height = current.heights.get(&row.id).copied().unwrap_or(estimated);
                    let frame = (y, height);
                    y += height;
                    frame
                })
                .collect();
            (current.band, frames)
        };
        let (top, bottom) = band.unwrap_or((0.0, Self::INITIAL_EXTENT));

        let mut children: Vec<AnyElement> = Vec::with_capacity(self.rows.len());
        for (row, (y, height)) in self.rows.iter().zip(&frames) {
            if y + height < top - Self::OVERDRAW || *y > bottom + Self::OVERDRAW {
                children.push(div().w_full().h(px(*height)).into_any_element());
                continue;
            }
            let reader_state = state.clone();
            let key = row.id.clone();
            let reader = measure(SharedString::from(format!("{}-height", row.id)), move |bounds, _, cx| {
                let measured = f32::from(bounds.size.height);
                reader_state.update(cx, |current, cx| {
                    let is_same = current
                        .heights
                        .get(&key)
                        .is_some_and(|known| (known - measured).abs() < Self::HEIGHT_TOLERANCE);
                    if !is_same {
                        current.heights.insert(key, measured);
                        cx.notify();
                    }
                });
            });
            children.push(
                div().relative().w_full().child((row.build)(window, cx)).child(reader).into_any_element(),
            );
        }

        let pinned_overlay = band.and_then(|(top, _)| {
            let index = self
                .rows
                .iter()
                .zip(&frames)
                .rposition(|(row, (y, _))| row.pinned && *y < top)?;
            let height = frames[index].1;
            let next_y = self.rows[index + 1..]
                .iter()
                .zip(&frames[index + 1..])
                .find_map(|(row, (y, _))| row.pinned.then_some(*y));
            let overlay_top = next_y.map_or(top, |next_y| top.min(next_y - height));
            Some(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(overlay_top))
                    .child((self.rows[index].build)(window, cx))
                    .into_any_element(),
            )
        });

        let has_pinned = self.rows.iter().any(|row| row.pinned);
        let band_state = state;
        let probe = GiftScrollWindowProbe::new(
            ElementId::NamedChild(Arc::new(self.id), "window-probe".into()),
            move |band, _, cx| {
                let Some(band) = band else { return };
                band_state.update(cx, |current, cx| {
                    if Self::note_band(&mut current.band, &band, has_pinned) {
                        cx.notify();
                    }
                });
            },
        );

        div()
            .relative()
            .w_full()
            .children(children)
            .children(pinned_overlay)
            .child(div().absolute().inset_0().child(probe))
    }
}
