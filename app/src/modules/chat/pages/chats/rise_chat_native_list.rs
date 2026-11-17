use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, App, AppContext, AsyncApp, AvailableSpace, Bounds, Context, Edges, ElementId, EmptyView, Entity,
    EntityId,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, RenderOnce, SharedString, Size, StyleRefinement, Styled,
    WeakEntity, Window, canvas, deferred, div, px,
};
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::language_store::LanguageStore;
use rise_theme::ThemeStore;
use rise_ui::frame_request::is_painted_on_screen;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::environment::{env_scope, layout_direction};
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::page_refresh::PageRefreshKeys;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListConformances;
use crate::core::widgets::wrappers::list_shell_header_collapse::{
    ListShellHeaderCollapse, ListShellHeaderCollapseProbe, ListShellHeaderCollapseProbeCoordinator,
};
use crate::core::widgets::wrappers::list_shell_ui::list_shell_floating_top_inset;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::{
    ChatListPerformanceProbe, Coordinator as ChatListPerformanceProbeCoordinator,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListItemProvider, RiseManualListReusableView,
    RiseManualListScrollMetrics, RiseManualListTransaction, RiseManualListTransactionAnchor,
    RiseManualListTransactionEdgeMutation,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::{
    RiseManualListDeferredRecyclePolicy, RiseManualListView,
};
use crate::modules::chat::components::chat_item::chat_list_row_activity::{ChatListRowActivity, ChatListRowActivityKey};
use crate::modules::chat::pages::chats::chats_archive_pull_gate::{
    ChatsArchivePullEvents, ChatsArchivePullGate, ChatsArchivePullGateCoordinator,
};
use crate::modules::chat::pages::chats::chats_archive_reveal::ChatsArchiveRevealPolicy;
use crate::modules::chat::pages::chats::chats_page_viewport_store::ChatsPageViewportStore;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_refresh_control::PostListRefreshControl;

const FOOTER_ID: &str = "rise-chat-list-footer";
const FOOTER_REUSE_IDENTIFIER: &str = "ChatListFooter";
const ROW_REUSE_IDENTIFIER: &str = "RiseManualMessageHostingView.Chat";
const PREPARATION_TURNS_PER_FRAME: f64 = 4.0;
const BACKGROUND_PREPARATION_TURNS_PER_FRAME: f64 = 1.0;
const ZONE_RECHECK_INTERVAL: Duration = Duration::from_secs(1);

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// false until the page list lands its first rows
/// own entity so other fields never invalidate the page body
#[derive(Default)]
pub struct RiseChatNativeListFirstRows {
    installed: bool,
}

impl RiseChatNativeListFirstRows {
    pub fn installed(&self) -> bool {
        self.installed
    }

    pub fn mark_installed(&mut self, cx: &mut Context<Self>) {
        if self.installed {
            return;
        }
        self.installed = true;
        cx.notify();
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        if !self.installed {
            return;
        }
        self.installed = false;
        cx.notify();
    }
}

#[derive(Default)]
pub struct RiseChatNativeListControl {
    list: RefCell<Option<WeakEntity<RiseManualListView>>>,
    first_rows: RefCell<Option<Entity<RiseChatNativeListFirstRows>>>,
    initial_top_offset: Cell<f32>,
    row_shifts: RefCell<HashMap<String, f32>>,
    lifted_row_id: RefCell<Option<String>>,
    settling_order: RefCell<Option<Vec<String>>>,
    settle_generation: Cell<u64>,
    requested_ids: RefCell<Vec<String>>,
    settle_extensions: Cell<u32>,
    known_hosts: RefCell<HashMap<String, WeakEntity<RiseChatNativeRowHost>>>,
    list_busy: Cell<bool>,
    missed_hosts: RefCell<Vec<String>>,
}

impl RiseChatNativeListControl {
    const SETTLE_DEADLINE: Duration = Duration::from_millis(1_500);
    const SETTLE_MAX_EXTENSIONS: u32 = 3;
    const SHIFT_DURATION: Duration = Duration::from_millis(300);

    pub fn new() -> Self {
        Self::default()
    }

    pub fn first_rows(&self, cx: &mut App) -> Entity<RiseChatNativeListFirstRows> {
        self.first_rows.borrow_mut().get_or_insert_with(|| cx.new(|_| RiseChatNativeListFirstRows::default())).clone()
    }

    pub fn note_requested_order(&self, ids: Vec<String>) {
        *self.requested_ids.borrow_mut() = ids;
    }

    pub fn row_shift(&self, id: &str) -> f32 {
        self.row_shifts.borrow().get(id).copied().unwrap_or(0.0)
    }

    fn row_host(&self, id: &str, cx: &App) -> Option<Entity<RiseChatNativeRowHost>> {
        if self.list_busy.get() {
            let known = self.known_hosts.borrow().get(id).and_then(WeakEntity::upgrade);
            let host = known.filter(|host| host.read(cx).hosts_row(id));
            if host.is_none() {
                self.missed_hosts.borrow_mut().push(id.to_owned());
            }
            return host;
        }
        let list = self.list()?;
        let host = list.read(cx).mounted_view(id)?.downcast::<RiseChatNativeRowHost>().ok()?;
        self.known_hosts.borrow_mut().insert(id.to_owned(), host.downgrade());
        Some(host)
    }

    pub fn lift_row(&self, id: Option<&str>, cx: &mut App) {
        if self.lifted_row_id.borrow().as_deref() == id {
            return;
        }
        let previous = self.lifted_row_id.borrow_mut().take();
        if let Some(host) = previous.and_then(|previous| self.row_host(&previous, cx)) {
            host.update(cx, |host, cx| host.set_lifted(false, cx));
        }
        *self.lifted_row_id.borrow_mut() = id.map(str::to_owned);
        if let Some(host) = id.and_then(|id| self.row_host(id, cx)) {
            host.update(cx, |host, cx| host.set_lifted(true, cx));
        }
    }

    pub fn shift_row(
        &self,
        id: &str,
        y: f32,
        animated: bool,
        completion: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut App,
    ) {
        let current = self.row_shift(id);
        if y == 0.0 {
            self.row_shifts.borrow_mut().remove(id);
        } else {
            self.row_shifts.borrow_mut().insert(id.to_owned(), y);
        }
        let host = self.row_host(id, cx);
        let Some(host) = host.filter(|_| animated && (current - y).abs() > 0.5) else {
            if let Some(host) = self.row_host(id, cx) {
                host.update(cx, |host, cx| host.set_shift(y, None, cx));
            }
            if let Some(completion) = completion {
                completion(cx);
            }
            return;
        };
        host.update(cx, |host, cx| host.set_shift(y, Some(spring(0.3, 0.86)), cx));
        if let Some(completion) = completion {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Self::SHIFT_DURATION).await;
                let _ = cx.update(completion);
            })
            .detach();
        }
    }

    pub fn settle_row_shifts(self: &Rc<Self>, order: Vec<String>, cx: &mut App) {
        *self.settling_order.borrow_mut() = Some(order);
        self.settle_extensions.set(0);
        self.schedule_settle_deadline(cx);
    }

    fn schedule_settle_deadline(self: &Rc<Self>, cx: &mut App) {
        let generation = self.settle_generation.get().wrapping_add(1);
        self.settle_generation.set(generation);
        let this = Rc::downgrade(self);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Self::SETTLE_DEADLINE).await;
            let _ = cx.update(|cx| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                if this.settle_generation.get() != generation || this.settling_order.borrow().is_none() {
                    return;
                }
                let requested = this.requested_ids.borrow().clone();
                if this.settle_extensions.get() < Self::SETTLE_MAX_EXTENSIONS && this.lands_settling_order(&requested) {
                    this.settle_extensions.set(this.settle_extensions.get() + 1);
                    this.schedule_settle_deadline(cx);
                    return;
                }
                *this.settling_order.borrow_mut() = None;
                this.clear_row_shifts(true, cx);
            });
        })
        .detach();
    }

    pub fn clear_row_shifts(&self, animated: bool, cx: &mut App) {
        self.settle_generation.set(self.settle_generation.get().wrapping_add(1));
        *self.settling_order.borrow_mut() = None;
        let ids: Vec<String> = self.row_shifts.borrow().keys().cloned().collect();
        for id in ids {
            self.shift_row(&id, 0.0, animated, None, cx);
        }
        self.lift_row(None, cx);
    }

    /// true when ids carry the order a released drag is waiting for
    pub fn lands_settling_order(&self, ids: &[String]) -> bool {
        let settling = self.settling_order.borrow();
        let Some(order) = settling.as_ref() else {
            return false;
        };
        let listed: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let members: HashSet<&str> = order.iter().map(String::as_str).collect();
        ids.iter().filter(|id| members.contains(id.as_str())).eq(order.iter().filter(|id| listed.contains(id.as_str())))
    }

    pub fn transaction_applied(self: &Rc<Self>, ids: &[String], cx: &mut App) {
        if !self.lands_settling_order(ids) {
            return;
        }
        self.list_busy.set(true);
        self.clear_row_shifts(false, cx);
        self.list_busy.set(false);
        let missed = std::mem::take(&mut *self.missed_hosts.borrow_mut());
        if missed.is_empty() {
            return;
        }
        let this = self.clone();
        cx.defer(move |cx| {
            for id in missed {
                if this.row_shifts.borrow().contains_key(&id) || this.lifted_row_id.borrow().as_deref() == Some(id.as_str()) {
                    continue;
                }
                if let Some(host) = this.row_host(&id, cx) {
                    host.update(cx, |host, cx| {
                        host.set_shift(0.0, None, cx);
                        host.set_lifted(false, cx);
                    });
                }
            }
        });
    }

    pub fn list(&self) -> Option<Entity<RiseManualListView>> {
        self.list.borrow().as_ref().and_then(WeakEntity::upgrade)
    }

    pub fn normal_top(&self, window: &mut Window, cx: &mut App) {
        let Some(list) = self.list() else {
            return;
        };
        let initial_top_offset = self.initial_top_offset.get();
        list.update(cx, |list, cx| {
            let target = list.content_top_offset_y() + px(initial_top_offset);
            let _ = list.set_content_offset_y(target, false, true, window, cx);
        });
    }

    pub fn archive_marker_y(&self, cx: &App) -> Option<f32> {
        let list = self.list()?;
        let list = list.read(cx);
        Some(ChatsArchiveRevealPolicy::marker_y(f32::from(
            list.content_top_offset_y() - list.scroll_metrics().content_offset_y,
        )))
    }
}

pub type RiseChatNativeListRowElement = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

#[derive(Clone)]
pub struct RiseChatNativeListRow {
    pub id: String,
    pub version: i64,
    pub content: RiseChatNativeListRowElement,
    pub measurement: RiseChatNativeListRowElement,
    pub geometry_version: Option<i64>,
}

/// same floating origin edge transactions as message history
/// row sizes are measured before install, bounded main thread work per frame
/// a shift never resets momentum
#[derive(IntoElement)]
pub struct RiseChatNativeList {
    id: ElementId,
    rows: Rc<Vec<RiseChatNativeListRow>>,
    header: Option<RiseChatNativeListRow>,
    footer_height: f32,
    minimum_content_height: f32,
    top_inset: f32,
    initial_top_offset: f32,
    scroll_enabled: bool,
    is_active: bool,
    rows_active: Option<bool>,
    restoration_ready: bool,
    hold_rows: bool,
    page_id: String,
    bookmarks: Option<Rc<ChatsPageViewportStore>>,
    control: Option<Rc<RiseChatNativeListControl>>,
    collapse: Option<Entity<ListShellHeaderCollapse>>,
    archive_events: Option<ChatsArchivePullEvents>,
    archive_enabled: bool,
    on_metrics: Option<Rc<dyn Fn(&RiseManualListScrollMetrics, &mut Window, &mut App)>>,
    on_visible: Option<Rc<dyn Fn(&[String], &mut Window, &mut App)>>,
    on_refresh: Option<Rc<dyn Fn(&mut App)>>,
}

impl RiseChatNativeList {
    /// a page that keeps its rows passes the same Rc again so the list skips rehashing
    pub fn new(id: impl Into<ElementId>, rows: impl Into<Rc<Vec<RiseChatNativeListRow>>>) -> Self {
        Self {
            id: id.into(),
            rows: rows.into(),
            header: None,
            footer_height: 0.0,
            minimum_content_height: 0.0,
            top_inset: 0.0,
            initial_top_offset: 0.0,
            scroll_enabled: true,
            is_active: true,
            rows_active: None,
            restoration_ready: true,
            hold_rows: false,
            page_id: String::new(),
            bookmarks: None,
            control: None,
            collapse: None,
            archive_events: None,
            archive_enabled: false,
            on_metrics: None,
            on_visible: None,
            on_refresh: None,
        }
    }

    pub fn header(mut self, header: Option<RiseChatNativeListRow>) -> Self {
        self.header = header;
        self
    }

    pub fn footer_height(mut self, footer_height: f32) -> Self {
        self.footer_height = footer_height;
        self
    }

    pub fn minimum_content_height(mut self, minimum_content_height: f32) -> Self {
        self.minimum_content_height = minimum_content_height;
        self
    }

    pub fn top_inset(mut self, top_inset: f32) -> Self {
        self.top_inset = top_inset;
        self
    }

    pub fn initial_top_offset(mut self, initial_top_offset: f32) -> Self {
        self.initial_top_offset = initial_top_offset;
        self
    }

    pub fn scroll_enabled(mut self, scroll_enabled: bool) -> Self {
        self.scroll_enabled = scroll_enabled;
        self
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    /// row carrier activity when it differs from the list own, is_active otherwise
    pub fn rows_active(mut self, rows_active: bool) -> Self {
        self.rows_active = Some(rows_active);
        self
    }

    pub fn restoration_ready(mut self, restoration_ready: bool) -> Self {
        self.restoration_ready = restoration_ready;
        self
    }

    /// keeps rows off screen this frame, list keeps its state and installs them later
    pub fn hold_rows(mut self, hold_rows: bool) -> Self {
        self.hold_rows = hold_rows;
        self
    }

    pub fn page_id(mut self, page_id: impl Into<String>) -> Self {
        self.page_id = page_id.into();
        self
    }

    pub fn bookmarks(mut self, bookmarks: Option<Rc<ChatsPageViewportStore>>) -> Self {
        self.bookmarks = bookmarks;
        self
    }

    pub fn control(mut self, control: Option<Rc<RiseChatNativeListControl>>) -> Self {
        self.control = control;
        self
    }

    pub fn collapse(mut self, collapse: Option<Entity<ListShellHeaderCollapse>>) -> Self {
        self.collapse = collapse;
        self
    }

    pub fn archive_events(mut self, archive_events: Option<ChatsArchivePullEvents>) -> Self {
        self.archive_events = archive_events;
        self
    }

    pub fn archive_enabled(mut self, archive_enabled: bool) -> Self {
        self.archive_enabled = archive_enabled;
        self
    }

    /// runs one turn after the list reported, since the list is mid update while it reports
    pub fn on_metrics(mut self, on_metrics: impl Fn(&RiseManualListScrollMetrics, &mut Window, &mut App) + 'static) -> Self {
        self.on_metrics = Some(Rc::new(on_metrics));
        self
    }

    pub fn on_visible(mut self, on_visible: impl Fn(&[String], &mut Window, &mut App) + 'static) -> Self {
        self.on_visible = Some(Rc::new(on_visible));
        self
    }

    pub fn on_refresh(mut self, on_refresh: Option<Rc<dyn Fn(&mut App)>>) -> Self {
        self.on_refresh = on_refresh;
        self
    }
}

impl RenderOnce for RiseChatNativeList {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.top_inset == 0.0 {
            self.top_inset = list_shell_floating_top_inset(cx);
        }
        let id = self.id.clone();
        let state = window.use_keyed_state::<Rc<RiseChatNativeListCoordinator>>(id.clone(), cx, |window, cx| {
            cx.on_release(|coordinator: &mut Rc<RiseChatNativeListCoordinator>, cx: &mut App| coordinator.stop(cx))
                .detach();
            RiseChatNativeListCoordinator::new(window, cx)
        });
        let coordinator = state.read(cx).clone();
        let value = Rc::new(self);
        let (collapse_coordinator, archive_coordinator, refresh_control) = coordinator.update(value.clone(), cx);

        let applying = coordinator.clone();
        window.defer(cx, move |window, cx| applying.apply(window, cx));

        let list = coordinator.list.clone();
        let preparing = coordinator.clone();
        let content = div()
            .relative()
            .size_full()
            .child(
                canvas(move |bounds, window, cx| preparing.prepare(bounds, window, cx), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .when_some(value.collapse.clone().zip(collapse_coordinator), |content, (collapse, coordinator)| {
                content.child(
                    ListShellHeaderCollapseProbe::new(child_id(&id, "collapse"), collapse, value.page_id.clone())
                        .rest_offset(value.initial_top_offset)
                        .manual_list(list.downgrade().into())
                        .coordinator(coordinator),
                )
            })
            .when_some(archive_coordinator, |content, coordinator| content.child(ChatsArchivePullGate::new(coordinator)))
            .when(!value.hold_rows, |content| {
                content.child(AnyView::from(list.clone()).cached(StyleRefinement::default().size_full()))
            })
            .when_some(refresh_control.as_ref(), |content, control| {
                content.child(control.read(cx).indicator_view.clone())
            });

        match refresh_control {
            Some(control) => PageRefreshKeys::new(
                child_id(&id, "refresh-keys"),
                move |window, cx| control.update(cx, |control, cx| control.request_refresh(window, cx)),
                content,
            )
            .into_any_element(),
            None => content.into_any_element(),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct RiseChatNativeListPreparationKey {
    signature: u64,
    width: f32,
    footer_height: f32,
    minimum_content_height: f32,
    top_inset: f32,
}

struct RiseChatNativePreparedSignature {
    rows: Rc<Vec<RiseChatNativeListRow>>,
    header: Option<(String, i64, Option<i64>)>,
    environment_geometry: u64,
    border: gpui::Hsla,
    signature: u64,
}

struct RiseChatNativeListPreparation {
    token: u64,
    value: Rc<RiseChatNativeList>,
    source: Vec<RiseChatNativeListRow>,
    width: f32,
    footer_height: f32,
    measurements: HashMap<String, f32>,
    minimum_content_height: f32,
    next_row: usize,
    first_screen_start: usize,
    first_screen_height: f32,
    first_screen_filled: f32,
    first_screen_sent: bool,
}

#[derive(Clone, Copy)]
struct RiseChatNativeListMeasuredHeight {
    version: i64,
    width: f32,
    height: f32,
}

struct RiseChatNativeListCoordinator {
    this: Weak<Self>,
    list: Entity<RiseManualListView>,
    value: RefCell<Option<Rc<RiseChatNativeList>>>,
    generation: Cell<u64>,
    applied_ids: RefCell<Vec<String>>,
    applied_versions: RefCell<HashMap<String, i64>>,
    heights: RefCell<HashMap<String, RiseChatNativeListMeasuredHeight>>,
    collapse_coordinator: RefCell<Option<Entity<ListShellHeaderCollapseProbeCoordinator>>>,
    archive_coordinator: RefCell<Option<Entity<ChatsArchivePullGateCoordinator>>>,
    refresh_control: RefCell<Option<Entity<PostListRefreshControl>>>,
    refresh_tint: Cell<Option<gpui::Hsla>>,
    restored: Cell<bool>,
    last_width: Cell<f32>,
    applied_base_top_inset: Cell<Option<f32>>,
    trace: RefCell<ChatListPerformanceProbeCoordinator>,
    row_activity: Entity<ChatListRowActivity>,
    attached_rows_signature: Cell<Option<u64>>,
    applied_rows: RefCell<Option<Rc<Vec<RiseChatNativeListRow>>>>,
    prepared_signature: RefCell<Option<RiseChatNativePreparedSignature>>,
    preparation_key: Cell<Option<RiseChatNativeListPreparationKey>>,
    preparation: RefCell<Option<RiseChatNativeListPreparation>>,
    zone_signature: Cell<Option<(Duration, u64)>>,
    is_stopped: Cell<bool>,
    parked_retry: Cell<(u64, u32)>,
    parked_retry_pending: Cell<bool>,
    host_view: Cell<Option<EntityId>>,
}

impl RiseChatNativeListCoordinator {
    fn new(window: &mut Window, cx: &mut Context<Rc<Self>>) -> Rc<Self> {
        let list = cx.new(|cx| {
            let mut list = RiseManualListView::new(window, cx);
            list.rests_short_content_at_top = true;
            list.warms_mounted_rows = true;
            list
        });
        let row_activity = cx.new(|_| ChatListRowActivity::new());
        let trace = ChatListPerformanceProbe { is_active: false }.make_coordinator();
        let coordinator = Rc::new_cyclic(|this| Self {
            this: this.clone(),
            list,
            value: RefCell::new(None),
            generation: Cell::new(0),
            applied_ids: RefCell::new(Vec::new()),
            applied_versions: RefCell::new(HashMap::new()),
            heights: RefCell::new(HashMap::new()),
            collapse_coordinator: RefCell::new(None),
            archive_coordinator: RefCell::new(None),
            refresh_control: RefCell::new(None),
            refresh_tint: Cell::new(None),
            restored: Cell::new(false),
            last_width: Cell::new(0.0),
            applied_base_top_inset: Cell::new(None),
            trace: RefCell::new(trace),
            row_activity,
            attached_rows_signature: Cell::new(None),
            applied_rows: RefCell::new(None),
            prepared_signature: RefCell::new(None),
            preparation_key: Cell::new(None),
            preparation: RefCell::new(None),
            zone_signature: Cell::new(None),
            is_stopped: Cell::new(false),
            parked_retry: Cell::new((0, 0)),
            parked_retry_pending: Cell::new(false),
            host_view: Cell::new(None),
        });
        coordinator.attach(cx);
        coordinator
    }

    fn current_value(&self) -> Option<Rc<RiseChatNativeList>> {
        self.value.borrow().clone()
    }

    fn attach(&self, cx: &mut App) {
        let boundary_owner = self.this.clone();
        let width_owner = self.this.clone();
        let visible_owner = self.this.clone();
        let metrics_owner = self.this.clone();
        self.list.update(cx, |list, _| {
            list.leading_scroll_boundary = Some(Rc::new(move |cx: &App| -> Option<Pixels> {
                let value = boundary_owner.upgrade()?.current_value()?;
                if value.initial_top_offset <= 0.0 {
                    return None;
                }
                let events = value.archive_events.as_ref()?;
                let allows = value.archive_enabled && (events.allows_exposure)(cx);
                Some(if allows { px(0.0) } else { px(value.initial_top_offset) })
            }));
            list.on_viewport_width_changed = Some(Rc::new(move |width: Pixels, _window: &mut Window, cx: &mut App| {
                let Some(this) = width_owner.upgrade() else {
                    return;
                };
                let control = this.refresh_control.borrow().clone();
                if let Some(control) = control {
                    control.update(cx, |control, cx| control.layout_indicator(f32::from(width), cx));
                }
                this.rows_missed_width(f32::from(width), cx);
            }));
            list.on_visible_ids_changed = Some(Rc::new(move |ids: &[String], window: &mut Window, cx: &mut App| {
                let Some(on_visible) = visible_owner.upgrade().and_then(|this| this.current_value()?.on_visible.clone())
                else {
                    return;
                };
                let ids = ids.to_vec();
                window.defer(cx, move |window, cx| on_visible(&ids, window, cx));
            }));
            list.on_scroll_metrics_changed = Some(Rc::new(
                move |metrics: &RiseManualListScrollMetrics, window: &mut Window, cx: &mut App| {
                    let Some(this) = metrics_owner.upgrade() else {
                        return;
                    };
                    let archive_coordinator = this.archive_coordinator.borrow().clone();
                    if let Some(coordinator) = archive_coordinator {
                        cx.defer(move |cx| {
                            coordinator.update(cx, |coordinator, cx| coordinator.scroll_state_did_change(cx));
                        });
                    }
                    let control = this.refresh_control.borrow().clone();
                    if let Some(control) = control {
                        control.update(cx, |control, cx| control.observe(metrics, window, cx));
                    }
                    if let Some(on_metrics) = this.current_value().and_then(|value| value.on_metrics.clone()) {
                        let metrics = *metrics;
                        window.defer(cx, move |window, cx| on_metrics(&metrics, window, cx));
                    }
                },
            ));
        });
    }

    /// render half of update, what the element tree needs now
    /// the rest is apply
    #[allow(clippy::type_complexity)]
    fn update(
        &self,
        value: Rc<RiseChatNativeList>,
        cx: &mut App,
    ) -> (
        Option<Entity<ListShellHeaderCollapseProbeCoordinator>>,
        Option<Entity<ChatsArchivePullGateCoordinator>>,
        Option<Entity<PostListRefreshControl>>,
    ) {
        if let Some(bookmarks) = &value.bookmarks {
            let account = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
            let key = format!(
                "{}|{}",
                account.as_deref().unwrap_or_default(),
                SaiAccountCacheScope::active_epoch().as_deref().unwrap_or_default()
            );
            bookmarks.activate_account(&key);
        }
        *self.value.borrow_mut() = Some(value.clone());
        if let Some(collapse) = &value.collapse {
            let mut slot = self.collapse_coordinator.borrow_mut();
            if slot.is_none() {
                let collapse = collapse.clone();
                let page_id = value.page_id.clone();
                let rest_offset = value.initial_top_offset;
                *slot = Some(cx.new(|_| ListShellHeaderCollapseProbeCoordinator::new(collapse, page_id, rest_offset)));
            }
        }
        if let Some(events) = &value.archive_events {
            let mut slot = self.archive_coordinator.borrow_mut();
            if slot.is_none() {
                let events = events.clone();
                let is_enabled = value.archive_enabled;
                *slot = Some(cx.new(|_| ChatsArchivePullGateCoordinator::new(events, is_enabled)));
            }
        }
        if value.on_refresh.is_some() && self.refresh_control.borrow().is_none() {
            let refresh_owner = self.this.clone();
            let inset_owner = self.this.clone();
            let control = cx.new(|cx| {
                let mut control = PostListRefreshControl::new(cx);
                control.on_refresh = Some(Rc::new(move |cx: AsyncApp| {
                    let on_refresh = refresh_owner.upgrade().and_then(|this| this.current_value()?.on_refresh.clone());
                    async move {
                        if let Some(on_refresh) = on_refresh {
                            cx.update(|cx| on_refresh(cx));
                        }
                    }
                    .boxed_local()
                }));
                control.on_top_inset_change = Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                    if let Some(this) = inset_owner.upgrade() {
                        this.apply_viewport_insets(window, cx);
                    }
                }));
                control
            });
            *self.refresh_control.borrow_mut() = Some(control);
        }
        (
            self.collapse_coordinator.borrow().clone(),
            self.archive_coordinator.borrow().clone(),
            self.refresh_control.borrow().clone(),
        )
    }

    /// other half of update, runs one turn after render and every write here notifies
    fn apply(&self, window: &mut Window, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let Some(value) = self.current_value() else {
            return;
        };
        let is_active = value.rows_active.unwrap_or(value.is_active);
        self.row_activity.update(cx, |activity, cx| activity.set_active(is_active, cx));
        self.apply_viewport_insets(window, cx);
        let scroll_enabled = value.scroll_enabled;
        self.list.update(cx, |list, cx| {
            if list.is_scroll_enabled() != scroll_enabled {
                list.set_is_scroll_enabled(scroll_enabled, cx);
            }
        });
        let weak_list = self.list.downgrade();
        let rows_unchanged = self.applied_rows.borrow().as_ref().is_some_and(|rows| Rc::ptr_eq(rows, &value.rows));
        if let Some(control) = &value.control {
            let replaced = control.list.borrow().as_ref().is_some_and(|old| *old != weak_list);
            *control.list.borrow_mut() = Some(weak_list.clone());
            if replaced {
                control.first_rows(cx).update(cx, |first_rows, cx| first_rows.reset(cx));
            }
            control.initial_top_offset.set(value.initial_top_offset);
            if !rows_unchanged || replaced {
                control.note_requested_order(value.rows.iter().map(|row| row.id.clone()).collect());
            }
        }
        if let Some(bookmarks) = value.bookmarks.as_ref().filter(|_| !rows_unchanged) {
            let mut signature = DefaultHasher::new();
            value.page_id.hash(&mut signature);
            for row in value.rows.iter() {
                row.id.hash(&mut signature);
            }
            let signature = signature.finish();
            if self.attached_rows_signature.get() != Some(signature) {
                self.attached_rows_signature.set(Some(signature));
                let ids = value.rows.iter().map(|row| row.id.clone()).collect();
                bookmarks.attach_native(weak_list.clone(), ids, &value.page_id);
            }
        }
        *self.applied_rows.borrow_mut() = Some(value.rows.clone());
        let scroll_handle = self.list.read(cx).integration_scroll_handle();
        ChatListPerformanceProbe { is_active: value.is_active }.update_ui_view(&scroll_handle, &mut self.trace.borrow_mut());
        let archive_coordinator = self.archive_coordinator.borrow().clone();
        if let (Some(coordinator), Some(events)) = (archive_coordinator, value.archive_events.clone()) {
            let is_enabled = value.archive_enabled;
            let list = weak_list.clone();
            coordinator.update(cx, |coordinator, cx| {
                coordinator.update(events, is_enabled, cx);
                coordinator.attach(list, cx);
            });
        }
        let refresh_control = self.refresh_control.borrow().clone();
        if let Some(control) = refresh_control {
            let is_enabled = value.on_refresh.is_some() && value.is_active;
            let base_top_inset = value.top_inset;
            let width = f32::from(self.list.read(cx).bounds().size.width);
            let tint = ThemeStore::shared(cx).read(cx).current_theme().text.secondary;
            let tint_changed = self.refresh_tint.get() != Some(tint);
            self.refresh_tint.set(Some(tint));
            control.update(cx, |control, cx| {
                control.set_is_enabled(is_enabled, cx);
                control.set_base_top_inset(base_top_inset, cx);
                if tint_changed {
                    control.indicator_view.update(cx, |indicator, cx| indicator.set_tint(tint, cx));
                }
                control.layout_indicator(width, cx);
            });
        }
    }

    fn apply_viewport_insets(&self, window: &mut Window, cx: &mut App) {
        let Some(value) = self.current_value() else {
            return;
        };
        let held_inset = self.refresh_control.borrow().as_ref().map_or(0.0, |control| control.read(cx).held_inset());
        let insets = Edges { top: px(value.top_inset + held_inset), left: px(0.0), bottom: px(0.0), right: px(0.0) };
        let previous_base = self.applied_base_top_inset.replace(Some(value.top_inset));
        let resting_offset = value
            .bookmarks
            .as_ref()
            .and_then(|bookmarks| bookmarks.resting_offset(&value.page_id))
            .unwrap_or(value.initial_top_offset);
        self.list.update(cx, |list, cx| {
            if list.viewport_insets() == insets {
                return;
            }
            let base_changed = previous_base.is_some_and(|previous| previous != value.top_inset);
            let metrics = list.scroll_metrics();
            let is_idle = !metrics.is_tracking && !metrics.is_dragging && !metrics.is_decelerating;
            let rests = resting_offset > 2.0
                && (metrics.content_offset_y - (list.content_top_offset_y() + px(resting_offset))).abs() <= px(0.5);
            list.set_viewport_insets(insets, window, cx);
            if base_changed && is_idle && rests {
                let target = list.content_top_offset_y() + px(resting_offset);
                let _ = list.set_content_offset_y(target, false, true, window, cx);
            }
        });
    }

    /// a chat landing above the first at the top pushes rows down
    /// the first chat keeps the old first place so hidden archive row stays
    /// if scrolled away or held the reading position wins
    fn pinned_top(
        &self,
        value: &RiseChatNativeList,
        ids: &[String],
        cx: &App,
    ) -> Option<RiseManualListTransactionAnchor> {
        let list = self.list.read(cx);
        let metrics = list.scroll_metrics();
        if metrics.is_tracking || metrics.is_dragging || metrics.is_decelerating {
            return None;
        }
        let resting_offset_y = list.content_top_offset_y() + px(value.initial_top_offset);
        if metrics.content_offset_y > resting_offset_y + px(0.5) {
            return None;
        }
        let header_id = value.header.as_ref().map(|header| header.id.as_str());
        let is_chat = |id: &&String| Some(id.as_str()) != header_id && id.as_str() != FOOTER_ID;
        let applied_ids = self.applied_ids.borrow();
        let applied_first = applied_ids.iter().find(is_chat)?;
        let first = ids.iter().find(is_chat)?;
        let frame = list.viewport_frame(applied_first)?;
        Some(RiseManualListTransactionAnchor::Item { id: first.clone(), viewport_offset: f32::from(frame.origin.y) })
    }

    fn stop(&self, cx: &mut App) {
        self.is_stopped.set(true);
        self.generation.set(self.generation.get().wrapping_add(1));
        self.preparation.borrow_mut().take();
        let row_activity = self.row_activity.clone();
        cx.defer(move |cx| row_activity.update(cx, |activity, cx| activity.set_active(false, cx)));
        if let Some(value) = self.current_value()
            && let Some(bookmarks) = &value.bookmarks
        {
            bookmarks.detach(&self.list.downgrade(), &value.page_id, cx);
        }
        let collapse_coordinator = self.collapse_coordinator.borrow().clone();
        if let Some(coordinator) = collapse_coordinator {
            coordinator.update(cx, |coordinator, cx| coordinator.detach(cx));
        }
        let archive_coordinator = self.archive_coordinator.borrow().clone();
        if let Some(coordinator) = archive_coordinator {
            coordinator.update(cx, |coordinator, _| coordinator.detach());
        }
        let refresh_control = self.refresh_control.borrow_mut().take();
        if let Some(control) = refresh_control {
            control.update(cx, |control, cx| control.stop(cx));
        }
        self.refresh_tint.set(None);
        ChatListPerformanceProbe::dismantle_ui_view(&mut self.trace.borrow_mut());
        self.list.update(cx, |list, _| {
            list.on_visible_ids_changed = None;
            list.on_scroll_metrics_changed = None;
            list.on_viewport_width_changed = None;
            list.leading_scroll_boundary = None;
        });
        self.value.borrow_mut().take();
        self.heights.borrow_mut().clear();
        self.applied_rows.borrow_mut().take();
        self.prepared_signature.borrow_mut().take();
        self.preparation_key.set(None);
    }

    /// reading the time zone is a system call so it checks once a second not every frame
    fn zone_signature(&self) -> u64 {
        let now = RiseClock::monotonic();
        if let Some((checked_at, signature)) = self.zone_signature.get()
            && now.saturating_sub(checked_at) < ZONE_RECHECK_INTERVAL
        {
            return signature;
        }
        let mut hasher = DefaultHasher::new();
        format!("{:?}", TimeZone::current()).hash(&mut hasher);
        let signature = hasher.finish();
        self.zone_signature.set(Some((now, signature)));
        signature
    }

    /// runs in prepaint of every frame and sizes the rows there
    /// so they measure with the env and state they get drawn with
    fn prepare(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let Some(value) = self.current_value() else {
            return;
        };
        let width = f32::from(bounds.size.width);
        self.host_view.set(Some(window.current_view()));
        if self.is_stopped.get() || !value.restoration_ready || width <= 1.0 {
            return;
        }
        let scale = window.scale_factor().max(1.0);
        let theme = ThemeStore::shared(cx).read(cx);
        let text_delta = theme.text_delta();
        let border = theme.current_theme().border._100;
        let locale = LanguageStore::shared(cx).locale();

        let mut environment_geometry = DefaultHasher::new();
        text_delta.to_bits().hash(&mut environment_geometry);
        locale.hash(&mut environment_geometry);
        self.zone_signature().hash(&mut environment_geometry);
        layout_direction(cx).hash(&mut environment_geometry);
        scale.to_bits().hash(&mut environment_geometry);
        let environment_geometry = environment_geometry.finish();

        let minimum_content_height = if value.minimum_content_height > 0.0 {
            value.minimum_content_height
        } else {
            (f32::from(bounds.size.height) - value.top_inset).max(0.0)
        };

        let row_versions = |row: &RiseChatNativeListRow| -> (i64, i64) {
            let mut geometry = DefaultHasher::new();
            row.geometry_version.unwrap_or(row.version).hash(&mut geometry);
            environment_geometry.hash(&mut geometry);
            let geometry_version = geometry.finish() as i64;
            let mut content = DefaultHasher::new();
            row.version.hash(&mut content);
            geometry_version.hash(&mut content);
            border.hash(&mut content);
            (content.finish() as i64, geometry_version)
        };

        let rows = value.header.iter().chain(value.rows.iter());
        let header_key = value.header.as_ref().map(|header| (header.id.clone(), header.version, header.geometry_version));
        let kept_signature = self.prepared_signature.borrow().as_ref().and_then(|kept| {
            (Rc::ptr_eq(&kept.rows, &value.rows)
                && kept.header == header_key
                && kept.environment_geometry == environment_geometry
                && kept.border == border)
                .then_some(kept.signature)
        });
        let signature = match kept_signature {
            Some(signature) => signature,
            None => {
                let mut signature = DefaultHasher::new();
                for row in rows.clone() {
                    row.id.hash(&mut signature);
                    row_versions(row).hash(&mut signature);
                }
                let signature = signature.finish();
                *self.prepared_signature.borrow_mut() = Some(RiseChatNativePreparedSignature {
                    rows: value.rows.clone(),
                    header: header_key,
                    environment_geometry,
                    border,
                    signature,
                });
                signature
            }
        };
        let key = RiseChatNativeListPreparationKey {
            signature,
            width,
            footer_height: value.footer_height,
            minimum_content_height,
            top_inset: value.top_inset,
        };
        if self.preparation_key.get() != Some(key) {
            self.preparation_key.set(Some(key));
            let token = self.generation.get().wrapping_add(1);
            x509_mark(|| {
                let head = value.rows.first().map(|row| row.id.as_str()).unwrap_or_default();
                format!("list {} prepare token={token} rows={} head={head} active={} width={width}", value.page_id, value.rows.len(), value.is_active)
            });
            self.generation.set(token);
            let source: Vec<RiseChatNativeListRow> = rows
                .map(|row| {
                    let (version, geometry_version) = row_versions(row);
                    RiseChatNativeListRow {
                        id: row.id.clone(),
                        version,
                        content: row.content.clone(),
                        measurement: row.measurement.clone(),
                        geometry_version: Some(geometry_version),
                    }
                })
                .collect();
            let retained: HashSet<String> = source.iter().map(|row| row.id.clone()).collect();
            self.heights.borrow_mut().retain(|id, _| retained.contains(id));
            let first_install = !self.restored.get() && self.applied_ids.borrow().is_empty();
            let first_screen_start = if first_install {
                value
                    .bookmarks
                    .as_ref()
                    .and_then(|bookmarks| bookmarks.restoration(&value.page_id, Some(&retained)))
                    .and_then(|(saved, _)| source.iter().position(|row| row.id == saved))
                    .unwrap_or(0)
            } else {
                0
            };
            let first_screen_height = f32::from(self.list.read(cx).bounds().size.height);
            *self.preparation.borrow_mut() = Some(RiseChatNativeListPreparation {
                token,
                value: value.clone(),
                source,
                width,
                footer_height: value.footer_height,
                measurements: HashMap::new(),
                minimum_content_height,
                next_row: 0,
                first_screen_start,
                first_screen_height,
                first_screen_filled: 0.0,
                first_screen_sent: !first_install || first_screen_height <= 0.0,
            });
        }
        if !is_painted_on_screen(bounds, window) {
            self.retry_parked_preparation(&value, window, cx);
            return;
        }
        self.advance_preparation(window, cx);
    }

    fn retry_parked_preparation(&self, value: &RiseChatNativeList, window: &mut Window, cx: &mut App) {
        const PARKED_RETRY_INTERVAL: Duration = Duration::from_millis(120);
        const PARKED_RETRY_LIMIT: u32 = 6;
        if !value.is_active || self.parked_retry_pending.get() || self.preparation.borrow().is_none() {
            return;
        }
        let token = self.generation.get();
        let (retry_token, retries) = self.parked_retry.get();
        let retries = if retry_token == token { retries } else { 0 };
        if retries >= PARKED_RETRY_LIMIT {
            return;
        }
        self.parked_retry.set((token, retries + 1));
        self.parked_retry_pending.set(true);
        x509_mark(|| format!("list {} prepare parked off screen token={token} retry={}", value.page_id, retries + 1));
        let host = window.current_view();
        let this = self.this.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(PARKED_RETRY_INTERVAL).await;
            let _ = cx.update(|cx| {
                let Some(this) = this.upgrade() else {
                    return;
                };
                this.parked_retry_pending.set(false);
                if this.is_stopped.get() || this.generation.get() != token || this.preparation.borrow().is_none() {
                    return;
                }
                cx.notify(host);
            });
        })
        .detach();
    }

    fn advance_preparation(&self, window: &mut Window, cx: &mut App) {
        let Some(mut preparation) = self.preparation.borrow_mut().take() else {
            return;
        };
        let turns = if preparation.first_screen_sent {
            BACKGROUND_PREPARATION_TURNS_PER_FRAME
        } else {
            PREPARATION_TURNS_PER_FRAME
        };
        let budget = Duration::from_secs_f64(RiseManualListDeferredRecyclePolicy::TIME_BUDGET * turns);
        let turn_started = RiseClock::monotonic();
        let scale = window.scale_factor().max(1.0);
        while preparation.next_row < preparation.source.len() {
            let row = preparation.source[preparation.next_row].clone();
            let version = row.geometry_version.unwrap_or(row.version);
            let cached = self.heights.borrow().get(&row.id).copied();
            let index = preparation.next_row;
            let mut measured_now = false;
            let height = if let Some(cached) = cached
                && cached.version == version
                && cached.width == preparation.width
            {
                cached.height
            } else {
                let measurement = (row.measurement)(window, cx);
                let mut element = env_scope::<ChatListRowActivityKey>(Some(self.row_activity.clone()))
                    .child(measurement)
                    .into_any_element();
                let raw = f32::from(
                    element
                        .layout_as_root(
                            Size {
                                width: AvailableSpace::Definite(px(preparation.width)),
                                height: AvailableSpace::MinContent,
                            },
                            window,
                            cx,
                        )
                        .height,
                );
                if !raw.is_finite() || raw <= 0.0 {
                    x509_mark(|| format!("list measure abort row={} raw={raw}", row.id));
                    self.preparation_key.set(None);
                    window.refresh();
                    return;
                }
                let height = (raw * scale).ceil() / scale;
                self.heights.borrow_mut().insert(
                    row.id.clone(),
                    RiseChatNativeListMeasuredHeight { version, width: preparation.width, height },
                );
                measured_now = true;
                height
            };
            preparation.measurements.insert(row.id, height);
            preparation.next_row += 1;
            if !preparation.first_screen_sent && index >= preparation.first_screen_start {
                preparation.first_screen_filled += height;
                if preparation.first_screen_filled >= preparation.first_screen_height
                    && index < preparation.source.len() - 1
                {
                    preparation.first_screen_sent = true;
                    self.install_first_screen(&preparation, index, window, cx);
                }
            }
            if measured_now
                && preparation.next_row < preparation.source.len()
                && RiseClock::monotonic().saturating_sub(turn_started) >= budget
            {
                *self.preparation.borrow_mut() = Some(preparation);
                window.request_animation_frame();
                return;
            }
        }
        let Some(this) = self.this.upgrade() else {
            return;
        };
        window.defer(cx, move |window, cx| this.commit(preparation, window, cx));
    }

    fn install_first_screen(
        &self,
        preparation: &RiseChatNativeListPreparation,
        index: usize,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(this) = self.this.upgrade() else {
            return;
        };
        let token = preparation.token;
        let width = preparation.width;
        let value = preparation.value.clone();
        let source = preparation.source[..=index].to_vec();
        let measurements = preparation.measurements.clone();
        let footer_height = preparation.footer_height;
        let minimum_content_height = preparation.minimum_content_height;
        window.defer(cx, move |window, cx| {
            if this.is_stopped.get()
                || token != this.generation.get()
                || this.restored.get()
                || !this.applied_ids.borrow().is_empty()
                || (f32::from(this.list.read(cx).bounds().size.width) - width).abs() > 0.5
            {
                return;
            }
            this.install(value, source, measurements, width, footer_height, minimum_content_height, window, cx);
        });
    }

    fn commit(&self, preparation: RiseChatNativeListPreparation, window: &mut Window, cx: &mut App) {
        if self.is_stopped.get() || preparation.token != self.generation.get() {
            x509_mark(|| format!("list commit dropped token={} current={}", preparation.token, self.generation.get()));
            return;
        }
        let width = preparation.width;
        let RiseChatNativeListPreparation { value, source, footer_height, measurements, minimum_content_height, .. } =
            preparation;
        if (f32::from(self.list.read(cx).bounds().size.width) - width).abs() > 0.5 {
            x509_mark(|| "list commit width changed".to_owned());
            self.preparation_key.set(None);
            window.refresh();
            return;
        }
        self.install(value, source, measurements, width, footer_height, minimum_content_height, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn install(
        &self,
        value: Rc<RiseChatNativeList>,
        source: Vec<RiseChatNativeListRow>,
        measurements: HashMap<String, f32>,
        width: f32,
        footer_height: f32,
        minimum_content_height: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        let payload_height: f32 = measurements.values().sum();
        let footer_height = footer_height.max(minimum_content_height - payload_height);
        let footer_id = FOOTER_ID.to_owned();
        let mut ids: Vec<String> = source.iter().map(|row| row.id.clone()).collect();
        if footer_height > 0.0 {
            ids.push(footer_id.clone());
        }
        let versions: HashMap<String, i64> = source.iter().map(|row| (row.id.clone(), row.version)).collect();
        let width_changed = self.last_width.get() != width;
        let mut updated: HashSet<String> = {
            let applied_versions = self.applied_versions.borrow();
            source
                .iter()
                .filter(|row| width_changed || applied_versions.get(&row.id) != Some(&row.version))
                .map(|row| row.id.clone())
                .collect()
        };
        updated.insert(footer_id.clone());
        x509_mark(|| {
            let head = source.first().map(|row| row.id.as_str()).unwrap_or_default();
            format!(
                "list install rows={} head={head} updated={} width={width} last_width={} width_changed={width_changed}",
                source.len(),
                updated.len(),
                self.last_width.get()
            )
        });
        let is_restored = self.restored.get();
        let id_set: HashSet<String> = ids.iter().cloned().collect();
        let saved = if is_restored {
            None
        } else {
            value.bookmarks.as_ref().and_then(|bookmarks| bookmarks.restoration(&value.page_id, Some(&id_set)))
        };
        let anchor = if let Some((id, offset)) = saved {
            RiseManualListTransactionAnchor::Item { id, viewport_offset: value.top_inset + offset }
        } else if let (false, Some(first)) = (is_restored, ids.first()) {
            let resting_offset = value
                .bookmarks
                .as_ref()
                .and_then(|bookmarks| bookmarks.resting_offset(&value.page_id))
                .unwrap_or(value.initial_top_offset);
            RiseManualListTransactionAnchor::Item { id: first.clone(), viewport_offset: value.top_inset - resting_offset }
        } else if let Some(pinned) = self.pinned_top(&value, &ids, cx) {
            pinned
        } else {
            RiseManualListTransactionAnchor::PreserveVisible
        };
        let lands_reorder = {
            let applied = self.applied_ids.borrow();
            *applied != ids
                && ids.iter().collect::<HashSet<_>>() == applied.iter().collect::<HashSet<_>>()
                && value.control.as_ref().is_some_and(|control| control.lands_settling_order(&ids))
        };
        let edge_mutation = if lands_reorder {
            None
        } else {
            let applied = self.applied_ids.borrow();
            RiseManualListTransactionEdgeMutation::diff(&applied, &ids).filter(|mutation| {
                let applied_set: HashSet<&String> = applied.iter().collect();
                let suffix_start = ids.len() - mutation.inserted_suffix_count;
                !ids[..mutation.inserted_prefix_count]
                    .iter()
                    .chain(ids[suffix_start..].iter())
                    .any(|id| id != FOOTER_ID && applied_set.contains(id))
            })
        };
        let shift_control = value.control.clone();
        let content_by_id: HashMap<String, RiseChatNativeListRow> =
            source.into_iter().map(|row| (row.id.clone(), row)).collect();
        let row_activity = self.row_activity.clone();
        let provider: RiseManualListItemProvider = Rc::new(move |id: &str, _width: f32, _window: &mut Window, _cx: &mut App| {
            let row = content_by_id.get(id);
            let height = measurements.get(id).copied();
            let (Some(row), Some(height)) = (row, height) else {
                return RiseManualListItem::new(
                    footer_height,
                    FOOTER_REUSE_IDENTIFIER.to_owned(),
                    0,
                    RiseManualListItemMountPolicy::Immediate,
                    None,
                    None,
                    None,
                    None,
                    Rc::new(|old: Option<AnyView>, _window: &mut Window, cx: &mut App| -> AnyView {
                        old.unwrap_or_else(|| cx.new(|_| EmptyView).into())
                    }),
                );
            };
            let row = row.clone();
            let row_activity = row_activity.clone();
            let shift = shift_control.as_ref().map_or(0.0, |control| control.row_shift(id));
            let version = row.version;
            RiseManualListItem::new(
                height,
                ROW_REUSE_IDENTIFIER.to_owned(),
                version,
                RiseManualListItemMountPolicy::DeferredDuringFastScroll,
                None,
                None,
                None,
                None,
                Rc::new(move |old: Option<AnyView>, _window: &mut Window, cx: &mut App| -> AnyView {
                    let host = old
                        .and_then(|old| old.downcast::<RiseChatNativeRowHost>().ok())
                        .unwrap_or_else(|| cx.new(RiseChatNativeRowHost::new));
                    let row_id = SharedString::from(row.id.clone());
                    let content = row.content.clone();
                    let row_activity = row_activity.clone();
                    host.update(cx, |host, cx| {
                        host.configure(row_id, version, content, row_activity, cx);
                        host.set_shift(shift, None, cx);
                    });
                    host.into()
                }),
            )
        });
        let transaction = RiseManualListTransaction::new(
            ids.clone(),
            anchor,
            Some(width),
            Some(updated),
            None,
            None,
            edge_mutation,
            true,
            HashSet::from([footer_id]),
            provider,
        );
        let owner = self.this.clone();
        let discard_owner = self.this.clone();
        self.list.update(cx, |list, cx| {
            list.enqueue(
                transaction,
                Some(Box::new(move |window: &mut Window, cx: &mut App| {
                    let Some(this) = owner.upgrade() else {
                        return;
                    };
                    *this.applied_ids.borrow_mut() = ids;
                    *this.applied_versions.borrow_mut() = versions;
                    this.last_width.set(width);
                    this.restored.set(true);
                    let control = this.current_value().and_then(|value| value.control.clone());
                    if let Some(control) = &control {
                        control.first_rows(cx).update(cx, |first_rows, cx| first_rows.mark_installed(cx));
                    }
                    let collapse_coordinator = this.collapse_coordinator.borrow().clone();
                    if let Some(coordinator) = collapse_coordinator {
                        coordinator.update(cx, |coordinator, cx| coordinator.content_top_did_change(cx));
                    }
                    if let Some(control) = &control {
                        let applied = this.applied_ids.borrow().clone();
                        control.transaction_applied(&applied, cx);
                    }
                    window.defer(cx, move |window, cx| {
                        let Some(on_visible) = this.current_value().and_then(|value| value.on_visible.clone()) else {
                            return;
                        };
                        let visible_ids = this.list.read(cx).visible_ids().to_vec();
                        on_visible(&visible_ids, window, cx);
                    });
                })),
                Some(Box::new(move |_window: &mut Window, cx: &mut App| {
                    if let Some(this) = discard_owner.upgrade() {
                        let list_width = f32::from(this.list.read(cx).bounds().size.width);
                        if (list_width - width).abs() > 0.5 {
                            this.rows_missed_width(list_width, cx);
                        }
                    }
                })),
                window,
                cx,
            );
        });
    }

    /// rows were laid out at another width than the list has now
    /// next prepare measures them again at width
    fn rows_missed_width(&self, width: f32, cx: &mut App) {
        if self.is_stopped.get() || width <= 1.0 {
            return;
        }
        if self.preparation_key.get().is_some_and(|key| (key.width - width).abs() <= 0.5)
            && (self.last_width.get() - width).abs() > 0.5
        {
            self.preparation_key.set(None);
        }
        if (self.last_width.get() - width).abs() > 0.5
            && let Some(host) = self.host_view.get()
        {
            cx.notify(host);
        }
    }
}

/// reusable host of one chat row, the row element under the activity carrier
pub struct RiseChatNativeRowHost {
    hosted: Option<(SharedString, i64, RiseChatNativeListRowElement, Entity<ChatListRowActivity>)>,
    content: Entity<RiseChatNativeRowContent>,
    shift: Animated<f32>,
    lifted: bool,
}

impl RiseChatNativeRowHost {
    fn new(cx: &mut Context<Self>) -> Self {
        RiseManualListConformances::register_reusable_view::<Self>(cx);
        RiseManualListConformances::register_moved_replay_row::<Self>(cx);
        let content = cx.new(|_| RiseChatNativeRowContent { content: None, configured: None, last_env: 0 });
        Self { hosted: None, content, shift: Animated::new(0.0), lifted: false }
    }

    fn configure(
        &mut self,
        id: SharedString,
        version: i64,
        content: RiseChatNativeListRowElement,
        row_activity: Entity<ChatListRowActivity>,
        cx: &mut Context<Self>,
    ) {
        let unchanged = self.hosted.as_ref().is_some_and(|(hosted_id, hosted_version, hosted_content, hosted_activity)| {
            *hosted_id == id
                && (Rc::ptr_eq(hosted_content, &content) || *hosted_version == version)
                && hosted_activity == &row_activity
        });
        if unchanged {
            return;
        }
        if let Some((hosted_id, hosted_version, hosted_content, hosted_activity)) = self.hosted.as_ref() {
            tracing::warn!(
                target: "riseonly::X1503",
                row = %id,
                same_id = *hosted_id == id,
                same_version = *hosted_version == version,
                same_closure = Rc::ptr_eq(hosted_content, &content),
                same_activity = hosted_activity == &row_activity,
                "chat row reconfigured"
            );
        }
        self.hosted = Some((id.clone(), version, content.clone(), row_activity));
        self.content.update(cx, |row, cx| {
            row.content = Some(content);
            row.configured = Some(id);
            cx.notify();
        });
        cx.notify();
    }

    pub fn set_shift(&mut self, y: f32, spec: Option<MotionSpec>, cx: &mut Context<Self>) {
        if *self.shift.target() == y {
            return;
        }
        self.shift.set_in(y, spec, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn hosts_row(&self, id: &str) -> bool {
        self.hosted.as_ref().is_some_and(|(hosted_id, ..)| hosted_id.as_str() == id)
    }

    fn set_lifted(&mut self, lifted: bool, cx: &mut Context<Self>) {
        if self.lifted == lifted {
            return;
        }
        self.lifted = lifted;
        cx.notify();
    }
}

impl Render for RiseChatNativeRowHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((id, _, content, row_activity)) = self.hosted.clone() else {
            tracing::warn!(target: "riseonly::X1693", host = %cx.entity_id(), "chat row host drawn blank");
            return div().into_any_element();
        };
        let shift = self.shift.frame_value(RiseClock::monotonic(), window);
        let row = div().id(ElementId::Name(id)).relative().top(px(shift)).size_full();
        let row = if self.lifted {
            deferred(row.child(content(window, cx))).with_priority(1).into_any_element()
        } else {
            row.child(AnyView::from(self.content.clone()).cached(StyleRefinement::default().size_full()))
                .into_any_element()
        };
        env_scope::<ChatListRowActivityKey>(Some(row_activity)).child(row).into_any_element()
    }
}

impl RiseManualListReusableView for RiseChatNativeRowHost {
    fn prepare_for_manual_list_reuse(&mut self, cx: &mut Context<Self>) {
        self.hosted = None;
        self.content.update(cx, |row, cx| {
            row.content = None;
            row.configured = Some(SharedString::from("reuse"));
            cx.notify();
        });
        self.shift.jump(0.0);
        self.lifted = false;
        cx.notify();
    }
}

/// content of one chat row, drawn by its host as a cached view
pub struct RiseChatNativeRowContent {
    content: Option<RiseChatNativeListRowElement>,
    configured: Option<SharedString>,
    last_env: u64,
}

impl Render for RiseChatNativeRowContent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let env = window.env_fingerprint();
        tracing::warn!(
            target: "riseonly::X1503",
            configured = ?self.configured.take(),
            env_changed = env != self.last_env,
            "chat row content render"
        );
        self.last_env = env;
        match self.content.clone() {
            Some(content) => content(window, cx),
            None => {
                tracing::warn!(target: "riseonly::X1693", content = %cx.entity_id(), "chat row content drawn blank");
                div().into_any_element()
            }
        }
    }
}

fn x509_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X253", "{}", stage());
}
