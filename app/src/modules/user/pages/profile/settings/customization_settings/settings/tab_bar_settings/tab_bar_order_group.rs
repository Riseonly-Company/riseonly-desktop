use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Context, ElementId, Entity, FocusHandle, Hsla, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use rise_canon::RiseTabBarButton;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::MainText;
use rise_ui::reorder_grip::ReorderGripHandle;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::widgets::navigations::main_tab_bar_buttons::RiseTabBarButtonExt;
use crate::core::widgets::navigations::main_tab_bar_settings::RiseTabBarSettings;

pub type TabBarOrderOnCommit = Rc<dyn Fn(Vec<RiseTabBarButton>, &mut App)>;

const ROW_HEIGHT: f32 = 56.0;
const ROW_STRIDE: f32 = ROW_HEIGHT + 0.5;
const COMMITTED_ORDER_TIMEOUT: Duration = Duration::from_secs(3);
const GRIP_WIDTH: f32 = 44.0;
const GLYPH_SIDE: f32 = 17.0;
const ICON_CORNER_RADIUS: f32 = 11.0;
const DROP_SPRING_RESPONSE: f64 = 0.3;
const DROP_SPRING_DAMPING: f64 = 0.86;

#[derive(IntoElement)]
pub struct TabBarOrderGroup {
    settings: RiseTabBarSettings,
    surface: Hsla,
    on_commit: TabBarOrderOnCommit,
}

impl TabBarOrderGroup {
    pub fn new(
        settings: RiseTabBarSettings,
        surface: Hsla,
        on_commit: impl Fn(Vec<RiseTabBarButton>, &mut App) + 'static,
    ) -> Self {
        Self {
            settings,
            surface,
            on_commit: Rc::new(on_commit),
        }
    }

    fn store_order(settings: &RiseTabBarSettings) -> Vec<RiseTabBarButton> {
        if settings.more_tab { settings.more_mode_order() } else { settings.button_order.to_vec() }
    }

    fn title(button: RiseTabBarButton) -> String {
        button.item().map(|item| tr(item.title_key)).unwrap_or_else(|| button.wire().to_owned())
    }
}

/// group state, drag_offset_motion is the drawn drag offset so a drop can spring back
struct TabBarOrderGroupState {
    dragging_id: Option<RiseTabBarButton>,
    drag_offset_y: f32,
    drag_offset_motion: Animated<f32>,
    offset_owner: Option<RiseTabBarButton>,
    row_focus: Vec<(RiseTabBarButton, FocusHandle)>,
    working_order: Vec<RiseTabBarButton>,
    accumulated_shift: f32,
    committed_order: Vec<RiseTabBarButton>,
    observed_store_order: Vec<RiseTabBarButton>,
    release_task: Option<Task<()>>,
}

impl TabBarOrderGroupState {
    fn new(store_order: Vec<RiseTabBarButton>) -> Self {
        Self {
            dragging_id: None,
            drag_offset_y: 0.0,
            drag_offset_motion: Animated::new(0.0),
            offset_owner: None,
            row_focus: Vec::new(),
            working_order: Vec::new(),
            accumulated_shift: 0.0,
            committed_order: Vec::new(),
            observed_store_order: store_order,
            release_task: None,
        }
    }

    fn row_focus(&mut self, button: RiseTabBarButton, cx: &mut Context<Self>) -> FocusHandle {
        if let Some((_, focus)) = self.row_focus.iter().find(|(candidate, _)| *candidate == button) {
            return focus.clone();
        }
        let focus = cx.focus_handle();
        self.row_focus.push((button, focus.clone()));
        focus
    }

    fn shown_order(&self, store_order: &[RiseTabBarButton]) -> Vec<RiseTabBarButton> {
        if self.dragging_id.is_some() {
            return self.working_order.clone();
        }
        if self.committed_order.is_empty() { store_order.to_vec() } else { self.committed_order.clone() }
    }

    /// reacts to a change of the store order
    fn observe(&mut self, store_order: &[RiseTabBarButton]) -> bool {
        if self.observed_store_order == store_order {
            return false;
        }
        self.observed_store_order = store_order.to_vec();
        self.release_committed_order()
    }

    fn release_committed_order(&mut self) -> bool {
        if self.committed_order.is_empty() || self.dragging_id.is_some() {
            return false;
        }
        self.committed_order.clear();
        true
    }

    fn update_drag(&mut self, button: RiseTabBarButton, translation_y: f32, order: &[RiseTabBarButton]) {
        if self.dragging_id != Some(button) {
            self.dragging_id = Some(button);
            self.working_order = order.to_vec();
            self.accumulated_shift = 0.0;
            self.offset_owner = Some(button);
            let _ = selection_changed();
        }
        self.drag_offset_y = translation_y - self.accumulated_shift;
        let Some(index) = self.working_order.iter().position(|candidate| *candidate == button) else {
            return;
        };
        if self.drag_offset_y > ROW_STRIDE * 0.6 && index + 1 < self.working_order.len() {
            self.working_order.swap(index, index + 1);
            self.accumulated_shift += ROW_STRIDE;
            self.drag_offset_y = translation_y - self.accumulated_shift;
        } else if self.drag_offset_y < -ROW_STRIDE * 0.6 && index > 0 {
            self.working_order.swap(index, index - 1);
            self.accumulated_shift -= ROW_STRIDE;
            self.drag_offset_y = translation_y - self.accumulated_shift;
        }
        self.drag_offset_motion.jump(self.drag_offset_y);
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) -> Option<Vec<RiseTabBarButton>> {
        let order = std::mem::take(&mut self.working_order);
        let changed = !order.is_empty() && order != self.observed_store_order;
        self.committed_order = if changed { order.clone() } else { Vec::new() };
        self.drag_offset_y = 0.0;
        self.drag_offset_motion.set_in(
            0.0,
            spring(DROP_SPRING_RESPONSE, DROP_SPRING_DAMPING),
            RiseClock::monotonic(),
            cx,
        );
        self.dragging_id = None;
        self.accumulated_shift = 0.0;
        changed.then_some(order)
    }
}

impl RenderOnce for TabBarOrderGroup {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let store_order = Self::store_order(&self.settings);
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

        let items: Vec<GroupedBtn> = order
            .iter()
            .map(|button| {
                let button = *button;
                let (offset, focus) = state.update(cx, |state, cx| {
                    let offset = if state.offset_owner == Some(button) {
                        state.drag_offset_motion.frame_value(RiseClock::monotonic(), window)
                    } else {
                        0.0
                    };
                    (offset, state.row_focus(button, cx))
                });
                GroupedBtn::new(format!("tab-bar-order-{}", button.wire()), Self::title(button)).content_slot(
                    self.row(button, order.clone(), offset, focus, &state, &theme),
                )
            })
            .collect();

        GroupedBtns::new("tab-bar-order-group", items)
            .title(tr("tab_bar_order_title").to_uppercase())
            .end_group_title(tr("tab_bar_order_hint"))
            .surface_color(self.surface)
    }
}

impl TabBarOrderGroup {
    fn row(
        &self,
        button: RiseTabBarButton,
        order: Arc<Vec<RiseTabBarButton>>,
        offset: f32,
        focus: FocusHandle,
        state: &Entity<TabBarOrderGroupState>,
        theme: &rise_theme::AppTheme,
    ) -> impl IntoElement {
        let on_commit = &self.on_commit;
        let surface = self.surface;
        let item = button.item();
        let symbol = item.map(|item| item.symbol).unwrap_or("circle");
        let change_state = state.clone();
        let end_state = state.clone();
        let end_commit = on_commit.clone();
        let key_commit = on_commit.clone();
        let key_order = order.clone();
        let grip = ReorderGripHandle::new(
            ElementId::Name(SharedString::from(format!("tab-bar-order-grip-{}", button.wire()))),
            Rc::new(move |translation: f32, _: &mut Window, cx: &mut App| {
                let order = order.clone();
                change_state.update(cx, |state, cx| {
                    state.update_drag(button, translation, &order);
                    cx.notify();
                });
            }),
            Rc::new(move |_: &mut Window, cx: &mut App| {
                Self::end_drag(&end_state, &end_commit, cx);
            }),
        );

        div()
            .id(ElementId::Name(SharedString::from(format!("tab-bar-order-row-{}", button.wire()))))
            .relative()
            .top(px(offset))
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(GroupedCardMetrics::ICON_SPACING))
            .pl(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .pr(px(4.0))
            .bg(surface)
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
                    .id(ElementId::Name(SharedString::from(format!("tab-bar-order-grip-area-{}", button.wire()))))
                    .tooltip_label(tr("ctx_reorder"))
                    .relative()
                    .flex_none()
                    .w(px(GRIP_WIDTH))
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized("line.3.horizontal", px(GLYPH_SIDE), theme.text.secondary, false))
                    .child(div().absolute().inset_0().child(grip)),
            )
    }

    fn end_drag(state: &Entity<TabBarOrderGroupState>, on_commit: &TabBarOrderOnCommit, cx: &mut App) {
        let committed = state.update(cx, |state, cx| {
            let committed = state.end_drag(cx);
            cx.notify();
            committed
        });
        let Some(order) = committed else {
            return;
        };
        let _ = selection_changed();
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
        button: RiseTabBarButton,
        order: &[RiseTabBarButton],
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
