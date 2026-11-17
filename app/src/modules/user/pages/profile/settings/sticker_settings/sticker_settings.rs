use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AsyncApp, BoxShadow, Context, Entity, FocusHandle, KeyDownEvent, MouseButton, SharedString, Subscription, Task,
    WeakEntity, Window, black, deferred, point,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::grouped_btns::{GroupedBtn, GroupedBtns, GroupedCardMetrics};
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::TransitionModifiers;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::widgets::wrappers::list_shell_ui::{ListShellScrollUnderChromeModifiers, SettingsWrapperUi};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::input_picker::chat_input_picker_kit::ChatInputStickerAsset;
use crate::modules::stickers::components::sticker_pack_sheet::sticker_pack_sheet_store::StickerPackSheetStore;
use crate::modules::stickers::engine::rise_sticker_models::RiseStickerResourceID;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{PackKind, StickerItem, StickerPack};
use crate::modules::stickers::stores::stickers::stickers_interactions::stickers_interactions::StickersInteractionsStore;
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;

const PAGE_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const COMMITTED_ORDER_TIMEOUT: Duration = Duration::from_secs(3);
const ROW_HEIGHT: f32 = 60.0;
const ROW_STRIDE: f32 = ROW_HEIGHT + 0.5;
const GRIP_MINIMUM_DISTANCE: f32 = 2.0;
const GRIP_WIDTH: f32 = 44.0;

#[derive(Default)]
struct VisibleRowIds {
    ids: HashSet<String>,
}

/// page state and its task, which ends with the state when the page leaves
struct StickerSettingsState {
    visible_rows: Rc<RefCell<VisibleRowIds>>,
    dragging_id: Option<String>,
    drag_offset_y: f32,
    working_order: Vec<String>,
    accumulated_shift: f32,
    committed_order: Vec<String>,
    settling: HashMap<String, Animated<f32>>,
    lift: Option<(String, Animated<f32>)>,
    row_focus: HashMap<String, FocusHandle>,
    _subscriptions: Vec<Subscription>,
    _page_retry: Task<()>,
}

impl StickerSettingsState {
    fn new(kind: PackKind, cx: &mut Context<Self>) -> Self {
        let services = StickersServicesStore::shared(cx);
        let page_retry = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(PAGE_RETRY_INTERVAL).await;
                let retried = this.update(cx, |state, cx| {
                    let visible_rows = state.visible_rows.clone();
                    StickerSettings::retry_visible_end(kind, &visible_rows, cx);
                });
                if retried.is_err() {
                    return;
                }
            }
        });
        Self {
            visible_rows: Rc::new(RefCell::new(VisibleRowIds::default())),
            dragging_id: None,
            drag_offset_y: 0.0,
            working_order: Vec::new(),
            accumulated_shift: 0.0,
            committed_order: Vec::new(),
            settling: HashMap::new(),
            lift: None,
            row_focus: HashMap::new(),
            _subscriptions: vec![cx.observe(&services, |_, _, cx| cx.notify())],
            _page_retry: page_retry,
        }
    }

    /// own saved packs first in the users order, then default packs in admin order that cant be moved
    fn ordered_packs(&self, packs: Vec<StickerPack>) -> Vec<StickerPack> {
        let (own, defaults): (Vec<StickerPack>, Vec<StickerPack>) = packs.into_iter().partition(|pack| !pack.is_default);
        let local = if self.dragging_id.is_some() { &self.working_order } else { &self.committed_order };
        if local.is_empty() {
            return own.into_iter().chain(defaults).collect();
        }
        let mut by_id: HashMap<&str, &StickerPack> = HashMap::with_capacity(own.len());
        for pack in &own {
            by_id.entry(pack.id.as_str()).or_insert(pack);
        }
        let held: HashSet<&str> = local.iter().map(String::as_str).collect();
        let mut ordered: Vec<StickerPack> =
            local.iter().filter_map(|id| by_id.get(id.as_str()).map(|pack| (*pack).clone())).collect();
        ordered.extend(own.iter().filter(|pack| !held.contains(pack.id.as_str())).cloned());
        ordered.extend(defaults);
        ordered
    }

    fn settle(&mut self, id: &str, shift: f32, response: f64, damping_fraction: f64, cx: &App) {
        let now = RiseClock::monotonic();
        let settling = self.settling.entry(id.to_owned()).or_insert_with(|| Animated::new(0.0));
        let current = settling.value(now);
        settling.jump(current + shift);
        settling.set_in(0.0, spring(response, damping_fraction), now, cx);
    }

    fn settling_offset(&self, id: &str, window: &Window) -> f32 {
        self.settling.get(id).map_or(0.0, |settling| settling.frame_value(RiseClock::monotonic(), window))
    }

    fn lift_progress(&self, id: &str, window: &Window) -> f32 {
        match &self.lift {
            Some((lifted_id, lift)) if lifted_id == id => lift.frame_value(RiseClock::monotonic(), window),
            _ => 0.0,
        }
    }

    fn row_focus(&mut self, id: &str, cx: &mut Context<Self>) -> FocusHandle {
        self.row_focus.entry(id.to_owned()).or_insert_with(|| cx.focus_handle()).clone()
    }

    fn update_drag(&mut self, pack_id: &str, translation_y: f32, own_ids: Vec<String>, cx: &mut Context<Self>) {
        if self.dragging_id.as_deref() != Some(pack_id) {
            self.dragging_id = Some(pack_id.to_owned());
            self.working_order = own_ids;
            self.accumulated_shift = 0.0;
            self.lift = Some((pack_id.to_owned(), Animated::new(1.0)));
            let now = RiseClock::monotonic();
            self.settling.retain(|_, settling| settling.is_running(now));
            let _ = selection_changed();
        }
        self.drag_offset_y = translation_y - self.accumulated_shift;
        let Some(index) = self.working_order.iter().position(|id| id == pack_id) else {
            cx.notify();
            return;
        };
        if self.drag_offset_y > ROW_STRIDE * 0.6 && index + 1 < self.working_order.len() {
            let neighbour = self.working_order[index + 1].clone();
            self.working_order.swap(index, index + 1);
            self.settle(pack_id, -ROW_STRIDE, 0.28, 0.86, cx);
            self.settle(&neighbour, ROW_STRIDE, 0.28, 0.86, cx);
            self.accumulated_shift += ROW_STRIDE;
            self.drag_offset_y = translation_y - self.accumulated_shift;
        } else if self.drag_offset_y < -ROW_STRIDE * 0.6 && index > 0 {
            let neighbour = self.working_order[index - 1].clone();
            self.working_order.swap(index, index - 1);
            self.settle(pack_id, ROW_STRIDE, 0.28, 0.86, cx);
            self.settle(&neighbour, -ROW_STRIDE, 0.28, 0.86, cx);
            self.accumulated_shift -= ROW_STRIDE;
            self.drag_offset_y = translation_y - self.accumulated_shift;
        }
        cx.notify();
    }

    fn end_drag(&mut self, store_own_ids: &[String], cx: &mut Context<Self>) -> Option<Vec<String>> {
        let order = std::mem::take(&mut self.working_order);
        let before = &store_own_ids[..store_own_ids.len().min(order.len())];
        let changed = !order.is_empty() && before != order.as_slice();
        self.committed_order = if changed { order.clone() } else { Vec::new() };
        if let Some(dragging_id) = self.dragging_id.take() {
            let drag_offset_y = self.drag_offset_y;
            self.settle(&dragging_id, drag_offset_y, 0.3, 0.86, cx);
        }
        if let Some((_, lift)) = self.lift.as_mut() {
            lift.set_in(0.0, spring(0.3, 0.86), RiseClock::monotonic(), cx);
        }
        self.drag_offset_y = 0.0;
        self.accumulated_shift = 0.0;
        cx.notify();
        if !changed {
            return None;
        }
        let _ = selection_changed();
        let pending = order.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(COMMITTED_ORDER_TIMEOUT).await;
            let _ = this.update(cx, |state, cx| {
                if state.committed_order == pending {
                    state.release_committed_order(cx);
                }
            });
        })
        .detach();
        Some(order)
    }

    /// store holds the dropped order now or rolled it back after a refusal,
    /// either way it takes over quietly with no settle animation
    fn release_committed_order(&mut self, cx: &mut Context<Self>) {
        if self.committed_order.is_empty() || self.dragging_id.is_some() {
            return;
        }
        self.committed_order.clear();
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct StickerSettings {
    kind: PackKind,
}

impl StickerSettings {
    pub fn new(kind: PackKind) -> Self {
        Self { kind }
    }

    fn user_id(cx: &mut App) -> String {
        let auth = AuthServicesStore::shared(cx);
        auth.read(cx).effective_user_id_for_wire().unwrap_or_default()
    }

    fn saved_packs(user_id: &str, kind: PackKind, cx: &mut App) -> Vec<StickerPack> {
        let services = StickersServicesStore::shared(cx);
        services.read(cx).saved_pack_summaries(user_id, kind)
    }

    fn store_own_ids(user_id: &str, kind: PackKind, cx: &mut App) -> Vec<String> {
        Self::saved_packs(user_id, kind, cx).into_iter().filter(|pack| !pack.is_default).map(|pack| pack.id).collect()
    }

    fn displayed_own_ids(kind: PackKind, state: &Entity<StickerSettingsState>, cx: &mut App) -> Vec<String> {
        let user_id = Self::user_id(cx);
        let packs = Self::saved_packs(&user_id, kind, cx);
        state.read(cx).ordered_packs(packs).into_iter().filter(|pack| !pack.is_default).map(|pack| pack.id).collect()
    }

    fn retry_visible_end(kind: PackKind, visible_rows: &Rc<RefCell<VisibleRowIds>>, cx: &mut App) {
        let user_id = Self::user_id(cx);
        let last_id = Self::saved_packs(&user_id, kind, cx).last().map(|pack| pack.id.clone());
        if last_id.is_some_and(|last_id| RefCell::borrow(visible_rows).ids.contains(&last_id)) {
            Self::load_next_saved_page(kind, cx);
        }
    }

    fn my_packs_row(kind: PackKind, nav_store: Option<Entity<NavigationStore>>, theme: &AppTheme) -> GroupedBtn {
        GroupedBtn::new(
            format!("my-{}-packs", kind.wire()),
            tr(if kind == PackKind::Emoji { "emoji_my_packs" } else { "stickers_my_packs" }),
        )
        .icon(if kind == PackKind::Emoji { "face.smiling.fill" } else { "square.grid.2x2.fill" })
        .icon_background(theme.primary._100)
        .guide_anchor((kind == PackKind::Sticker).then_some(RiseGuideAnchor::StickersMyPacks))
        .callback(move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| {
                    nav_store.push(RootRoute::MyStickerPacks { kind }, NavigationStoreSettlingPush::Drop, cx);
                });
            }
        })
    }

    fn saved_pack_rows(
        kind: PackKind,
        saved_packs: Vec<StickerPack>,
        state: &Entity<StickerSettingsState>,
        theme: &AppTheme,
        window: &Window,
        cx: &mut App,
    ) -> Vec<GroupedBtn> {
        if saved_packs.is_empty() {
            let empty_text = tr(if kind == PackKind::Emoji { "emoji_pack_saved_empty" } else { "sticker_pack_saved_empty" });
            return vec![
                GroupedBtn::new(format!("saved-{}-empty", kind.wire()), empty_text.clone())
                    .show_chevron(false)
                    .content_slot(
                        hstack(GroupedCardMetrics::ICON_SPACING)
                            .px(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
                            .py(px(GroupedCardMetrics::ROW_VERTICAL_PADDING))
                            .child(
                                div()
                                    .flex_none()
                                    .size(px(GroupedCardMetrics::ICON_SIDE))
                                    .rounded(px(11.0))
                                    .bg(theme.bg._300)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .children(IconUi::sized(
                                        if kind == PackKind::Emoji { "face.smiling" } else { "square.grid.2x2" },
                                        px(17.0),
                                        theme.text.secondary,
                                        false,
                                    )),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    MainText::new(empty_text).px(15.0).tac(TextAlignT::Left).color(theme.text.secondary),
                                ),
                            ),
                    ),
            ];
        }

        let last_id = saved_packs.last().map(|pack| pack.id.clone());
        let packs = state.read(cx).ordered_packs(saved_packs);
        let own_ids: Vec<String> = packs.iter().filter(|pack| !pack.is_default).map(|pack| pack.id.clone()).collect();
        let shown: HashSet<String> = packs.iter().map(|pack| pack.id.clone()).collect();
        state.update(cx, |state, _| state.row_focus.retain(|id, _| shown.contains(id)));
        packs
            .into_iter()
            .map(|pack| {
                let id = format!("saved-pack-{}", pack.id);
                let title = pack.title.clone();
                let row = Self::saved_pack_row(kind, pack, &own_ids, last_id.as_deref(), state, theme, window, cx);
                GroupedBtn::new(id, title).content_slot(row)
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn saved_pack_row(
        kind: PackKind,
        pack: StickerPack,
        own_ids: &[String],
        last_id: Option<&str>,
        state: &Entity<StickerSettingsState>,
        theme: &AppTheme,
        window: &Window,
        cx: &mut App,
    ) -> AnyElement {
        let (is_dragging, drag_offset_y, settling, lift, visible_rows) = {
            let screen = state.read(cx);
            (
                screen.dragging_id.as_deref() == Some(pack.id.as_str()),
                screen.drag_offset_y,
                screen.settling_offset(&pack.id, window),
                screen.lift_progress(&pack.id, window),
                screen.visible_rows.clone(),
            )
        };
        let focus = state.update(cx, |state, cx| state.row_focus(&pack.id, cx));
        let item = StickersServicesStore::shared(cx).read(cx).pack_stickers(&pack.id).first().cloned();

        let is_last = last_id == Some(pack.id.as_str());
        let appeared_id = pack.id.clone();
        let appeared_rows = visible_rows.clone();
        let gone_id = pack.id.clone();
        let gone_rows = visible_rows;
        let icon = StickerPackSettingsIcon::new(pack.clone(), kind, item)
            .on_appear(SharedString::from(format!("saved-pack-appear-{}", pack.id)), move |_, cx| {
                appeared_rows.borrow_mut().ids.insert(appeared_id);
                if is_last {
                    Self::load_next_saved_page(kind, cx);
                }
            })
            .and_on_disappear(move |_, _| {
                gone_rows.borrow_mut().ids.remove(&gone_id);
            });

        let texts = vstack(2.0)
            .align(HorizontalAlignment::Leading)
            .flex_1()
            .min_w(px(0.0))
            .child(MainText::new(pack.title.clone()).px(15.0).color(theme.text.primary).number_of_lines(1))
            .child(
                MainText::new(tr(if kind == PackKind::Emoji { "emoji_pack_type" } else { "sticker_pack_type" }))
                    .px(12.0)
                    .color(theme.text.secondary)
                    .number_of_lines(1),
            );

        let trailing = if pack.is_default {
            div()
                .flex_none()
                .children(IconUi::sized(rise_ui::IconUi::trailing_chevron(), px(12.0), theme.text.secondary.opacity(0.75), false))
                .into_any_element()
        } else {
            Self::drag_handle(kind, &pack.id, state, theme).into_any_element()
        };

        let tap_state = state.clone();
        let tap_pack_id = pack.id.clone();
        let pack_kind = pack.pack_kind;
        let is_default = pack.is_default;
        let key_pack_id = pack.id.clone();
        let key_own_ids = own_ids.to_vec();
        let row = hstack(GroupedCardMetrics::ICON_SPACING)
            .id(SharedString::from(format!("saved-pack-row-{}", pack.id)))
            .w_full()
            .h(px(ROW_HEIGHT))
            .items_center()
            .pl(px(GroupedCardMetrics::ROW_HORIZONTAL_PADDING))
            .pr(px(if is_default { GroupedCardMetrics::ROW_HORIZONTAL_PADDING } else { 4.0 }))
            .bg(theme.bg._200)
            .track_focus(&focus)
            .cursor_pointer()
            .when(lift > 0.0, |row| {
                row.shadow(vec![BoxShadow {
                    color: black().opacity(0.18 * lift),
                    offset: point(px(0.0), px(3.0)),
                    blur_radius: px(10.0),
                    spread_radius: px(0.0),
                    inset: false,
                }])
            })
            .on_click(move |_, _, cx| {
                if tap_state.read(cx).dragging_id.is_some() {
                    return;
                }
                StickerPackSheetStore::shared(cx).update(cx, |store, cx| store.present(&tap_pack_id, pack_kind, cx));
            })
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if is_default || event.keystroke.modifiers.modified() {
                    return;
                }
                let up = match event.keystroke.key.as_str() {
                    "up" => true,
                    "down" => false,
                    _ => return,
                };
                cx.stop_propagation();
                Self::move_by_accessibility(kind, &key_pack_id, &key_own_ids, up, cx);
            })
            .child(icon)
            .child(texts)
            .child(trailing)
            .scale_effect(1.0 + 0.02 * lift);

        let offset = settling + if is_dragging { drag_offset_y } else { 0.0 };
        let placed = div().w_full().relative().top(px(offset)).child(row);
        if is_dragging || lift > 0.0 {
            deferred(placed).with_priority(1).into_any_element()
        } else {
            placed.into_any_element()
        }
    }

    fn drag_handle(
        kind: PackKind,
        pack_id: &str,
        state: &Entity<StickerSettingsState>,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let grip = div()
            .id(SharedString::from(format!("saved-pack-grip-{pack_id}")))
            .flex_none()
            .w(px(GRIP_WIDTH))
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .tooltip_label(tr("ctx_reorder"))
            .children(IconUi::sized("line.3.horizontal", px(17.0), theme.text.secondary, false));
        div()
            .flex_none()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(grip.gesture(
                SharedString::from(format!("saved-pack-grip-gesture-{pack_id}")),
                Self::drag_gesture(kind, state, pack_id),
            ))
    }

    fn drag_gesture(kind: PackKind, state: &Entity<StickerSettingsState>, pack_id: &str) -> DragGesture {
        let (changed_state, ended_state, changed_id) = (state.clone(), state.clone(), pack_id.to_owned());
        DragGesture::new(GRIP_MINIMUM_DISTANCE)
            .on_changed(move |value, _, cx| {
                let is_tracking = changed_state.read(cx).dragging_id.as_deref() == Some(changed_id.as_str());
                let own_ids = if is_tracking { Vec::new() } else { Self::displayed_own_ids(kind, &changed_state, cx) };
                let translation_y = f32::from(value.translation.height);
                changed_state.update(cx, |state, cx| state.update_drag(&changed_id, translation_y, own_ids, cx));
            })
            .on_ended(move |_, _, cx| {
                let user_id = Self::user_id(cx);
                let store_own_ids = Self::store_own_ids(&user_id, kind, cx);
                let order = ended_state.update(cx, |state, cx| state.end_drag(&store_own_ids, cx));
                if let Some(order) = order {
                    StickersInteractionsStore::shared(cx)
                        .update(cx, |interactions, cx| interactions.reorder_saved_packs(&user_id, kind, order, cx));
                }
            })
    }

    fn move_by_accessibility(kind: PackKind, pack_id: &str, own_ids: &[String], up: bool, cx: &mut App) {
        let Some(index) = own_ids.iter().position(|id| id == pack_id) else {
            return;
        };
        let target = if up { index.checked_sub(1) } else { Some(index + 1) };
        let Some(target) = target.filter(|target| *target < own_ids.len()) else {
            return;
        };
        let mut order = own_ids.to_vec();
        order.swap(index, target);
        let user_id = Self::user_id(cx);
        StickersInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.reorder_saved_packs(&user_id, kind, order, cx));
    }

    fn load_saved_packs(kind: PackKind, cx: &mut App) {
        let user_id = Self::user_id(cx);
        if user_id.is_empty() {
            return;
        }
        StickersInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.load_saved_packs(&user_id, kind, None, false, cx));
    }

    fn load_next_saved_page(kind: PackKind, cx: &mut App) {
        let user_id = Self::user_id(cx);
        if user_id.is_empty() {
            return;
        }
        let services = StickersServicesStore::shared(cx);
        if !services.read(cx).saved_packs_has_more(&user_id, kind) {
            return;
        }
        let Some(cursor) = services.read(cx).saved_packs_next_relative_id(&user_id, kind) else {
            return;
        };
        StickersInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.load_saved_packs(&user_id, kind, Some(cursor), false, cx));
    }
}

impl RenderOnce for StickerSettings {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let kind = self.kind;
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(SharedString::from(format!("sticker-settings-state-{}", kind.wire())), cx, move |_, cx| {
            StickerSettingsState::new(kind, cx)
        });
        let nav_store = env::<NavigationStoreKey>(cx);
        let user_id = Self::user_id(cx);
        let saved_packs = Self::saved_packs(&user_id, kind, cx);
        let store_own_ids: Vec<String> =
            saved_packs.iter().filter(|pack| !pack.is_default).map(|pack| pack.id.clone()).collect();
        let services = StickersServicesStore::shared(cx);
        let saved_empty = saved_packs.is_empty();
        let is_loading = services.read(cx).is_loading(RiseStickerResourceID::SavedPacks(kind));
        let has_more = services.read(cx).saved_packs_has_more(&user_id, kind);
        let is_dragging = state.read(cx).dragging_id.is_some();
        let rows = Self::saved_pack_rows(kind, saved_packs, &state, &theme, window, cx);

        on_change(
            SharedString::from(format!("sticker-settings-user-{}", kind.wire())),
            user_id,
            false,
            window,
            cx,
            move |_, _, _, cx| Self::load_saved_packs(kind, cx),
        );
        let release_state = state.clone();
        on_change(
            SharedString::from(format!("sticker-settings-own-ids-{}", kind.wire())),
            store_own_ids,
            false,
            window,
            cx,
            move |_, _, _, cx| release_state.update(cx, |state, cx| state.release_committed_order(cx)),
        );

        let tint = theme.primary._100;
        let content = vstack(20.0)
            .w_full()
            .pt(px(12.0))
            .child(GroupedBtns::new(
                SharedString::from(format!("sticker-settings-my-{}", kind.wire())),
                vec![Self::my_packs_row(kind, nav_store.clone(), &theme)],
            ))
            .child(
                GroupedBtns::new(SharedString::from(format!("sticker-settings-saved-{}", kind.wire())), rows).title(tr(
                    if kind == PackKind::Emoji { "emoji_saved_packs_title" } else { "stickers_saved_packs_title" },
                )),
            )
            .when(is_loading && saved_empty, |stack| stack.child(div().pt(px(8.0)).child(progress_view().tint(tint))))
            .when(has_more, |stack| stack.child(div().py(px(8.0)).child(progress_view().tint(tint))))
            .child(div().flex_none().h(px(80.0)));

        let title = tr(if kind == PackKind::Emoji { "settings_emoji_title" } else { "settings_stickers_title" });
        SettingsWrapperUi::new(title, move |_, cx| {
            if let Some(nav_store) = &nav_store {
                nav_store.update(cx, |nav_store, cx| nav_store.pop(cx));
            }
        })
        .content(
            scroll_view(SharedString::from(format!("sticker-settings-scroll-{}", kind.wire())))
                .scroll_disabled(is_dragging)
                .size_full()
                .bg(theme.bg._100)
                .list_shell_scroll_under_chrome(cx)
                .refreshable(move |_, cx| {
                    Self::load_saved_packs(kind, cx);
                    Task::ready(())
                })
                .child(content),
        )
        .on_appear(SharedString::from(format!("sticker-settings-appear-{}", kind.wire())), move |_, cx| {
            Self::load_saved_packs(kind, cx);
        })
    }
}

#[derive(IntoElement)]
pub struct StickerPackSettingsIcon {
    pack: StickerPack,
    fallback_kind: PackKind,
    item: Option<StickerItem>,
}

impl StickerPackSettingsIcon {
    pub fn new(pack: StickerPack, fallback_kind: PackKind, item: Option<StickerItem>) -> Self {
        Self { pack, fallback_kind, item }
    }
}

impl RenderOnce for StickerPackSettingsIcon {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let creator_item = if self.pack.is_creator { self.item.clone() } else { None };
        let cover = creator_item.or_else(|| self.pack.cover_item()).or(self.item);

        div()
            .flex_none()
            .size(px(40.0))
            .rounded(px(10.0))
            .bg(theme.bg._300)
            .flex()
            .items_center()
            .justify_center()
            .map(|icon| match cover {
                Some(cover) => icon.child(
                    div()
                        .size(px(38.0))
                        .rounded(px(9.0))
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ChatInputStickerAsset::new(cover, 34.0).is_playing(false)),
                ),
                None => icon.children(IconUi::sized(
                    if self.fallback_kind == PackKind::Emoji { "face.smiling.fill" } else { "square.grid.2x2.fill" },
                    px(18.0),
                    theme.text.secondary,
                    false,
                )),
            })
    }
}
