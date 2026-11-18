use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, BoxShadow, Context, ElementId, Entity, FocusHandle, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, RenderOnce, SharedString, Styled, Task, Window, deferred, div, hsla, point,
    prelude::FluentBuilder, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::MainText;
use rise_ui::reorder_grip::ReorderMotion;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::widgets::navigations::main_tab_bar::MainTabBarMoreEntry;
use crate::core::widgets::navigations::main_tab_bar_buttons::{RailEntry, RiseTabBarButtonExt};
use crate::core::widgets::navigations::main_tab_bar_settings::RiseTabBarSettings;

pub type TabBarOrderOnCommit = Rc<dyn Fn(Vec<RailEntry>, &mut App)>;

const ROW_HEIGHT: f32 = 56.0;
const ROW_STRIDE: f32 = ROW_HEIGHT + 0.5;
const GRIP_DRAG_DISTANCE: f32 = 2.0;
const COMMITTED_ORDER_TIMEOUT: Duration = Duration::from_secs(3);
const GRIP_WIDTH: f32 = 44.0;
const GLYPH_SIDE: f32 = 17.0;
const ICON_CORNER_RADIUS: f32 = 11.0;

#[derive(IntoElement)]
pub struct TabBarOrderGroup {
    settings: RiseTabBarSettings,
    surface: Hsla,
    hint: Option<String>,
    on_commit: TabBarOrderOnCommit,
}

impl TabBarOrderGroup {
    pub fn new(
        settings: RiseTabBarSettings,
        surface: Hsla,
        on_commit: impl Fn(Vec<RailEntry>, &mut App) + 'static,
    ) -> Self {
        Self {
            settings,
            surface,
            hint: None,
            on_commit: Rc::new(on_commit),
        }
    }

    pub fn hint(mut self, hint: String) -> Self {
        self.hint = Some(hint);
        self
    }

    /// buttons the left bar shows, gated ones are hidden
    fn store_order(settings: &RiseTabBarSettings, cx: &mut App) -> Vec<RailEntry> {
        RailEntry::saved(&settings.button_order, cx)
    }

    fn title(entry: RailEntry) -> String {
        match entry {
            RailEntry::Button(button) => {
                button.item().map(|item| tr(item.title_key)).unwrap_or_else(|| button.wire().to_owned())
            }
            RailEntry::Settings => tr(MainTabBarMoreEntry::settings().title_key),
        }
    }

    fn symbol(entry: RailEntry) -> &'static str {
        match entry {
            RailEntry::Button(button) => button.item().map(|item| item.symbol).unwrap_or("circle"),
            RailEntry::Settings => "gearshape",
        }
    }

    fn key(entry: RailEntry) -> &'static str {
        match entry {
            RailEntry::Button(button) => button.wire(),
            RailEntry::Settings => "settings",
        }
    }
}

/// group state, the shared reorder motion draws the drag and the drop
struct TabBarOrderGroupState {
    motion: ReorderMotion<RailEntry>,
    row_focus: Vec<(RailEntry, FocusHandle)>,
    committed_order: Vec<RailEntry>,
    observed_store_order: Vec<RailEntry>,
    release_task: Option<Task<()>>,
}

impl TabBarOrderGroupState {
    fn new(store_order: Vec<RailEntry>) -> Self {
        Self {
            motion: ReorderMotion::new(),
            row_focus: Vec::new(),
            committed_order: Vec::new(),
            observed_store_order: store_order,
            release_task: None,
        }
    }

    fn row_focus(&mut self, button: RailEntry, cx: &mut Context<Self>) -> FocusHandle {
        if let Some((_, focus)) = self.row_focus.iter().find(|(candidate, _)| *candidate == button) {
            return focus.clone();
        }
        let focus = cx.focus_handle();
        self.row_focus.push((button, focus.clone()));
        focus
    }

    fn shown_order(&self, store_order: &[RailEntry]) -> Vec<RailEntry> {
        let base = if self.committed_order.is_empty() { store_order.to_vec() } else { self.committed_order.clone() };
        self.motion.shown_order(&base)
    }

    /// reacts to a change of the store order
    fn observe(&mut self, store_order: &[RailEntry]) -> bool {
        if self.observed_store_order == store_order {
            return false;
        }
        self.observed_store_order = store_order.to_vec();
        self.release_committed_order()
    }

    fn release_committed_order(&mut self) -> bool {
        if self.committed_order.is_empty() || self.motion.dragging().is_some() {
            return false;
        }
        self.committed_order.clear();
        true
    }

    fn update_drag(&mut self, entry: RailEntry, translation_y: f32, order: &[RailEntry], reduce_motion: bool) {
        self.motion.drag(&entry, translation_y, order, |_| ROW_STRIDE, RiseClock::monotonic(), reduce_motion);
    }

    fn end_drag(&mut self, reduce_motion: bool) -> Option<Vec<RailEntry>> {
        let base = if self.committed_order.is_empty() {
            self.observed_store_order.clone()
        } else {
            self.committed_order.clone()
        };
        let order = self.motion.end(RiseClock::monotonic(), reduce_motion)?;
        let changed = !order.is_empty() && order != base;
        if changed {
            self.committed_order = order.clone();
        }
        changed.then_some(order)
    }
}

impl RenderOnce for TabBarOrderGroup {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store_order = Self::store_order(&self.settings, cx);
        let initial = store_order.clone();
        let state = window.use_keyed_state(
            ElementId::Name(SharedString::from("tab-bar-order-group-state")),
            cx,
            move |_: &mut Window, _: &mut Context<TabBarOrderGroupState>| TabBarOrderGroupState::new(initial),
        );
        let order = state.update(cx, |state, cx| {
            if state.observe(&store_order) {
                cx.notify();
            }
            let order = state.shown_order(&store_order);
            state.row_focus.retain(|(button, _)| order.contains(button));
            order
        });
        let order = Arc::new(order);
        let count = order.len();

        let items: Vec<GroupedBtn> = order
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let entry = *entry;
                let corners = (index == 0, index + 1 == count);
                let (offset, raise, focus) = state.update(cx, |state, cx| {
                    let offset = state.motion.offset(&entry, window);
                    let raise = state.motion.lift(&entry, window);
                    (offset, raise, state.row_focus(entry, cx))
                });
                GroupedBtn::new(format!("tab-bar-order-{}", Self::key(entry)), Self::title(entry)).content_slot(
                    self.row(entry, order.clone(), (offset, raise), corners, focus, &state, &theme),
                )
            })
            .collect();

        GroupedBtns::new("tab-bar-order-group", items)
            .title(tr("tab_bar_order_title").to_uppercase())
            .end_group_title(self.hint.clone().unwrap_or_else(|| tr("tab_bar_order_hint")))
            .surface_color(self.surface)
    }
}

impl TabBarOrderGroup {
    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        button: RailEntry,
        order: Arc<Vec<RailEntry>>,
        (offset, raise): (f32, f32),
        (rounds_top, rounds_bottom): (bool, bool),
        focus: FocusHandle,
        state: &Entity<TabBarOrderGroupState>,
        theme: &rise_theme::AppTheme,
    ) -> AnyElement {
        let on_commit = &self.on_commit;
        let surface = self.surface;
        let symbol = Self::symbol(button);
        let change_state = state.clone();
        let end_state = state.clone();
        let end_commit = on_commit.clone();
        let key_commit = on_commit.clone();
        let key_order = order.clone();
        let grip = DragGesture::new(GRIP_DRAG_DISTANCE)
            .on_changed(move |value: &DragGestureValue, _: &mut Window, cx: &mut App| {
                let translation = f32::from(value.translation.height);
                let reduce_motion = cx.reduce_motion();
                change_state.update(cx, |state, cx| {
                    state.update_drag(button, translation, &order, reduce_motion);
                    cx.notify();
                });
            })
            .on_ended(move |_: &DragGestureValue, _: &mut Window, cx: &mut App| {
                Self::end_drag(&end_state, &end_commit, cx);
            });

        let row = div()
            .id(ElementId::Name(SharedString::from(format!("tab-bar-order-row-{}", Self::key(button)))))
            .relative()
            .top(px(offset))
            .when(raise > 0.0, |row| {
                row.rounded(px(GroupedCardMetrics::CORNER_RADIUS)).shadow(vec![BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.22 * raise),
                    offset: point(px(0.0), px(6.0 * raise)),
                    blur_radius: px(18.0 * raise),
                    spread_radius: px(0.0),
                    inset: false,
                }])
            })
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(GroupedCardMetrics::ICON_SPACING))
            .pl(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .pr(px(4.0))
            .bg(surface)
            .when(rounds_top, |row| row.rounded_t(px(GroupedCardMetrics::CORNER_RADIUS)))
            .when(rounds_bottom, |row| row.rounded_b(px(GroupedCardMetrics::CORNER_RADIUS)))
            .track_focus(&focus)
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                let up = match event.keystroke.key.as_str() {
                    "up" => true,
                    "down" => false,
                    _ => return,
                };
                cx.stop_propagation();
                Self::move_by_accessibility(button, &key_order, up, &key_commit, cx);
            })
            .child(
                div()
                    .flex_none()
                    .size(px(GroupedCardMetrics::ICON_SIDE))
                    .rounded(px(ICON_CORNER_RADIUS))
                    .bg(theme.primary._100)
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(symbol, px(GLYPH_SIDE), theme.content_on(theme.primary._100), false)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(MainText::new(Self::title(button)).px(15.0).color(theme.text.primary).number_of_lines(1)),
            )
            .child(
                div()
                    .id(ElementId::Name(SharedString::from(format!("tab-bar-order-grip-area-{}", Self::key(button)))))
                    .tooltip_label(tr("ctx_reorder"))
                    .relative()
                    .flex_none()
                    .w(px(GRIP_WIDTH))
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("line.3.horizontal", px(GLYPH_SIDE), theme.text.secondary, false))
                    .gesture(
                        ElementId::Name(SharedString::from(format!("tab-bar-order-grip-{}", Self::key(button)))),
                        grip,
                    ),
            );
        if raise > 0.0 { deferred(row).with_priority(1).into_any_element() } else { row.into_any_element() }
    }

    fn end_drag(state: &Entity<TabBarOrderGroupState>, on_commit: &TabBarOrderOnCommit, cx: &mut App) {
        let reduce_motion = cx.reduce_motion();
        let committed = state.update(cx, |state, cx| {
            let committed = state.end_drag(reduce_motion);
            cx.notify();
            committed
        });
        let Some(order) = committed else {
            return;
        };
        on_commit(order.clone(), cx);
        let weak = state.downgrade();
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(COMMITTED_ORDER_TIMEOUT).await;
            let _ = weak.update(cx, |state, cx| {
                if state.committed_order == order && state.release_committed_order() {
                    cx.notify();
                }
            });
        });
        state.update(cx, |state, _| state.release_task = Some(task));
    }

    fn move_by_accessibility(
        button: RailEntry,
        order: &[RailEntry],
        up: bool,
        on_commit: &TabBarOrderOnCommit,
        cx: &mut App,
    ) {
        let Some(index) = order.iter().position(|candidate| *candidate == button) else {
            return;
        };
        let Some(target) = (if up { index.checked_sub(1) } else { Some(index + 1) }) else {
            return;
        };
        if target >= order.len() {
            return;
        }
        let mut next = order.to_vec();
        next.swap(index, target);
        on_commit(next, cx);
    }
}
