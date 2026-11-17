use std::rc::Rc;

use gpui::{
    AnyElement, AnyView, App, AppContext as _, AsyncApp, Context, ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, Render, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    StyleRefinement, Subscription, Window, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactFeedbackGenerator, ImpactStyle};
use rise_theme::ThemeStore;
use rise_ui::animated_tabs_ui::animated_tabs_types::{AnimatedTabsHaptics, AnimatedTabsMetrics};
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::minimum_scale_factor::MinimumScaleText;
use rise_ui::swiftui::motion::ease_out;

use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_shared_ui::ChatProfileEmptyState;
use crate::modules::gifts::components::gift_card_view::{GiftGridCell, GiftsGridMetrics};
use crate::modules::gifts::components::gift_shop_shelf_grid::{
    GiftScrollWindowProbe, GiftShopShelfFixedCells, GiftShopShelfGrid,
};
use crate::modules::gifts::engine::rise_gifts_engine_models::{
    RiseGiftInstanceDTO, RiseGiftInstanceExt, RiseGiftItemKind,
};
use crate::modules::gifts::shared::gift_market_kinds::GiftMarketKinds;
use crate::modules::gifts::shared::gift_people::GiftPeople;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::{
    GiftRecipient, GiftsInstanceList, GiftsInteractionsStore,
};
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::{GiftsServicesStore, GiftsTabState};
use crate::modules::post::components::post::post_skeleton::PostGridSkeleton;
use crate::modules::post::components::post_collection_ui::PostPaginationTriggerModifiers;
use crate::modules::user::stores::user::user_actions::user_wire_types::ProfileViewerPrivacy;

/// items tab of a profile with gifts and frames and banners and effects
/// shared by profile page and chat profile, kind tabs are tapped and never swiped
/// owner sees all with hidden ones dimmed and can pin or hide, others see what privacy allows
#[derive(IntoElement)]
pub struct UserGiftsTab {
    pub owner_user_id: String,
}

#[derive(Default)]
pub struct UserGiftsTabState {
    owner: Option<GiftRecipient>,
    kind: Option<RiseGiftItemKind>,
    menu_instance_id: Option<String>,
    width: Option<f32>,
}

impl UserGiftsTab {
    pub fn new(owner_user_id: impl Into<String>) -> Self {
        Self {
            owner_user_id: owner_user_id.into(),
        }
    }

    fn is_owner(&self, cx: &mut App) -> bool {
        AuthServicesStore::shared(cx).read(cx).is_current_user(&self.owner_user_id)
    }

    fn skeleton(width: Option<f32>) -> impl IntoElement {
        let columns = width.map_or(GiftsGridMetrics::COLUMNS, GiftsGridMetrics::columns);
        let skeleton = PostGridSkeleton::default()
            .columns(columns)
            .spacing(GiftsGridMetrics::SPACING)
            .rows(2);
        let skeleton = match width {
            Some(width) => skeleton.cell_height(GiftsGridMetrics::cell_side(width)),
            None => skeleton,
        };
        div().px(px(GiftsGridMetrics::HORIZONTAL_PADDING)).pt(px(8.0)).child(skeleton)
    }

    /// a first read that failed offline leaves the tab loaded and empty, which would say the
    /// owner has no items and thats false
    /// so it says the connection is lost and offers a retry
    fn failure(&self, cx: &mut App) -> AnyElement {
        let accent = ThemeStore::shared(cx).read(cx).current_theme().primary._100;
        let owner_id = self.owner_user_id.clone();
        div()
            .flex()
            .flex_col()
            .items_center()
            .child(ChatProfileEmptyState::new("wifi.slash", tr("error_connection_lost")))
            .child(
                plain(self.child_id("retry"))
                    .on_press(move |_, cx| {
                        let owner_id = owner_id.clone();
                        cx.spawn(async move |cx: &mut AsyncApp| GiftsActionsStore::refresh_tab(&owner_id, cx).await)
                            .detach();
                    })
                    .child(
                        MainText::new(tr("common_retry"))
                            .px(15.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(accent),
                    ),
            )
            .into_any_element()
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::Name(format!("user-gifts-tab-{name}-{}", self.owner_user_id).into())
    }

    /// the tab renders every scroll frame of the profile page since the page owns the scroll
    /// it reads the kind list by reference and clones only a cell whose instance changed
    #[allow(clippy::too_many_arguments)]
    fn grid(
        &self,
        item_count: i64,
        has_more: bool,
        is_loading_more: bool,
        kind: Option<RiseGiftItemKind>,
        owner: Option<GiftRecipient>,
        is_owner: bool,
        width: f32,
        state: Entity<UserGiftsTabState>,
        items: Rc<Vec<RiseGiftInstanceDTO>>,
        snapshot: Entity<UserGiftsTabSnapshot>,
    ) -> AnyElement {
        let side = GiftsGridMetrics::cell_side(width);
        let columns = GiftsGridMetrics::columns(width);
        let inner_width = width - GiftsGridMetrics::HORIZONTAL_PADDING * 2.0;
        let column_width = (inner_width - GiftsGridMetrics::SPACING * (columns - 1) as f32) / columns.max(1) as f32;
        let tail_probe = self.tail_retry_probe(kind, item_count, columns, side, snapshot);
        let shared = UserGiftsCellShared {
            owner_user_id: self.owner_user_id.clone(),
            kind,
            owner,
            is_owner,
            side,
            item_count,
            has_more,
            is_loading_more,
        };
        let indices: Vec<i64> = (0..item_count).collect();
        let template = move |_: &mut Window, _: &mut App| div().w_full().h(px(side));
        let cell = move |index: &i64, window: &mut Window, cx: &mut App| -> AnyElement {
            let index = *index;
            let instance_id = items.get(index as usize).map(|instance| instance.id.clone());
            let Some(instance_id) = instance_id else {
                return div().into_any_element();
            };
            let is_menu_presented = state.read(cx).menu_instance_id.as_deref() == Some(instance_id.as_str());
            let host_id = ElementId::Name(format!("user-gifts-cell-view-{instance_id}").into());
            let host = window.with_global_id(host_id, |global_id, window| {
                window.with_element_state(global_id, |state: Option<Entity<UserGiftsCellView>>, _| {
                    let host = state.unwrap_or_else(|| cx.new(|_| UserGiftsCellView::new()));
                    (host.clone(), host)
                })
            });
            let fresh = {
                items.get(index as usize).and_then(|instance| {
                    let current = host
                        .read(cx)
                        .props
                        .as_ref()
                        .is_some_and(|props| props.matches(&shared, index, instance, is_menu_presented));
                    (!current).then(|| UserGiftsCellProps {
                        shared: shared.clone(),
                        index,
                        instance: instance.clone(),
                        is_menu_presented,
                    })
                })
            };
            let cell_state = state.clone();
            let stale = fresh.is_some();
            host.update(cx, |host, _| host.set_props(fresh, cell_state));
            if stale {
                AnyView::from(host).into_any_element()
            } else {
                host.cached(StyleRefinement::default().w(px(side)).h(px(side))).replay_when_moved().into_any_element()
            }
        };
        div()
            .w(px(width))
            .pt(px(8.0))
            .pb(px(24.0))
            .child(
                div()
                    .relative()
                    .h(px(GiftsGridMetrics::grid_height(item_count, width)))
                    .px(px(GiftsGridMetrics::HORIZONTAL_PADDING))
                    .child(
                        GiftShopShelfGrid::new(self.child_id("grid"), indices, columns, template, cell)
                            .spacing(GiftsGridMetrics::SPACING)
                            .fixed_cells(GiftShopShelfFixedCells {
                                column_width,
                                row_height: side,
                            }),
                    )
                    .child(div().absolute().inset_0().child(tail_probe)),
            )
            .into_any_element()
    }

    const TAIL_RETRY_THRESHOLD: i64 = 6;

    fn tail_retry_probe(
        &self,
        kind: Option<RiseGiftItemKind>,
        item_count: i64,
        columns: i64,
        side: f32,
        snapshot: Entity<UserGiftsTabSnapshot>,
    ) -> GiftScrollWindowProbe {
        let owner_id = self.owner_user_id.clone();
        let tail_index = (item_count - Self::TAIL_RETRY_THRESHOLD).max(0);
        let tail_top = (tail_index / columns.max(1)) as f32 * (side + GiftsGridMetrics::SPACING);
        GiftScrollWindowProbe::new(self.child_id("tail-retry"), move |band, height, cx| {
            let Some(band) = band else { return };
            if item_count <= 0 || *band.start() >= height || *band.end() < tail_top {
                return;
            }
            let can_load = {
                let fetched = snapshot.read(cx).fetched;
                fetched.loaded && fetched.has_more && !fetched.loading && fetched.has_cursor
            };
            if !can_load {
                return;
            }
            let owner_id = owner_id.clone();
            GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_tab(owner_id, kind, cx));
        })
        .on_catch_up(|_, _, _| true)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell(props: UserGiftsCellProps, state: Entity<UserGiftsTabState>) -> AnyElement {
        let UserGiftsCellProps {
            shared:
                UserGiftsCellShared {
                    owner_user_id,
                    kind,
                    owner,
                    is_owner,
                    side,
                    item_count,
                    has_more,
                    is_loading_more,
                },
            index,
            instance,
            is_menu_presented: _,
        } = props;
        let opened_list = Self::opened_list(&owner_user_id, kind);
        let id = ElementId::Name(format!("user-gifts-cell-{}", instance.id).into());
        let menu_id = ElementId::Name(format!("user-gifts-cell-menu-{}", instance.id).into());
        let trigger_id = ElementId::Name(format!("user-gifts-cell-page-{}", instance.id).into());
        let tapped = instance.clone();
        let tap_state = state.clone();
        let tap_list = opened_list.clone();
        let button = plain(id)
            .on_press(move |_, cx| {
                if tap_state.read(cx).menu_instance_id.is_some() {
                    return;
                }
                let instance = tapped.clone();
                let list = tap_list.clone();
                GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.open_instance(instance, Some(list), cx));
            })
            .child(GiftGridCell::new(instance.clone(), side, is_owner).person(owner.clone()));
        let menu_get_id = instance.id.clone();
        let menu_set_id = instance.id.clone();
        let is_presented = Binding::entity(
            state.clone(),
            move |state: &UserGiftsTabState| state.menu_instance_id.as_deref() == Some(menu_get_id.as_str()),
            move |state: &mut UserGiftsTabState, is_presented: bool, cx: &mut Context<UserGiftsTabState>| {
                if !is_presented && state.menu_instance_id.as_deref() == Some(menu_set_id.as_str()) {
                    state.menu_instance_id = None;
                    cx.notify();
                }
            },
        );
        let listed = instance.clone();
        let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| Self::owner_actions(&listed));
        let acted = instance.clone();
        let action_list = opened_list;
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::perform_owner_action(&id, acted.clone(), action_list.clone(), cx);
        });
        let source_instance = instance.clone();
        let source_owner = owner;
        let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| {
            GiftGridCell::new(source_instance.clone(), side, is_owner)
                .person(source_owner.clone())
                .into_any_element()
        });
        let held_id = instance.id.clone();
        div()
            .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, _, cx| {
                if !is_owner {
                    return;
                }
                cx.stop_propagation();
                ImpactFeedbackGenerator::new(ImpactStyle::Medium).impact_occurred();
                let held = held_id.clone();
                state.update(cx, |state, cx| {
                    state.menu_instance_id = Some(held);
                    cx.notify();
                });
            })
            .child(button)
            .tap_context_menu(
                menu_id,
                is_presented,
                HoldContextMenuAlignment::Leading,
                false,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            )
            .post_pagination_trigger(trigger_id, index, item_count, has_more, is_loading_more, move |cx: &mut App| {
                let owner_id = owner_user_id.clone();
                GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_tab(owner_id, kind, cx));
            })
            .into_any_element()
    }

    fn owner_actions(instance: &RiseGiftInstanceDTO) -> Vec<HoldContextMenuAction> {
        let mut actions = Vec::new();
        if instance.is_cosmetic() {
            actions.push(
                HoldContextMenuAction::new(
                    "toggle_equipped",
                    tr(if instance.is_equipped() { "gift_equip_take_off" } else { "gift_equip_put_on" }),
                )
                .system_image(if instance.is_equipped() { "xmark.circle" } else { "checkmark.circle" }),
            );
        }
        actions.push(
            HoldContextMenuAction::new("toggle_pinned", tr(if instance.is_pinned() { "gift_unpin" } else { "gift_pin" }))
                .system_image(if instance.is_pinned() { "pin.slash" } else { "pin" }),
        );
        actions.push(
            HoldContextMenuAction::new(
                "toggle_displayed",
                tr(if instance.is_displayed() { "gift_hide" } else { "gift_show" }),
            )
            .system_image(if instance.is_displayed() { "eye.slash" } else { "eye" }),
        );
        actions.push(HoldContextMenuAction::new("info", tr("gift_info_open")).system_image("gift"));
        actions
    }

    fn perform_owner_action(id: &str, instance: RiseGiftInstanceDTO, list: GiftsInstanceList, cx: &mut App) {
        let interactions = GiftsInteractionsStore::shared(cx);
        match id {
            "toggle_equipped" => interactions.update(cx, |store, cx| store.toggle_equipped(instance, cx)),
            "toggle_pinned" => interactions.update(cx, |store, cx| store.toggle_pinned_instance(instance, cx)),
            "toggle_displayed" => interactions.update(cx, |store, cx| store.toggle_displayed_instance(instance, cx)),
            "info" => interactions.update(cx, |store, cx| store.open_instance(instance, Some(list), cx)),
            _ => {}
        }
    }

    /// the list on screen, which the opened card pages through
    fn opened_list(owner_user_id: &str, kind: Option<RiseGiftItemKind>) -> GiftsInstanceList {
        GiftsInstanceList {
            owner_id: owner_user_id.to_owned(),
            kind,
        }
    }
}

impl RenderOnce for UserGiftsTab {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _: &mut Context<UserGiftsTabState>| {
            UserGiftsTabState::default()
        });
        let kind = state.read(cx).kind;
        let owner = state.read(cx).owner.clone();
        let width = state.read(cx).width;
        let snapshot_owner = self.owner_user_id.clone();
        let snapshot =
            window.use_keyed_state(self.child_id("snapshot"), cx, move |_, cx: &mut Context<UserGiftsTabSnapshot>| {
                UserGiftsTabSnapshot::new(snapshot_owner, kind, cx)
            });
        if snapshot.read(cx).kind != kind {
            snapshot.update(cx, |snapshot, cx| snapshot.set_kind(kind, cx));
        }
        let (whole, shown, items) = {
            let snapshot = snapshot.read(cx);
            (snapshot.whole, snapshot.shown, snapshot.items.clone())
        };
        let (whole_loaded, whole_has_more, whole_failed, whole_loading, whole_is_empty) =
            (whole.loaded, whole.has_more, whole.failed, whole.loading, whole.is_empty);
        let (shown_loaded, shown_count, shown_has_more, shown_loading) =
            (shown.loaded, items.len() as i64, shown.has_more, shown.loading);
        let is_owner = self.is_owner(cx);
        let closed = !is_owner && ProfileViewerPrivacy::closes_gifts(&self.owner_user_id, cx);

        let owner_user_id = self.owner_user_id.clone();
        let owner_state = state.clone();
        use_task(self.child_id("owner-task"), owner_user_id.clone(), window, cx, async move |cx| {
            let person_id = owner_user_id.clone();
            let _ = owner_state.update(cx, |state, cx| {
                state.owner = GiftPeople::person(&person_id, cx);
                cx.notify();
            });
            GiftsInteractionsStore::revalidate_tab(owner_user_id, cx).await;
        });
        let load_key = GiftsTabLoadKey {
            owner_id: self.owner_user_id.clone(),
            kind,
            loaded: whole_loaded,
            has_more: whole_has_more,
        };
        let load_owner = self.owner_user_id.clone();
        use_task(self.child_id("load-task"), load_key, window, cx, async move |cx| {
            GiftsInteractionsStore::show_tab(load_owner, kind, cx).await;
        });

        let mut column = div().relative().flex().flex_col().w_full();
        if closed {
            column = column.child(ChatProfileEmptyState::new("lock", tr("profile_section_hidden")));
        } else {
            let tabs_state = state.clone();
            let tabs_snapshot = snapshot.clone();
            column = column.child(GiftsKindTabs::new(
                self.child_id("kinds"),
                kind,
                move |next, cx: &mut App| {
                    tabs_snapshot.update(cx, |snapshot, cx| snapshot.set_kind(next, cx));
                    tabs_state.update(cx, |state, cx| {
                        state.kind = next;
                        cx.notify();
                    });
                },
            ));
        }
        if !closed && whole_failed && whole_is_empty {
            column = if whole_loading { column.child(Self::skeleton(width)) } else { column.child(self.failure(cx)) };
        } else if !closed && whole_loaded && whole_is_empty {
            column = column.child(ChatProfileEmptyState::new(
                "gift",
                tr(if is_owner { "gifts_empty_own" } else { "gifts_empty" }),
            ));
        } else if !closed {
            if !shown_loaded {
                column = column.child(Self::skeleton(width));
            } else if shown_count == 0 {
                column = column.child(ChatProfileEmptyState::new(
                    kind.map_or("gift", |kind| kind.empty_tab_icon()),
                    tr("gifts_filter_empty"),
                ));
            } else if let Some(width) = width {
                column = column.child(self.grid(
                    shown_count,
                    shown_has_more,
                    shown_loading,
                    kind,
                    owner,
                    is_owner,
                    width,
                    state.clone(),
                    items,
                    snapshot.clone(),
                ));
            } else {
                column = column.child(Self::skeleton(width));
            }
        }
        let width_state = state.clone();
        column.child(measure(self.child_id("width"), move |bounds, _, cx| {
            let width = f32::from(bounds.size.width);
            width_state.update(cx, |state, cx| {
                if state.width != Some(width) {
                    state.width = Some(width);
                    cx.notify();
                }
            });
        }))
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
struct UserGiftsTabFlags {
    loaded: bool,
    has_more: bool,
    failed: bool,
    loading: bool,
    is_empty: bool,
    has_cursor: bool,
}

impl UserGiftsTabFlags {
    fn of(tab: &GiftsTabState) -> Self {
        Self {
            loaded: tab.loaded,
            has_more: tab.has_more,
            failed: tab.failed,
            loading: tab.loading,
            is_empty: tab.items.is_empty(),
            has_cursor: tab.next_cursor.is_some(),
        }
    }
}

/// the owner whole tab and the chosen kind list as the page draws them, mirrored from the
/// services store
/// notifies the page only when these change not on other store writes
struct UserGiftsTabSnapshot {
    owner_user_id: String,
    kind: Option<RiseGiftItemKind>,
    whole: UserGiftsTabFlags,
    shown: UserGiftsTabFlags,
    fetched: UserGiftsTabFlags,
    items: Rc<Vec<RiseGiftInstanceDTO>>,
    _observation: Subscription,
}

impl UserGiftsTabSnapshot {
    fn new(owner_user_id: String, kind: Option<RiseGiftItemKind>, cx: &mut Context<Self>) -> Self {
        let services = GiftsServicesStore::shared(cx);
        let observation = cx.observe(&services, |snapshot, services, cx| {
            if snapshot.refresh(&services, cx) {
                cx.notify();
            }
        });
        let mut snapshot = Self {
            owner_user_id,
            kind,
            whole: UserGiftsTabFlags::default(),
            shown: UserGiftsTabFlags::default(),
            fetched: UserGiftsTabFlags::default(),
            items: Rc::default(),
            _observation: observation,
        };
        snapshot.refresh(&services, cx);
        snapshot
    }

    fn set_kind(&mut self, kind: Option<RiseGiftItemKind>, cx: &mut Context<Self>) {
        if self.kind == kind {
            return;
        }
        self.kind = kind;
        let services = GiftsServicesStore::shared(cx);
        self.refresh(&services, cx);
        cx.notify();
    }

    /// takes the store current tab, returns whether anything the page draws changed
    fn refresh(&mut self, services: &Entity<GiftsServicesStore>, cx: &App) -> bool {
        let store = services.read(cx);
        let whole = UserGiftsTabFlags::of(store.tab(&self.owner_user_id));
        let tab = store.tab_kind(&self.owner_user_id, self.kind);
        let shown = UserGiftsTabFlags::of(tab);
        let fetched = UserGiftsTabFlags::of(store.fetched_tab(&self.owner_user_id, self.kind));
        let same_items = self.items.as_slice() == tab.items.as_slice();
        if whole == self.whole && shown == self.shown && fetched == self.fetched && same_items {
            return false;
        }
        if !same_items {
            self.items = Rc::new(tab.items.clone());
        }
        self.whole = whole;
        self.shown = shown;
        self.fetched = fetched;
        true
    }
}

/// what every cell of one render shares, built once per render and cloned only into a cell
/// whose props changed
#[derive(Clone, PartialEq)]
struct UserGiftsCellShared {
    owner_user_id: String,
    kind: Option<RiseGiftItemKind>,
    owner: Option<GiftRecipient>,
    is_owner: bool,
    side: f32,
    item_count: i64,
    has_more: bool,
    is_loading_more: bool,
}

/// everything one grid cell draws from, any change renders the cell fresh and otherwise it
/// replays its last frame wherever the scroll moved it
#[derive(Clone, PartialEq)]
struct UserGiftsCellProps {
    shared: UserGiftsCellShared,
    index: i64,
    instance: RiseGiftInstanceDTO,
    is_menu_presented: bool,
}

impl UserGiftsCellProps {
    /// whether these props are what this render would build, compared without cloning
    fn matches(
        &self,
        shared: &UserGiftsCellShared,
        index: i64,
        instance: &RiseGiftInstanceDTO,
        is_menu_presented: bool,
    ) -> bool {
        self.index == index
            && self.is_menu_presented == is_menu_presented
            && self.shared == *shared
            && self.instance == *instance
    }
}

/// one grid cell as its own cached view
/// a scroll of the profile renders the tab page while cells only move, each replays its last
/// frame at the new origin instead of building its card and ribbon again
struct UserGiftsCellView {
    props: Option<UserGiftsCellProps>,
    state: Option<Entity<UserGiftsTabState>>,
}

impl UserGiftsCellView {
    fn new() -> Self {
        Self { props: None, state: None }
    }

    fn set_props(&mut self, props: Option<UserGiftsCellProps>, state: Entity<UserGiftsTabState>) {
        self.state = Some(state);
        if let Some(props) = props {
            self.props = Some(props);
        }
    }
}

impl Render for UserGiftsCellView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let side = self.props.as_ref().map_or(0.0, |props| props.shared.side);
        let cell = self.props.clone().zip(self.state.clone()).map(|(props, state)| UserGiftsTab::cell(props, state));
        div().w(px(side)).h(px(side)).children(cell)
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct GiftsTabLoadKey {
    owner_id: String,
    kind: Option<RiseGiftItemKind>,
    loaded: bool,
    has_more: bool,
}

#[derive(IntoElement)]
struct GiftsKindTabs {
    id: ElementId,
    selected: Option<RiseGiftItemKind>,
    on_select: std::rc::Rc<dyn Fn(Option<RiseGiftItemKind>, &mut App)>,
}

/// how chosen each tab looks, eased 0.18s ease out
struct GiftsKindTabsInks {
    selection: Vec<Animated<f32>>,
}

impl GiftsKindTabs {
    const HEIGHT: f32 = 40.0;
    const TAB_INSET: f32 = 4.0;
    const TAB_GAP: f32 = 8.0;

    fn new(
        id: ElementId,
        selected: Option<RiseGiftItemKind>,
        on_select: impl Fn(Option<RiseGiftItemKind>, &mut App) + 'static,
    ) -> Self {
        Self {
            id,
            selected,
            on_select: std::rc::Rc::new(on_select),
        }
    }

    fn kinds() -> Vec<Option<RiseGiftItemKind>> {
        std::iter::once(None)
            .chain(RiseGiftItemKind::SHOP_ORDER.iter().copied().map(Some))
            .collect()
    }

    fn title(kind: Option<RiseGiftItemKind>) -> SharedString {
        match kind {
            None => tr("gifts_filter_all").into(),
            Some(kind) => tr(kind.shop_tab_key()).into(),
        }
    }

    fn mix(from: Hsla, to: Hsla, amount: f32) -> Hsla {
        let from = from.to_rgb();
        let to = to.to_rgb();
        let lerp = |a: f32, b: f32| a + (b - a) * amount;
        gpui::Rgba {
            r: lerp(from.r, to.r),
            g: lerp(from.g, to.g),
            b: lerp(from.b, to.b),
            a: lerp(from.a, to.a),
        }
        .into()
    }
}

impl RenderOnce for GiftsKindTabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx);
        let (primary, secondary) = (theme.text.primary, theme.text.secondary);
        let kinds = Self::kinds();
        let selected = self.selected;
        let inks = window.use_keyed_state(self.id.clone(), cx, |_, _: &mut Context<GiftsKindTabsInks>| {
            GiftsKindTabsInks {
                selection: kinds
                    .iter()
                    .map(|kind| Animated::new(if *kind == selected { 1.0 } else { 0.0 }))
                    .collect(),
            }
        });
        let now = RiseClock::monotonic();
        inks.update(cx, |inks, cx| {
            for (ink, kind) in inks.selection.iter_mut().zip(kinds.iter()) {
                ink.set_in(if *kind == selected { 1.0 } else { 0.0 }, ease_out(0.18), now, cx);
            }
        });
        let amounts: Vec<f32> = inks.read(cx).selection.iter().map(|ink| ink.frame_value(now, window)).collect();

        let mut row = div()
            .flex()
            .flex_row()
            .items_stretch()
            .justify_between()
            .gap(px(Self::TAB_GAP))
            .w_full()
            .h(px(Self::HEIGHT))
            .px(px(GiftsGridMetrics::HORIZONTAL_PADDING));
        for (index, kind) in kinds.into_iter().enumerate() {
            let active = kind == selected;
            let amount = amounts.get(index).copied().unwrap_or(0.0);
            let on_select = self.on_select.clone();
            row = row.child(
                div()
                    .id(ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), format!("tab-{index}").into()))
                    .flex()
                    .flex_col()
                    .flex_shrink_1()
                    .min_w_0()
                    .justify_center()
                    .px(px(Self::TAB_INSET))
                    .cursor_pointer()
                    .text_size(AnimatedTabsMetrics::TAB_FONT_SIZE)
                    .font_weight(if active { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                    .text_color(Self::mix(secondary, primary, amount))
                    .on_click(move |_, _, cx| {
                        if active {
                            return;
                        }
                        AnimatedTabsHaptics::selection_changed();
                        on_select(kind, cx);
                    })
                    .child(MinimumScaleText::new(Self::title(kind), 0.7).line_limit(1)),
            );
        }
        row
    }
}

trait UserGiftsTabKindIcon {
    fn empty_tab_icon(self) -> &'static str;
}

impl UserGiftsTabKindIcon for RiseGiftItemKind {
    fn empty_tab_icon(self) -> &'static str {
        match self {
            RiseGiftItemKind::Item => "gift",
            RiseGiftItemKind::AvatarFrame => "person.crop.circle",
            RiseGiftItemKind::Nameplate => "person.text.rectangle",
            RiseGiftItemKind::ProfileEffect => "sparkles",
        }
    }
}
