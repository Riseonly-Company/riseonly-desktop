use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::FutureExt;
use gpui::prelude::*;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use gpui::{
    AnyView, AnyWindowHandle, App, AsyncApp, Bounds, Context, EmptyView, Entity, EntityId, FocusHandle, Hsla, KeyDownEvent, Pixels,
    StyleRefinement, Subscription, Task, Window, canvas, div, px,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};

use crate::app::router::main_tab_chrome::MainTabPageHostKey;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RISE_ENGINE_CONNECTION_READY;
use crate::core::shared_element_transition::kernel::shared_element_types::ProfilePostListWindowPolicy;
use crate::core::shared_element_transition::runtime::shared_element_containers::SharedElementDestinationContainer;
use crate::core::shared_element_transition::runtime::shared_element_host_probe::SharedElementHostProbe;
use crate::core::shared_element_transition::runtime::shared_element_transition_coordinator::SharedElementTransitionCoordinator;
use crate::core::ui::list_engine::rise_manual_list_integration::RiseManualListInstallCost;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListScrollMetrics,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::{
    RiseManualListMountEnvelopePolicy, RiseManualListView,
};
use crate::modules::post::engine::rise_post_engine_models::RisePostPageStatus;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_commit_reconciler::{
    PostListCommitReconciler, PostListCommitReconcilerCommit,
};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_leaf_mount_queue::PostListLeafMountQueue;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_observation::{
    PostListObservation, PostListObservedRow,
};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_pagination::{
    PostListLoadMoreFooter, PostListPagination, PostListPaginationPolicy,
};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_playback_focus::PostListPlaybackFocus;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_pull_down_dismiss::PostListPullDownDismiss;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_refresh_control::{
    PostListRefreshControl, PostListRefreshIndicatorView,
};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_reveal_controller::{
    PostListRevealController, PostListRevealControllerDecision,
};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_view_tracker::PostListViewTracker;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_warmups::PostListWarmups;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_row_action_router::PostRowActionRouter;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostCaptionExpansion, PostListCachePurge, PostListCachePurgeRegistration, PostListFocusReporting, PostListHostedEnvironment, PostListReusableRow,
    PostListReuseIdentifier, PostListWindowOwner, PostRowActionRouting, PostRowAnchorRegistering, PostRowBodyKind,
    PostRowLayout, PostRowMainContext, PostRowNativeKind, PostRowRoute, PostRowStoryRingState, PreparedPostRow,
};
use crate::modules::post::pages::posts::post_detail::native_list::fallback::post_hosted_row_measurer::{
    PostHostedRowMeasurePolicy, PostHostedRowMeasurer, PostHostedRowMeasurerResolution,
};
use crate::modules::post::pages::posts::post_detail::native_list::fallback::post_hosted_row_view::PostHostedRowView;
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_premium_badge_resolver::PostRowPremiumBadgeResolver;
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_preparer::{
    PostRowPrepareBatch, PostRowPrepareEnvironment, PostRowPrepareInput, PostRowPreparePriority, PostRowPreparation,
    PostRowPreparer,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_list_configuration::{
    PostListConfiguration, PostListConfigurationWindow, PostListHeader,
};
use crate::modules::post::pages::posts::post_detail::native_list::post_list_input::{PostListInput, PostListRows};
use crate::modules::post::pages::posts::post_detail::native_list::post_list_performance_trace::{
    PostListPerformanceTrace, PostListPerformanceTraceSpan, PostListTraceEvent, PostListTraceSpan,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_list_header_row_view::PostListHeaderRowView;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_native_row_view::{
    PostNativeRowView, PostRowMediaHosting,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_shared_element_anchors::PostRowAnchorRegistration;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_header_view::PostRowPremiumDiamond;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_media_view::{
    PostRowMediaView, PostRowViewerLiftHub,
};
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_capture::PostListSharedElementScrollView;
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_destination::PostListSharedElementDestination;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum PostListControllerPrepareSlot {
    Refresh,
    Suffix,
    Above,
}

impl PostListControllerPrepareSlot {
    fn name(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::Suffix => "suffix",
            Self::Above => "above",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PostListControllerStoreReason {
    Prepared,
    Aspect,
    Measured,
    Spec,
}

impl PostListControllerStoreReason {
    fn name(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Aspect => "aspect",
            Self::Measured => "measured",
            Self::Spec => "spec",
        }
    }
}

struct PostListControllerInFlight {
    token: u64,
    _task: Task<()>,
    span: Option<PostListPerformanceTraceSpan>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PostListControllerEngineStamp {
    serial: u64,
    row_generation: u64,
    global_generation: u64,
}

#[derive(Clone, Copy)]
struct PostListControllerEngineVersion {
    stamp: PostListControllerEngineStamp,
    version: i64,
}

fn trace_fields<const N: usize>(pairs: [(&str, String); N]) -> HashMap<String, String> {
    pairs.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

pub struct PostListController {
    this: Weak<Self>,
    pub session: u64,
    pub configuration: PostListConfiguration,

    view: Entity<PostListControllerView>,
    window_handle: AnyWindowHandle,
    list_view: Entity<RiseManualListView>,
    trace: Rc<PostListPerformanceTrace>,
    preparer: Rc<PostRowPreparer>,
    measurer: Rc<PostHostedRowMeasurer>,
    reconciler: Rc<PostListCommitReconciler>,
    reveal: RefCell<PostListRevealController>,
    pagination: RefCell<PostListPagination>,
    load_more_footer: Entity<PostListLoadMoreFooter>,
    pagination_retry_work: RefCell<Option<Task<()>>>,
    connection_ready_observer: RefCell<Option<RiseSubscription>>,
    connection_ready_delivery: RefCell<Option<Task<()>>>,
    pull_down: RefCell<Option<PostListPullDownDismiss>>,
    refresh_control: Option<Entity<PostListRefreshControl>>,
    refresh_indicator: Option<Entity<PostListRefreshIndicatorView>>,
    playback_focus: Entity<PostListPlaybackFocus>,
    leaf_queue: Entity<PostListLeafMountQueue>,
    router: Rc<PostRowActionRouter>,
    tracker: Entity<PostListViewTracker>,
    warmups: Rc<PostListWarmups>,
    observation: Entity<PostListObservation>,

    input: RefCell<Option<Rc<PostListInput>>>,
    pending_input: RefCell<Option<Rc<PostListInput>>>,
    is_input_flush_scheduled: Cell<bool>,
    input_flush_token: Cell<u64>,

    rows: RefCell<Rc<PostListRows>>,
    warmup_posts: RefCell<Option<Rc<Vec<GetPostFeedResponse>>>>,
    lower: Cell<usize>,
    table: RefCell<HashMap<String, Arc<PreparedPostRow>>>,
    table_serials: RefCell<HashMap<String, u64>>,
    next_serial: Cell<u64>,
    engine_versions: RefCell<HashMap<String, PostListControllerEngineVersion>>,
    next_engine_version: Cell<i64>,
    row_context_generations: RefCell<HashMap<String, u64>>,
    global_context_generation: Cell<u64>,
    cached_prepared_end: Cell<Option<usize>>,

    environment: RefCell<Option<PostRowPrepareEnvironment>>,
    environment_generation: Cell<u64>,
    environment_needs_rebuild: Cell<bool>,
    row_context: RefCell<Option<Rc<PostRowMainContext>>>,

    stale_order: RefCell<Vec<String>>,
    stale_set: RefCell<HashSet<String>>,
    idle_only_stale: RefCell<HashSet<String>>,
    aspect_stale: RefCell<HashSet<String>>,
    in_flight: RefCell<HashMap<PostListControllerPrepareSlot, PostListControllerInFlight>>,
    last_pagination_diagnostic: RefCell<String>,
    next_prepare_token: Cell<u64>,

    did_start_first_commit: Cell<bool>,
    first_commit_started_at: Cell<f64>,
    first_row_id: RefCell<Option<String>>,
    did_record_first_viewport: Cell<bool>,
    last_metrics: Cell<Option<RiseManualListScrollMetrics>>,
    was_scrolling: Cell<bool>,
    is_evaluation_scheduled: Cell<bool>,
    evaluation_token: Cell<u64>,
    is_visible_flush_scheduled: Cell<bool>,
    visible_flush_token: Cell<u64>,
    did_warm_diamond: Cell<bool>,
    last_reveal_block: RefCell<Option<String>>,
    purge_registration: RefCell<Option<PostListCachePurgeRegistration>>,
    shared_element_scroll_view: RefCell<Option<Rc<PostListSharedElementScrollView>>>,
    is_active: Cell<bool>,
    presented_from_top: Cell<bool>,
    has_reported_anchor: Cell<bool>,
    last_reported_anchor_id: RefCell<Option<String>>,
    is_remounting_at_top: Cell<bool>,
    is_stopped: Cell<bool>,

    bounds: Cell<Bounds<Pixels>>,
    is_in_window: Cell<bool>,
    display_scale: Cell<f32>,
    background: Cell<Option<Hsla>>,
}

impl PostListController {
    const SEEDS_PER_TURN: i64 = 8;
    const FIRST_VIEWPORT_MAX_EXTRA_ROWS: usize = 6;
    const FIRST_VIEWPORT_SYNC_BUDGET_SECONDS: f64 = 0.024;

    pub fn new(configuration: PostListConfiguration, window: &mut Window, cx: &mut App) -> Rc<Self> {
        let session = configuration
            .destination
            .as_ref()
            .map_or_else(SharedElementTransitionCoordinator::next_session, |destination| destination.session);
        let is_feed = configuration.is_feed();
        let has_destination = configuration.destination.is_some();
        let viewport_height = window.viewport_size().height;
        let display_scale = window.scale_factor();
        let controller = Rc::new_cyclic(|this: &Weak<Self>| {
            let list_view = cx.new(|cx| RiseManualListView::new(window, cx));
            let window_handle = list_view.read(cx).window_handle();
            let trace = Rc::new(PostListPerformanceTrace::new(session));
            let preparer = PostRowPreparer::new(session, cx);
            let measurer = PostHostedRowMeasurer::new(session, window_handle, cx);
            let router = {
                let row_owner = this.clone();
                let environment_owner = this.clone();
                PostRowActionRouter::new(
                    Rc::new(move |post_id: &str, cx: &mut App| {
                        row_owner.upgrade().and_then(|controller| controller.router_row(post_id, cx))
                    }),
                    Rc::new(move |_: &App| {
                        environment_owner.upgrade().and_then(|controller| controller.hosted_environment())
                    }),
                )
            };
            let reconciler = PostListCommitReconciler::new(&list_view, cx);
            let reveal = match &configuration.window {
                PostListConfigurationWindow::GridOrder { anchor_id, anchor_index } => {
                    PostListRevealController::new(anchor_id, *anchor_index)
                }
                PostListConfigurationWindow::Top { restore_id } => {
                    PostListRevealController::new_feed_restore_id(restore_id.clone())
                }
            };
            let pull_down = has_destination.then(|| PostListPullDownDismiss::new(list_view.downgrade()));
            let refresh_control = is_feed.then(|| cx.new(|cx| PostListRefreshControl::new(cx)));
            let refresh_indicator = refresh_control.as_ref().map(|control| control.read(cx).indicator_view.clone());
            let playback_focus =
                cx.new(|cx| PostListPlaybackFocus::new(session, Some(list_view.downgrade()), cx));
            let leaf_queue = cx.new(|cx| PostListLeafMountQueue::new(&list_view, Some(trace.clone()), cx));
            let tracker = {
                let owner = this.clone();
                let router_reference = Rc::downgrade(&router);
                let router_reference: Weak<dyn PostRowActionRouting> = router_reference;
                let surface = configuration.view_surface.clone();
                let opened_post_id = configuration.opened_post_id.clone();
                cx.new(|cx| {
                    PostListViewTracker::new(
                        &list_view,
                        router_reference,
                        surface,
                        opened_post_id,
                        Rc::new(move |presentation_id: &str, _: &App| {
                            owner.upgrade().and_then(|controller| controller.table.borrow().get(presentation_id).cloned())
                        }),
                        window,
                        cx,
                    )
                })
            };
            let warmups = {
                let posts_owner = this.clone();
                let environment_owner = this.clone();
                PostListWarmups::new(
                    preparer.clone(),
                    Some(measurer.hosted_height_lookup()),
                    Rc::new(move |_: &App| {
                        posts_owner.upgrade().map(|controller| controller.warmup_posts()).unwrap_or_default()
                    }),
                    Rc::new(move |_: &mut App| {
                        environment_owner.upgrade().and_then(|controller| controller.environment.borrow().clone())
                    }),
                )
            };
            let observation = {
                let owner = this.clone();
                cx.new(|cx| {
                    PostListObservation::new(
                        Rc::new(move |cx: &App| {
                            owner.upgrade().map(|controller| controller.observed_rows(cx)).unwrap_or_default()
                        }),
                        cx,
                    )
                })
            };
            let load_more_footer = cx.new(|_| PostListLoadMoreFooter::new());
            let view = cx.new(|cx| PostListControllerView {
                controller: this.clone(),
                focus_handle: cx.focus_handle(),
                tab_host_observer: None,
                left_with_tab: false,
            });
            Self {
                this: this.clone(),
                session,
                configuration,
                view,
                window_handle,
                list_view,
                trace,
                preparer,
                measurer,
                reconciler,
                reveal: RefCell::new(reveal),
                pagination: RefCell::new(PostListPagination::new()),
                load_more_footer,
                pagination_retry_work: RefCell::new(None),
                connection_ready_observer: RefCell::new(None),
                connection_ready_delivery: RefCell::new(None),
                pull_down: RefCell::new(pull_down),
                refresh_control,
                refresh_indicator,
                playback_focus,
                leaf_queue,
                router,
                tracker,
                warmups,
                observation,
                input: RefCell::new(None),
                pending_input: RefCell::new(None),
                is_input_flush_scheduled: Cell::new(false),
                input_flush_token: Cell::new(0),
                rows: RefCell::new(Rc::new(PostListRows::empty())),
                warmup_posts: RefCell::new(None),
                lower: Cell::new(0),
                table: RefCell::new(HashMap::new()),
                table_serials: RefCell::new(HashMap::new()),
                next_serial: Cell::new(0),
                engine_versions: RefCell::new(HashMap::new()),
                next_engine_version: Cell::new(0),
                row_context_generations: RefCell::new(HashMap::new()),
                global_context_generation: Cell::new(0),
                cached_prepared_end: Cell::new(None),
                environment: RefCell::new(None),
                environment_generation: Cell::new(0),
                environment_needs_rebuild: Cell::new(false),
                row_context: RefCell::new(None),
                stale_order: RefCell::new(Vec::new()),
                stale_set: RefCell::new(HashSet::new()),
                idle_only_stale: RefCell::new(HashSet::new()),
                aspect_stale: RefCell::new(HashSet::new()),
                in_flight: RefCell::new(HashMap::new()),
                last_pagination_diagnostic: RefCell::new(String::new()),
                next_prepare_token: Cell::new(0),
                did_start_first_commit: Cell::new(false),
                first_commit_started_at: Cell::new(0.0),
                first_row_id: RefCell::new(None),
                did_record_first_viewport: Cell::new(false),
                last_metrics: Cell::new(None),
                was_scrolling: Cell::new(false),
                is_evaluation_scheduled: Cell::new(false),
                evaluation_token: Cell::new(0),
                is_visible_flush_scheduled: Cell::new(false),
                visible_flush_token: Cell::new(0),
                did_warm_diamond: Cell::new(false),
                last_reveal_block: RefCell::new(None),
                purge_registration: RefCell::new(None),
                shared_element_scroll_view: RefCell::new(None),
                is_active: Cell::new(true),
                presented_from_top: Cell::new(false),
                has_reported_anchor: Cell::new(false),
                last_reported_anchor_id: RefCell::new(None),
                is_remounting_at_top: Cell::new(false),
                is_stopped: Cell::new(false),
                bounds: Cell::new(Bounds::default()),
                is_in_window: Cell::new(false),
                display_scale: Cell::new(display_scale),
                background: Cell::new(None),
            }
        });

        let is_hold_menu_active = controller.observation.read(cx).is_hold_menu_active();
        controller.list_view.update(cx, |list, cx| {
            list.set_inter_item_spacing(px(PostRowLayout::INTER_ITEM_SPACING), window, cx);
            list.set_reuse_pool_limit(16);
            list.set_prevents_bottom_overscroll(false, cx);
            list.rests_short_content_at_top = true;
            list.set_cancels_control_touches(true);
            list.set_preload_distance(
                RiseManualListMountEnvelopePolicy::default_preload_distance(viewport_height),
                window,
                cx,
            );
            list.set_is_scroll_enabled(!is_hold_menu_active, cx);
        });
        controller.load_more_footer.update(cx, |footer, _| {
            let owner = controller.this.clone();
            footer.on_retry = Some(Rc::new(move |_: &mut Window, cx: &mut App| {
                if let Some(controller) = owner.upgrade() {
                    controller.connection_did_return(cx);
                }
            }));
        });

        if let Some(destination) = controller.destination() {
            controller.attach_destination(&destination, window);
        }

        controller.install_connection_ready_observer(cx);
        controller.wire_engine(cx);
        controller.wire_reconciler();
        controller.wire_measurer();
        controller.wire_behaviours(cx);
        if let Some(pull_down) = controller.pull_down.borrow_mut().as_mut() {
            pull_down.install(cx);
        }
        controller.install_purge_registration(cx);
        controller
    }

    /// profile grid only
    pub fn destination(&self) -> Option<Rc<PostListSharedElementDestination>> {
        self.configuration.destination.clone()
    }

    /// what the bridge draws, controller lives as long as the bridge holds it
    pub fn view(&self) -> AnyView {
        self.view.clone().into()
    }

    fn window_owner(&self) -> Option<Rc<dyn PostListWindowOwner>> {
        self.this.upgrade().map(|controller| controller as Rc<dyn PostListWindowOwner>)
    }

    fn attach_destination(&self, destination: &Rc<PostListSharedElementDestination>, window: &Window) {
        if let Some(owner) = self.window_owner() {
            destination.attach_window_owner(Rc::downgrade(&owner));
        }
        let mut slot = self.shared_element_scroll_view.borrow_mut();
        let scroll_view = slot.get_or_insert_with(|| {
            let this = self.this.clone();
            Rc::new(PostListSharedElementScrollView::new(
                window.window_handle(),
                window.scale_factor(),
                move || this.upgrade().map_or_else(Bounds::default, |controller| controller.bounds.get()),
            ))
        });
        scroll_view.update_scale(window.scale_factor());
        destination.attach_scroll_view(scroll_view.clone());
    }

    fn current_input(&self) -> Option<Rc<PostListInput>> {
        self.input.borrow().clone()
    }

    fn current_rows(&self) -> Rc<PostListRows> {
        self.rows.borrow().clone()
    }

    fn warmup_posts(&self) -> Rc<Vec<GetPostFeedResponse>> {
        if let Some(posts) = self.warmup_posts.borrow().as_ref() {
            return posts.clone();
        }
        let posts = Rc::new(self.rows.borrow().items.clone());
        *self.warmup_posts.borrow_mut() = Some(posts.clone());
        posts
    }

    fn row(&self, presentation_id: &str) -> Option<Arc<PreparedPostRow>> {
        self.table.borrow().get(presentation_id).cloned()
    }

    fn has_row(&self, presentation_id: &str) -> bool {
        self.table.borrow().contains_key(presentation_id)
    }

    fn pull_down_is_dismissing(&self) -> bool {
        self.pull_down.borrow().as_ref().is_some_and(PostListPullDownDismiss::is_dismissing)
    }

    /// runs on the next main turn with the window this list lives in
    fn on_next_main_turn(&self, cx: &mut App, work: impl FnOnce(Rc<Self>, &mut Window, &mut App) + 'static) {
        let owner = self.this.clone();
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                if let Some(controller) = owner.upgrade() {
                    work(controller, window, cx);
                }
            });
        });
    }

    fn install_connection_ready_observer(&self, cx: &mut App) {
        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let subscription = RiseAppEvents::default_center().observe(RISE_ENGINE_CONNECTION_READY, move |_| {
            let _ = sender.unbounded_send(());
        });
        let owner = self.this.clone();
        let delivery = cx.spawn(async move |cx: &mut AsyncApp| {
            while receiver.next().await.is_some() {
                let Some(controller) = owner.upgrade() else {
                    return;
                };
                cx.update(|cx| controller.connection_did_return(cx));
            }
        });
        *self.connection_ready_observer.borrow_mut() = Some(subscription);
        *self.connection_ready_delivery.borrow_mut() = Some(delivery);
    }

    fn install_purge_registration(&self, cx: &mut App) {
        let owner = self.this.clone();
        let registration = PostListCachePurge::register(
            move |cx: &mut App| {
                if let Some(controller) = owner.upgrade() {
                    controller.caches_did_purge(cx);
                }
            },
            cx,
        );
        *self.purge_registration.borrow_mut() = Some(registration);
    }

    /// first call applies inline, later ones apply on the next main turn deduped by page revision
    /// anchor and environment identity
    /// never prepares or measures
    pub fn submit(&self, new_input: PostListInput, window: &mut Window, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let new_input = Rc::new(new_input);
        let Some(applied) = self.current_input() else {
            self.apply(new_input, window, cx);
            return;
        };
        let baseline = self.pending_input.borrow().clone().unwrap_or(applied);
        if baseline.is_equivalent(&new_input) {
            return;
        }
        *self.pending_input.borrow_mut() = Some(new_input);
        if self.is_input_flush_scheduled.get() {
            return;
        }
        self.is_input_flush_scheduled.set(true);
        let token = self.input_flush_token.get().wrapping_add(1);
        self.input_flush_token.set(token);
        self.on_next_main_turn(cx, move |controller, window, cx| {
            if controller.input_flush_token.get() != token {
                return;
            }
            controller.is_input_flush_scheduled.set(false);
            let next = controller.pending_input.borrow_mut().take();
            let Some(next) = next else {
                return;
            };
            controller.apply(next, window, cx);
        });
    }

    /// tears down engine and probe and pan and observers and trace and dwell sessions
    /// also focus tokens and leaves and prefetch and preparation
    pub fn stop(&self, window: Option<&mut Window>, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        self.is_stopped.set(true);
        self.evaluation_token.set(self.evaluation_token.get().wrapping_add(1));
        self.input_flush_token.set(self.input_flush_token.get().wrapping_add(1));
        self.visible_flush_token.set(self.visible_flush_token.get().wrapping_add(1));
        self.list_view.update(cx, |list, _| {
            list.on_visible_ids_changed = None;
            list.on_active_scroll_metrics_changed = None;
            list.on_scroll_metrics_changed = None;
            list.on_viewport_width_changed = None;
            list.on_main_tab_scroll_to_top = None;
        });
        self.report_visible_anchor(cx);
        self.reconciler.stop();
        self.leaf_queue.update(cx, |queue, cx| queue.stop(cx));
        if let Some(window) = window {
            self.list_view.update(cx, |list, cx| list.stop(window, cx));
        }
        if let Some(pull_down) = self.pull_down.borrow_mut().as_mut() {
            pull_down.stop(cx);
        }
        if let Some(refresh_control) = &self.refresh_control {
            refresh_control.update(cx, |control, cx| control.stop(cx));
        }
        self.cancel_preparation();
        self.preparer.invalidate();
        self.measurer.stop(cx);
        self.playback_focus.update(cx, |focus, cx| focus.stop(cx));
        self.tracker.update(cx, |tracker, cx| tracker.stop(cx));
        self.warmups.stop(cx);
        self.observation.update(cx, |observation, _| observation.stop());
        self.pagination.borrow_mut().stop();
        self.pagination_retry_work.borrow_mut().take();
        self.connection_ready_observer.borrow_mut().take();
        self.connection_ready_delivery.borrow_mut().take();
        self.load_more_footer.update(cx, |footer, cx| footer.stop(cx));
        self.trace.stop_recording("stop");
        if let Some(destination) = self.destination() {
            cx.notify(self.view.entity_id());
            if let Some(owner) = self.window_owner() {
                destination.detach_window_owner(&Rc::downgrade(&owner));
            }
            if let Some(scroll_view) = self.shared_element_scroll_view.borrow().as_ref() {
                destination.detach_scroll_view(scroll_view);
            }
        }
        let registration = self.purge_registration.borrow_mut().take();
        if let Some(registration) = registration {
            PostListCachePurge::unregister(registration);
        }
    }

    fn layout_subviews(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let height_changed = (self.bounds.get().size.height - bounds.size.height).abs() > px(0.5);
        self.bounds.set(bounds);
        self.display_scale.set(window.scale_factor());
        let viewport = gpui::size(bounds.size.width, window.viewport_size().height);
        if PostMediaBox::set_viewport(viewport) && self.environment.borrow().is_some() {
            self.environment_needs_rebuild.set(true);
            let _ = self.ensure_environment(cx);
        } else if self.environment_needs_rebuild.get()
            && bounds.size.width > px(1.0)
            && self.environment.borrow().is_some()
        {
            let _ = self.ensure_environment(cx);
            self.schedule_evaluation(cx);
        }
        if !self.did_start_first_commit.get() {
            self.start_first_commit_if_possible(window, cx);
        }
        if let Some(refresh_control) = &self.refresh_control {
            let width = bounds.size.width.as_f32();
            refresh_control.update(cx, |control, cx| control.layout_indicator(width, cx));
        }
        if height_changed && bounds.size.height > px(0.0) {
            let preload = RiseManualListMountEnvelopePolicy::default_preload_distance(bounds.size.height);
            self.list_view.update(cx, |list, cx| {
                if (list.preload_distance() - preload).abs() > px(0.5) {
                    list.set_preload_distance(preload, window, cx);
                }
            });
        }
    }

    pub fn did_move_to_window(&self, is_in_window: bool, window: &mut Window, cx: &mut App) {
        if self.is_stopped.get() || self.is_in_window.get() == is_in_window {
            return;
        }
        self.is_in_window.set(is_in_window);
        self.list_view.update(cx, |list, cx| list.did_move_to_window(is_in_window, window, cx));
        if is_in_window {
            self.host_did_appear(window, cx);
        } else {
            self.host_did_disappear(cx);
        }
    }

    fn window_did_close(&self, cx: &mut App) {
        if self.is_stopped.get() || !self.is_in_window.get() {
            return;
        }
        self.is_in_window.set(false);
        self.host_did_disappear(cx);
    }

    fn wire_engine(&self, cx: &mut App) {
        let visible_owner = self.this.clone();
        let active_metrics_owner = self.this.clone();
        let metrics_owner = self.this.clone();
        let width_owner = self.this.clone();
        let jump_owner = self.this.clone();
        self.list_view.update(cx, |list, _| {
            list.on_visible_ids_changed = Some(Rc::new(move |ids: &[String], window: &mut Window, cx: &mut App| {
                let owner = visible_owner.clone();
                let ids = ids.to_vec();
                window.defer(cx, move |_, cx| {
                    if let Some(controller) = owner.upgrade() {
                        controller.visible_ids_did_change(&ids, cx);
                    }
                });
            }));
            list.on_active_scroll_metrics_changed = Some(Rc::new(
                move |metrics: &RiseManualListScrollMetrics, window: &mut Window, cx: &mut App| {
                    let owner = active_metrics_owner.clone();
                    let metrics = *metrics;
                    window.defer(cx, move |window, cx| {
                        if let Some(controller) = owner.upgrade() {
                            controller.scroll_metrics_did_change(metrics, window, cx);
                        }
                    });
                },
            ));
            list.on_scroll_metrics_changed = Some(Rc::new(
                move |metrics: &RiseManualListScrollMetrics, window: &mut Window, cx: &mut App| {
                    let owner = metrics_owner.clone();
                    let metrics = *metrics;
                    window.defer(cx, move |window, cx| {
                        if let Some(controller) = owner.upgrade() {
                            controller.scroll_metrics_did_change(metrics, window, cx);
                        }
                    });
                },
            ));
            list.on_viewport_width_changed = Some(Rc::new(move |_: Pixels, window: &mut Window, cx: &mut App| {
                let owner = width_owner.clone();
                window.defer(cx, move |_, cx| {
                    if let Some(controller) = owner.upgrade() {
                        controller.viewport_width_did_change(cx);
                    }
                });
            }));
            let wheel_owner = jump_owner.clone();
            list.on_main_tab_scroll_to_top =
                Some(Rc::new(move |refreshes_feed: bool, window: &mut Window, cx: &mut App| {
                    let Some(controller) = jump_owner.upgrade() else {
                        return false;
                    };
                    if refreshes_feed && controller.refresh_feed_from_tab(window, cx) {
                        return true;
                    }
                    controller.jump_to_feed_top_by_remount(cx)
                }));
            list.on_wheel_past_top = Some(Rc::new(move |push: Pixels, window: &mut Window, cx: &mut App| {
                let Some(controller) = wheel_owner.upgrade() else {
                    return;
                };
                if controller.lower.get() != 0 {
                    controller.reveal.borrow_mut().reader_pushed_above_window_top();
                    controller.schedule_evaluation(cx);
                    return;
                }
                if let Some(refresh_control) = &controller.refresh_control {
                    refresh_control.update(cx, |control, cx| control.wheel_push_past_top(push.as_f32(), window, cx));
                }
            }));
        });
    }

    fn wire_reconciler(&self) {
        let owner = self.this.clone();
        *self.reconciler.provider.borrow_mut() =
            Rc::new(move |id: &str, _: f32, _: &mut Window, cx: &mut App| match owner.upgrade() {
                Some(controller) => controller.item(id, cx),
                None => Self::missing_item(),
            });
        let owner = self.this.clone();
        *self.reconciler.desired_ids.borrow_mut() = Rc::new(move |_: &App| {
            owner.upgrade().map(|controller| controller.desired_ids()).unwrap_or_default()
        });
        let owner = self.this.clone();
        *self.reconciler.expected_width.borrow_mut() = Rc::new(move |_: &App| {
            let controller = owner.upgrade()?;
            let width = controller.environment.borrow().as_ref().map(|environment| environment.spec.width);
            width
        });
        let owner = self.this.clone();
        *self.reconciler.can_commit.borrow_mut() = Rc::new(move |_: &App| {
            owner.upgrade().is_some_and(|controller| {
                !controller.is_stopped.get()
                    && controller.environment.borrow().is_some()
                    && controller.input.borrow().is_some()
            })
        });
        let owner = self.this.clone();
        *self.reconciler.on_applied.borrow_mut() = Some(Rc::new(
            move |commit: &PostListCommitReconcilerCommit, window: &mut Window, cx: &mut App| {
                let owner = owner.clone();
                let commit = commit.clone();
                window.defer(cx, move |_, cx| {
                    if let Some(controller) = owner.upgrade() {
                        controller.commit_did_apply(&commit, cx);
                    }
                });
            },
        ));
    }

    fn wire_measurer(&self) {
        let owner = self.this.clone();
        self.measurer.set_can_measure(move |cx: &mut App| {
            owner.upgrade().is_some_and(|controller| controller.can_measure_hosted_rows(cx))
        });
        let owner = self.this.clone();
        self.measurer.set_visible_ids(move |cx: &App| {
            owner.upgrade().map(|controller| controller.list_view.read(cx).visible_ids().to_vec()).unwrap_or_default()
        });
        let owner = self.this.clone();
        self.measurer.set_mounted_row(move |presentation_id: &str, cx: &App| {
            let controller = owner.upgrade()?;
            let mounted = controller.list_view.read(cx).mounted_view(presentation_id)?;
            mounted.downcast::<PostHostedRowView>().ok()
        });
        let owner = self.this.clone();
        self.measurer.set_hosted_environment(move |_: &App| {
            owner.upgrade().and_then(|controller| controller.hosted_environment())
        });
        let owner = self.this.clone();
        self.measurer.set_on_heights_resolved(Some(Rc::new(
            move |resolutions: &[PostHostedRowMeasurerResolution], cx: &mut App| {
                if let Some(controller) = owner.upgrade() {
                    controller.hosted_heights_did_resolve(resolutions, cx);
                }
            },
        )));
    }

    fn wire_behaviours(&self, cx: &mut App) {
        let owner = self.this.clone();
        self.preparer.set_on_aspects_resolved(Rc::new(move |ids: &HashSet<String>, cx: &mut App| {
            if let Some(controller) = owner.upgrade() {
                controller.aspects_did_resolve(ids, cx);
            }
        }));
        let owner = self.this.clone();
        self.leaf_queue.update(cx, |queue, _| {
            queue.needs_leaves = Some(Rc::new(move |presentation_id: &str, _: &App| {
                owner.upgrade().is_some_and(|controller| controller.needs_leaves(presentation_id))
            }));
        });
        let owner = self.this.clone();
        self.playback_focus.update(cx, |focus, _| {
            focus.on_focus_changed = Some(Rc::new(move |ids: &HashSet<String>, cx: &mut App| {
                if let Some(controller) = owner.upgrade() {
                    controller.focus_did_change(ids, cx);
                }
            }));
        });
        let spec_owner = self.this.clone();
        let rows_owner = self.this.clone();
        let muted_owner = self.this.clone();
        let hold_owner = self.this.clone();
        let premium_owner = self.this.clone();
        let caption_owner = self.this.clone();
        self.router.set_on_caption_expansion_toggled(Some(Rc::new(move |presentation_id: &str, cx: &mut App| {
            if let Some(controller) = caption_owner.upgrade() {
                controller.caption_expansion_did_change(presentation_id, cx);
            }
        })));
        self.observation.update(cx, |observation, _| {
            observation.on_spec_changed = Some(Rc::new(move |cx: &mut App| {
                if let Some(controller) = spec_owner.upgrade() {
                    controller.spec_did_change(cx);
                }
            }));
            observation.on_rows_changed = Some(Rc::new(move |ids: &HashSet<String>, cx: &mut App| {
                if let Some(controller) = rows_owner.upgrade() {
                    controller.context_did_change(ids, cx);
                }
            }));
            observation.on_audio_muted_changed = Some(Rc::new(move |_: bool, cx: &mut App| {
                if let Some(controller) = muted_owner.upgrade() {
                    controller.global_context_did_change(cx);
                }
            }));
            observation.on_hold_menu_active_changed = Some(Rc::new(move |is_active: bool, cx: &mut App| {
                if let Some(controller) = hold_owner.upgrade() {
                    controller.list_view.update(cx, |list, cx| list.set_is_scroll_enabled(!is_active, cx));
                }
            }));
            observation.on_premium_badges_may_have_changed =
                Some(Rc::new(move |ids: &HashSet<String>, cx: &mut App| {
                    if let Some(controller) = premium_owner.upgrade() {
                        controller.premium_badges_may_have_changed(ids, cx);
                    }
                }));
        });
        if let Some(pull_down) = self.pull_down.borrow().as_ref() {
            let owner = self.this.clone();
            pull_down.set_allows_begin(move |_: &App| {
                owner.upgrade().is_some_and(|controller| {
                    let lower = controller.lower.get().max(controller.presented_window_lower());
                    let allows = controller.reveal.borrow().allows_pull_down_dismiss(lower);
                    allows
                })
            });
            let owner = self.this.clone();
            pull_down.set_navigation_controller(move |_: &App| {
                let controller = owner.upgrade()?;
                let input = controller.current_input()?;
                Some(input.navigation_store.clone())
            });
        }
        if let Some(refresh_control) = &self.refresh_control {
            let refresh_owner = self.this.clone();
            let inset_owner = self.this.clone();
            let landing_owner = self.this.clone();
            refresh_control.update(cx, |control, _| {
                control.on_refresh = Some(Rc::new(move |cx: AsyncApp| {
                    let refresh = refresh_owner
                        .upgrade()
                        .and_then(|controller| controller.current_input())
                        .and_then(|input| input.refresh.clone());
                    async move {
                        if let Some(refresh) = refresh {
                            let refreshing = cx.update(|cx| refresh(cx));
                            refreshing.await;
                        }
                    }
                    .boxed_local()
                }));
                control.on_top_inset_change = Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                    if let Some(controller) = inset_owner.upgrade() {
                        controller.apply_viewport_insets(window, cx);
                    }
                }));
                control.on_refresh_end_at_top = Some(Rc::new(move |window: &mut Window, cx: &mut App| {
                    if let Some(controller) = landing_owner.upgrade() {
                        controller.land_at_top_after_refresh(window, cx);
                    }
                }));
            });
        }
        let owner = self.this.clone();
        *self.reconciler.prefers_top_anchor.borrow_mut() = Rc::new(move |_: &App| {
            owner.upgrade().is_some_and(|controller| {
                let reveal = controller.reveal.borrow();
                reveal.restores_departed_top()
                    || (controller.configuration.is_feed()
                        && reveal.is_pinned_to_top()
                        && controller.presented_from_top.get())
            })
        });
        let returning_owner = self.this.clone();
        *self.reconciler.returning_row.borrow_mut() = Rc::new(move |_: &App| {
            let controller = returning_owner.upgrade()?;
            let rows = controller.current_rows();
            let reveal = controller.reveal.borrow();
            let index = rows.first_index_by_id.get(reveal.returning_departed_id()?).copied()?;
            Some((rows.presentation_ids.get(index)?.clone(), rows.presentation_ids.get(index + 1)?.clone()))
        });
    }

    fn apply(&self, new_input: Rc<PostListInput>, window: &mut Window, cx: &mut App) {
        let previous = self.input.borrow_mut().replace(new_input.clone());
        self.pagination.borrow_mut().page_did_change(&new_input.page);
        self.load_more_footer.update(cx, |footer, cx| {
            footer.page_did_change(&new_input.page, cx);
        });
        self.schedule_pagination_retry(cx);
        let is_waiting = self.is_waiting_pagination_retry();
        self.load_more_footer.update(cx, |footer, cx| {
            footer.set_waiting_retry(is_waiting, cx);
        });
        self.update_refresh_enabled(cx);
        self.apply_viewport_insets(window, cx);
        if let Some(previous) = &previous
            && !previous.has_same_hosted_identity(&new_input)
        {
            self.global_context_did_change(cx);
        }
        let rows_changed = previous.as_ref().is_none_or(|previous| {
            previous.page.revision != new_input.page.revision
                || !PostListInput::shares_items(&previous.page.items, &new_input.page.items)
                || previous.anchor_id != new_input.anchor_id
                || previous.anchor_post != new_input.anchor_post
                || previous.scope != new_input.scope
        });
        if rows_changed {
            self.rebuild_rows(cx);
        }
        self.update_destination_layout();
        if previous.as_ref().map(|previous| previous.is_active) != Some(new_input.is_active) {
            self.activity_did_change(cx);
        }

        if let Some(previous) = &previous {
            let previous_anchor_id = previous.anchor_post.as_ref().map(|post| &post.id);
            let next_anchor_id = new_input.anchor_post.as_ref().map(|post| &post.id);
            if previous_anchor_id != next_anchor_id
                && let Some(anchor_post) = &new_input.anchor_post
                && let Some(consume_comment_focus) = &new_input.consume_comment_focus
            {
                consume_comment_focus(anchor_post, cx);
            }
            if previous.page.status != new_input.page.status && new_input.page.status == RisePostPageStatus::Idle {
                self.reactivate_origin_if_evicted(&new_input.page.status, cx);
            }
        } else {
            self.configure_destination();
        }

        if !self.did_start_first_commit.get() {
            let bounds = self.bounds.get();
            if bounds.size.width > px(1.0) {
                self.on_next_main_turn(cx, move |controller, window, cx| {
                    controller.layout_subviews(bounds, window, cx);
                });
            }
        }
        self.schedule_evaluation(cx);
    }

    fn rebuild_rows(&self, cx: &mut App) {
        let Some(input) = self.current_input() else {
            return;
        };
        let previous_rows = self.current_rows();
        let rows = Rc::new(PostListRows::make(&input.page.items, &input.anchor_id, input.anchor_post.as_ref()));
        *self.rows.borrow_mut() = rows.clone();
        *self.warmup_posts.borrow_mut() = None;
        self.cached_prepared_end.set(None);
        self.reveal.borrow_mut().rows_did_change(&rows);
        let previous_lower = self.lower.get();
        let lower = self.reveal.borrow().lower(&rows);
        self.lower.set(lower);
        if let Some(first_id) = previous_rows.ids.first()
            && rows.count() > previous_rows.count()
            && rows.ids.first() == Some(first_id)
        {
            let from = previous_rows.count();
            self.warmups.rows_did_append(from, lower, cx);
        }
        if lower != previous_lower {
            self.warmups.window_top_did_move(lower, cx);
        }
        let changed: Vec<String> = self
            .table
            .borrow()
            .iter()
            .filter(|(id, row)| {
                rows.index_by_presentation_id.get(id.as_str()).is_some_and(|index| rows.items[*index] != row.post)
            })
            .map(|(id, _)| id.clone())
            .collect();
        x251_mark(|| {
            let liked: Vec<String> = rows.items.iter().filter(|post| post.is_liked).map(|post| post.id.clone()).take(8).collect();
            format!("x251 rebuild_rows rows={} changed={:?} liked={liked:?}", rows.count(), changed)
        });
        for id in changed {
            self.mark_stale(&id, false);
        }
        self.drop_unused_rows(cx);
    }

    /// top is the floating header plus a running pull to refresh
    /// bottom is the screen inset below the last row
    fn apply_viewport_insets(&self, window: &mut Window, cx: &mut App) {
        let Some(input) = self.current_input() else {
            return;
        };
        let held_inset = if let Some(refresh_control) = &self.refresh_control {
            let base_top_inset = input.floating_top_inset.as_f32();
            let width = self.bounds.get().size.width.as_f32();
            refresh_control.update(cx, |control, cx| {
                control.set_base_top_inset(base_top_inset, cx);
                control.layout_indicator(width, cx);
            });
            refresh_control.read(cx).held_inset()
        } else {
            0.0
        };
        let reserved_inset = self.load_more_footer.read(cx).reserved_inset();
        let insets = gpui::Edges {
            top: input.floating_top_inset + px(held_inset),
            left: px(0.0),
            bottom: input.bottom_inset + px(reserved_inset),
            right: px(0.0),
        };
        self.list_view.update(cx, |list, cx| {
            if list.viewport_insets() != insets {
                let metrics = list.scroll_metrics();
                tracing::warn!(
                    target: "riseonly::X1993",
                    "viewport insets top {}->{} held={} offset={} min={} scrolling={}",
                    list.viewport_insets().top.as_f32(),
                    insets.top.as_f32(),
                    held_inset,
                    metrics.content_offset_y.as_f32(),
                    metrics.minimum_content_offset_y.as_f32(),
                    list.is_scroll_in_progress()
                );
                list.set_viewport_insets(insets, window, cx);
            }
        });
        self.layout_load_more_footer(cx);
    }

    fn layout_load_more_footer(&self, cx: &mut App) {
        let Some(metrics) = self.last_metrics.get() else {
            return;
        };
        let bottom_inset = self.list_view.read(cx).viewport_insets().bottom.as_f32();
        let width = self.bounds.get().size.width.as_f32();
        self.load_more_footer.update(cx, |footer, cx| footer.layout(&metrics, bottom_inset, width, cx));
    }

    fn schedule_pagination_retry(&self, cx: &mut App) {
        self.pagination_retry_work.borrow_mut().take();
        if self.is_stopped.get() {
            return;
        }
        let Some(delay) = self.pagination.borrow().retry_delay_remaining() else {
            return;
        };
        let owner = self.this.clone();
        let wait = Duration::from_secs_f64(delay.max(0.0) + 0.05);
        let work = cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(wait).await;
            let Some(controller) = owner.upgrade() else {
                return;
            };
            cx.update(|cx| {
                let _finished = controller.pagination_retry_work.borrow_mut().take();
                controller.retry_pagination(cx);
            });
        });
        *self.pagination_retry_work.borrow_mut() = Some(work);
    }

    fn retry_pagination(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let Some(metrics) = self.last_metrics.get() else {
            return;
        };
        self.evaluate_pagination(&metrics, cx);
        let is_waiting = self.is_waiting_pagination_retry();
        let changed = self.load_more_footer.update(cx, |footer, cx| footer.set_waiting_retry(is_waiting, cx));
        if changed {
            self.layout_load_more_footer(cx);
        }
    }

    fn is_waiting_pagination_retry(&self) -> bool {
        let is_loading_more = self.input.borrow().as_ref().is_some_and(|input| input.page.is_loading_more);
        !is_loading_more && self.pagination.borrow().retry_delay_remaining().is_some()
    }

    fn connection_did_return(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        self.pagination.borrow_mut().connection_did_return();
        self.pagination_retry_work.borrow_mut().take();
        self.retry_pagination(cx);
    }

    fn activity_did_change(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let is_active = self.input.borrow().as_ref().is_none_or(|input| input.is_active);
        self.is_active.set(is_active);
        let is_in_window = self.is_in_window.get();
        self.playback_focus.update(cx, |focus, cx| focus.set_suspended(!is_active || !is_in_window, cx));
        self.tracker.update(cx, |tracker, cx| tracker.set_suspended(!is_active || !is_in_window, cx));
        if !is_in_window {
            return;
        }
        if is_active {
            let lower = self.lower.get();
            self.warmups.activate();
            self.warmups.window_top_did_move(lower, cx);
            self.schedule_evaluation(cx);
        } else {
            self.warmups.deactivate(cx);
        }
    }

    fn configure_destination(&self) {
        let (Some(input), Some(destination)) = (self.current_input(), self.destination()) else {
            return;
        };
        let scope = input.scope.clone();
        let anchor_id = input.anchor_id.clone();
        let origin_scope_key = scope.key();
        destination.configure(
            origin_scope_key,
            Rc::new(move |cx: &mut App| {
                let store = PostServicesStore::shared(cx);
                let page_items = store.update(cx, |store, cx| store.page(&scope, cx)).items;
                if page_items.iter().any(|item| item.id == anchor_id) {
                    return page_items;
                }
                let Some(anchor) = store.update(cx, |store, cx| store.resolved_post(&anchor_id, cx)) else {
                    return page_items;
                };
                let mut items = Vec::with_capacity(page_items.len() + 1);
                items.push(anchor);
                items.extend(page_items);
                items
            }),
        );
    }

    /// origin scope can get evicted while this page sits in
    /// the stack, reload it so rows around the anchor come back
    fn reactivate_origin_if_evicted(&self, status: &RisePostPageStatus, cx: &mut App) {
        let Some(input) = self.current_input() else {
            return;
        };
        if *status == RisePostPageStatus::Idle || *status == RisePostPageStatus::Failed {
            (input.reactivate)(cx);
        }
    }

    /// a feed restored mid page and pulled at its window top swapped the whole
    /// feed for a fresh first page under the reader, so refresh is gated here
    fn update_refresh_enabled(&self, cx: &mut App) {
        let Some(refresh_control) = &self.refresh_control else {
            return;
        };
        let input = self.current_input();
        let is_enabled = input.as_ref().is_some_and(|input| input.refresh.is_some() && input.is_active)
            && ProfilePostListWindowPolicy::refresh_allowed(self.lower.get().max(self.presented_window_lower()));
        refresh_control.update(cx, |control, cx| control.set_is_enabled(is_enabled, cx));
    }

    /// row index of the first presented row, lower until the first commit applied
    fn presented_window_lower(&self) -> usize {
        let rows = self.current_rows();
        let presented_ids = self.reconciler.presented_ids();
        presented_ids
            .first()
            .and_then(|id| rows.index_by_presentation_id.get(id).copied())
            .unwrap_or(self.lower.get())
    }

    fn update_destination_layout(&self) {
        let (Some(input), Some(destination)) = (self.current_input(), self.destination()) else {
            return;
        };
        destination.update_layout(
            self.presented_window_lower(),
            f32::from(input.floating_top_inset),
            f32::from(input.safe_bottom),
        );
    }

    fn host_did_appear(&self, window: &mut Window, cx: &mut App) {
        if let Some(destination) = self.destination() {
            self.attach_destination(&destination, window);
        } else {
            self.warm_diamond_if_needed(cx);
        }
        self.configure_destination();
        self.observation.update(cx, |observation, cx| observation.start(cx));
        if self.environment.borrow().is_some() {
            self.spec_did_change(cx);
        }
        self.trace.start_recording_if_enabled(window, cx);
        let is_active = self.is_active.get();
        if is_active {
            let lower = self.lower.get();
            self.warmups.activate();
            self.warmups.window_top_did_move(lower, cx);
        }
        self.tracker.update(cx, |tracker, cx| tracker.set_suspended(!is_active, cx));
        self.playback_focus.update(cx, |focus, cx| focus.set_suspended(!is_active, cx));
        self.leaf_queue.update(cx, |queue, cx| queue.resume(cx));
        if let Some(input) = self.current_input() {
            if let Some(anchor_post) = &input.anchor_post
                && let Some(consume_comment_focus) = &input.consume_comment_focus
            {
                consume_comment_focus(anchor_post, cx);
            }
            self.reactivate_origin_if_evicted(&input.page.status, cx);
        }
        self.playback_focus.update(cx, |focus, cx| focus.report_frames(cx));
        self.tracker.update(cx, |tracker, cx| tracker.evaluate(cx));
        self.schedule_evaluation(cx);
    }

    fn host_did_disappear(&self, cx: &mut App) {
        self.warmups.deactivate(cx);
        self.tracker.update(cx, |tracker, cx| tracker.set_suspended(true, cx));
        self.playback_focus.update(cx, |focus, cx| focus.set_suspended(true, cx));
        self.trace.stop_recording("disappear");
    }

    fn start_first_commit_if_possible(&self, window: &mut Window, cx: &mut App) {
        let bounds = self.bounds.get();
        let rows = self.current_rows();
        if bounds.size.width <= px(1.0)
            || bounds.size.height <= px(1.0)
            || self.input.borrow().is_none()
            || rows.count() == 0
        {
            return;
        }
        let Some(environment) = self.ensure_environment(cx) else {
            return;
        };
        let start = self.lower.get().min(rows.count() - 1);
        let Some(index) = (start..rows.count()).find(|index| rows.is_engine_row(*index)) else {
            return;
        };
        self.did_start_first_commit.set(true);
        self.first_commit_started_at.set(Self::current_media_time());
        *self.first_row_id.borrow_mut() = Some(rows.presentation_ids[index].clone());

        let post_input = self.preparer.input(&rows.items[index], cx);
        let row = if let Some(cached) = self.preparer.cached_row(&post_input, &environment) {
            cached
        } else {
            let lookup = self.measurer.hosted_height_lookup();
            let span = self.trace.begin(PostListTraceSpan::OpenSyncAnchorPrepare, || {
                trace_fields([("route", PostHostedRowView::route_name(&post_input.route).to_owned())])
            });
            let row = self.preparer.prepare_synchronously(&post_input, &environment, Some(lookup), cx);
            self.trace.end(span, 0.0, HashMap::new);
            row
        };
        self.store(row.clone(), PostListControllerStoreReason::Prepared, cx);
        let mut hosted = Vec::new();
        if row.route.is_hosted_fallback() {
            hosted.push(row.clone());
        }
        self.prepare_first_viewport_rows(index, &row, &environment, &mut hosted, cx);
        if !hosted.is_empty() {
            self.measurer.submit(&hosted, cx);
        }
        self.reconciler.commit_now(window, cx);
        let lower = self.lower.get();
        self.warmups.window_top_did_move(lower, cx);
        self.schedule_evaluation(cx);
    }

    fn prepare_first_viewport_rows(
        &self,
        after: usize,
        first_row: &Arc<PreparedPostRow>,
        environment: &PostRowPrepareEnvironment,
        hosted: &mut Vec<Arc<PreparedPostRow>>,
        cx: &mut App,
    ) {
        let rows = self.current_rows();
        let header_height = if self.lower.get() == 0 {
            self.configuration.header.as_ref().map_or(0.0, |header| header.height)
        } else {
            0.0
        };
        let viewport_height = f32::from(self.bounds.get().size.height);
        let mut covered = first_row.height() + header_height;
        let started_at = Self::current_media_time();
        let mut added = 0;
        let mut index = after + 1;
        while index < rows.count()
            && covered < viewport_height
            && added < Self::FIRST_VIEWPORT_MAX_EXTRA_ROWS
            && Self::current_media_time() - started_at < Self::FIRST_VIEWPORT_SYNC_BUDGET_SECONDS
        {
            if !rows.is_engine_row(index) || self.row(&rows.presentation_ids[index]).is_some() {
                index += 1;
                continue;
            }
            let post_input = self.preparer.input(&rows.items[index], cx);
            let next = match self.preparer.cached_row(&post_input, environment) {
                Some(cached) => cached,
                None => {
                    let lookup = self.measurer.hosted_height_lookup();
                    self.preparer.prepare_synchronously(&post_input, environment, Some(lookup), cx)
                }
            };
            self.store(next.clone(), PostListControllerStoreReason::Prepared, cx);
            covered += next.height();
            if next.route.is_hosted_fallback() {
                hosted.push(next);
            }
            added += 1;
            index += 1;
        }
    }

    fn current_media_time() -> f64 {
        RiseClock::monotonic().as_secs_f64()
    }

    fn make_environment(&self, width: f32, cx: &mut App) -> PostRowPrepareEnvironment {
        PostRowPrepareEnvironment::current(width, Some(self.display_scale.get()), cx)
    }

    /// current prepare environment, rebuilt when width or account changed
    fn ensure_environment(&self, cx: &mut App) -> Option<PostRowPrepareEnvironment> {
        let width = self.bounds.get().size.width.as_f32();
        let current = self.environment.borrow().clone();
        if width <= 1.0 {
            return current;
        }
        if self.environment_needs_rebuild.get() {
            self.environment_needs_rebuild.set(false);
            let next = self.make_environment(width, cx);
            if current.as_ref() != Some(&next) {
                self.install(next.clone(), current.is_some(), cx);
            }
            return Some(next);
        }
        if let Some(environment) = &current
            && (environment.spec.width - width).abs() <= 0.5
            && (environment.spec.screen_height - f32::from(PostMediaBox::viewport().height)).abs() <= 0.5
            && environment.account_user_id == SaiAccountCacheScope::active_user_id()
            && environment.account_epoch == SaiAccountCacheScope::active_epoch()
        {
            return current;
        }
        let next = self.make_environment(width, cx);
        self.install(next.clone(), current.is_some(), cx);
        Some(next)
    }

    fn install(&self, next: PostRowPrepareEnvironment, marks_rows_stale: bool, cx: &mut App) {
        let text_secondary = next.palette.text_secondary;
        self.background.set(Some(next.palette.background100));
        *self.environment.borrow_mut() = Some(next);
        self.environment_generation.set(self.environment_generation.get().wrapping_add(1));
        *self.row_context.borrow_mut() = None;
        cx.notify(self.view.entity_id());
        if let Some(indicator) = &self.refresh_indicator {
            indicator.update(cx, |indicator, cx| indicator.set_tint(text_secondary, cx));
        }
        self.load_more_footer.update(cx, |footer, cx| footer.set_tint(text_secondary, cx));
        self.cancel_preparation();
        if !marks_rows_stale {
            return;
        }
        let ids: Vec<String> = self.table.borrow().keys().cloned().collect();
        for id in ids {
            self.mark_stale(&id, true);
        }
    }

    /// environment for hosted leaves, fallback rows and the header row
    fn hosted_environment(&self) -> Option<PostListHostedEnvironment> {
        let surface = self.destination().map(|destination| destination.surface());
        self.input.borrow().as_ref().map(|input| input.hosted_environment(surface))
    }

    fn current_context(&self, cx: &mut App) -> Option<Rc<PostRowMainContext>> {
        if let Some(row_context) = self.row_context.borrow().as_ref() {
            return Some(row_context.clone());
        }
        let environment = self.environment.borrow().clone()?;
        let input = self.current_input()?;
        let surface = self.destination().map(|destination| destination.surface());
        let lift = Rc::downgrade(&PostRowViewerLiftHub::observer(&input.media_viewer_presentation, cx));
        let observation = self.observation.read(cx);
        let own_premium = observation.own_premium().cloned();
        let feed_audio_muted = observation.is_feed_audio_muted();
        let revision = observation.revision();
        let story_observation = self.observation.downgrade();
        let registration_surface = surface.clone();
        let built = Rc::new(PostRowMainContext {
            session: self.session,
            spec: environment.spec.clone(),
            palette: environment.palette.clone(),
            locale: input.locale.clone(),
            calendar: input.calendar.clone(),
            time_zone: input.time_zone.clone(),
            account_user_id: environment.account_user_id.clone(),
            account_epoch: environment.account_epoch.clone(),
            own_premium,
            feed_audio_muted,
            revision,
            router: self.router.clone() as Rc<dyn PostRowActionRouting>,
            make_anchor_registration: Rc::new(move |_: &mut App| -> Rc<dyn PostRowAnchorRegistering> {
                Rc::new(PostRowAnchorRegistration::new(registration_surface.clone(), lift.upgrade()))
            }),
            focus: Rc::new(self.playback_focus.clone()) as Rc<dyn PostListFocusReporting>,
            story_ring_state: Rc::new(move |user_id: &str, cx: &App| {
                story_observation
                    .upgrade()
                    .map(|observation| observation.read(cx).story_ring_state(user_id, cx))
                    .unwrap_or(PostRowStoryRingState::Unknown)
            }),
            shared_element_surface: surface.clone(),
            hosted: input.hosted_environment(surface),
        });
        *self.row_context.borrow_mut() = Some(built.clone());
        Some(built)
    }

    fn spec_did_change(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let width = self.bounds.get().size.width.as_f32();
        if width <= 1.0 {
            self.environment_needs_rebuild.set(true);
            tracing::warn!(target: "riseonly::post_list_theme", "spec change deferred: no width");
            return;
        }
        self.environment_needs_rebuild.set(false);
        let next = self.make_environment(width, cx);
        let current = self.environment.borrow().clone();
        if current.as_ref() == Some(&next) {
            return;
        }
        let rows = self.table.borrow().len();
        let in_window = self.is_in_window.get();
        tracing::warn!(target: "riseonly::post_list_theme", "spec change installed rows={rows} in_window={in_window}");
        self.install(next, current.is_some(), cx);
        self.schedule_evaluation(cx);
    }

    fn viewport_width_did_change(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let list_bounds = self.list_view.read(cx).bounds();
        tracing::warn!(
            target: "riseonly::X1856",
            "viewport width changed list={} host={} env={:?} rows={}",
            list_bounds.size.width.as_f32(),
            self.bounds.get().size.width.as_f32(),
            self.environment.borrow().as_ref().map(|environment| environment.spec.width),
            self.table.borrow().len()
        );
        if list_bounds.size.width > px(1.0) {
            let mut bounds = self.bounds.get();
            bounds.size.width = list_bounds.size.width;
            self.bounds.set(bounds);
        }
        let _ = self.ensure_environment(cx);
        self.schedule_evaluation(cx);
    }

    fn caches_did_purge(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let account_changed = match self.environment.borrow().as_ref() {
            Some(environment) => {
                environment.account_user_id != SaiAccountCacheScope::active_user_id()
                    || environment.account_epoch != SaiAccountCacheScope::active_epoch()
            }
            None => return,
        };
        if !account_changed {
            return;
        }
        let _ = self.ensure_environment(cx);
        self.schedule_evaluation(cx);
    }

    fn item(&self, id: &str, cx: &mut App) -> RiseManualListItem {
        if let Some(header) = &self.configuration.header
            && header.id == id
        {
            return self.header_item(header);
        }
        let Some(row) = self.row(id) else {
            return Self::missing_item();
        };
        let Some(context) = self.current_context(cx) else {
            return Self::missing_item();
        };
        match &row.route {
            PostRowRoute::HostedFallback(_) => {
                let owner = self.this.clone();
                let fallback_context = context;
                PostHostedRowView::make_list_item(
                    &row,
                    Some(&self.measurer),
                    Rc::new(move |cx: &mut App| {
                        owner
                            .upgrade()
                            .and_then(|controller| controller.current_context(cx))
                            .unwrap_or_else(|| fallback_context.clone())
                    }),
                    cx,
                )
            }
            PostRowRoute::Native(_) => self.native_item(&row, context),
        }
    }

    fn header_item(&self, header: &PostListHeader) -> RiseManualListItem {
        let environment = self.hosted_environment();
        let generation = self.global_context_generation.get();
        let header_id = header.id.clone();
        let content = header.content.clone();
        RiseManualListItem::new(
            header.height,
            PostListHeaderRowView::REUSE_IDENTIFIER.to_string(),
            i64::from_ne_bytes(generation.to_ne_bytes()),
            RiseManualListItemMountPolicy::Immediate,
            None,
            None,
            None,
            Some(RiseManualListInstallCost::Native),
            Rc::new(move |reusable_view: Option<AnyView>, window: &mut Window, cx: &mut App| -> AnyView {
                let row_view = reusable_view
                    .and_then(|view| view.downcast::<PostListHeaderRowView>().ok())
                    .unwrap_or_else(|| cx.new(PostListHeaderRowView::new));
                if let Some(environment) = &environment {
                    row_view.update(cx, |row_view, cx| {
                        row_view.configure(&header_id, generation, |cx| content(window, cx), environment, cx);
                    });
                }
                row_view.into()
            }),
        )
    }

    fn native_item(&self, row: &Arc<PreparedPostRow>, context: Rc<PostRowMainContext>) -> RiseManualListItem {
        let reuse_identifier = row.reuse_identifier().to_string();
        let background = context.palette.background100;
        let separator = context.palette.border100;
        let trace = self.trace.clone();
        let leaf_queue = self.leaf_queue.clone();
        let owner = self.this.clone();
        let row_for_view = row.clone();
        let row_reuse_identifier = reuse_identifier.clone();
        RiseManualListItem::new(
            row.height(),
            reuse_identifier,
            self.engine_version(&row.presentation_id),
            row.route.mount_policy(),
            None,
            None,
            Some(Rc::new(move |_: &mut Window, cx: &mut App| -> AnyView {
                cx.new(|_| PostListRowPlaceholderView { background, separator }).into()
            })),
            Some(RiseManualListInstallCost::Native),
            Rc::new(move |reusable_view: Option<AnyView>, _: &mut Window, cx: &mut App| -> AnyView {
                let row_view = reusable_view
                    .and_then(|view| view.downcast::<PostNativeRowView>().ok())
                    .unwrap_or_else(|| Self::make_native_row(&row_reuse_identifier, cx));
                let span = trace.begin(PostListTraceSpan::RowConfigureNative, || {
                    trace_fields([("type", row_reuse_identifier.clone())])
                });
                let current = owner
                    .upgrade()
                    .and_then(|controller| controller.current_context(cx))
                    .unwrap_or_else(|| context.clone());
                row_view.update(cx, |row_view, cx| row_view.configure(&row_for_view, &current, cx));
                trace.end(span, 1.0, HashMap::new);
                let presentation_id = row_for_view.presentation_id.clone();
                leaf_queue.update(cx, |queue, cx| queue.row_did_configure(&presentation_id, cx));
                row_view.into()
            }),
        )
    }

    fn make_native_row(reuse_identifier: &str, cx: &mut App) -> Entity<PostNativeRowView> {
        if reuse_identifier == PostListReuseIdentifier::NATIVE_PLAIN {
            return cx.new(|cx| PostNativeRowView::new(None, cx));
        }
        cx.new(|cx| {
            PostNativeRowView::new(
                Some(Rc::new(|row_view: EntityId, cx: &mut App| -> Rc<dyn PostRowMediaHosting> {
                    Rc::new(cx.new(|cx| PostRowMediaView::new(row_view, cx)))
                })),
                cx,
            )
        })
    }

    /// never reached, desired ids only name prepared rows
    fn missing_item() -> RiseManualListItem {
        RiseManualListItem::new(
            1.0,
            "post.missing".to_string(),
            0,
            RiseManualListItemMountPolicy::Immediate,
            None,
            None,
            None,
            Some(RiseManualListInstallCost::Native),
            Rc::new(|reusable_view: Option<AnyView>, _: &mut Window, cx: &mut App| {
                reusable_view.unwrap_or_else(|| cx.new(|_| EmptyView).into())
            }),
        )
    }

    fn engine_version(&self, id: &str) -> i64 {
        let stamp = PostListControllerEngineStamp {
            serial: self.table_serials.borrow().get(id).copied().unwrap_or(0),
            row_generation: self.row_context_generations.borrow().get(id).copied().unwrap_or(0),
            global_generation: self.global_context_generation.get(),
        };
        if let Some(entry) = self.engine_versions.borrow().get(id)
            && entry.stamp == stamp
        {
            return entry.version;
        }
        let version = self.next_engine_version.get().wrapping_add(1);
        self.next_engine_version.set(version);
        self.engine_versions.borrow_mut().insert(id.to_owned(), PostListControllerEngineVersion { stamp, version });
        version
    }

    fn store(&self, row: Arc<PreparedPostRow>, reason: PostListControllerStoreReason, cx: &App) {
        let id = row.presentation_id.clone();
        let previous = self.row(&id);
        if previous.as_ref().is_some_and(|previous| Arc::ptr_eq(previous, &row)) {
            return;
        }
        self.table.borrow_mut().insert(id.clone(), row.clone());
        let serial = self.next_serial.get().wrapping_add(1);
        self.next_serial.set(serial);
        self.table_serials.borrow_mut().insert(id.clone(), serial);
        self.cached_prepared_end.set(None);
        let rows = self.current_rows();
        let is_current_post =
            rows.index_by_presentation_id.get(&id).is_some_and(|index| rows.items[*index] == row.post);
        let is_current_spec =
            self.environment.borrow().as_ref().is_some_and(|environment| row.key.spec == environment.spec);
        if row.post.is_liked || previous.as_ref().is_some_and(|previous| previous.post.is_liked != row.post.is_liked) {
            x251_mark(|| {
                format!(
                    "x251 store id={id} liked={} current_post={is_current_post} current_spec={is_current_spec} had_previous={} retained={}",
                    row.post.is_liked,
                    previous.is_some(),
                    self.reconciler.is_retained(&id)
                )
            });
        }
        if is_current_post && is_current_spec {
            self.stale_set.borrow_mut().remove(&id);
            self.idle_only_stale.borrow_mut().remove(&id);
            self.aspect_stale.borrow_mut().remove(&id);
        }
        let Some(previous) = previous else {
            return;
        };
        if !self.reconciler.is_retained(&id) {
            return;
        }
        let delta = row.height() - previous.height();
        let height_changed = delta.abs() > 0.5;
        self.reconciler.mark_dirty(&id, height_changed, reason == PostListControllerStoreReason::Spec);
        if !height_changed || !self.reconciler.presented_set().contains(&id) {
            return;
        }
        let is_visible = self.list_view.read(cx).visible_ids().contains(&id);
        self.trace.count_late_height_update(delta, is_visible, reason.name());
        if reason == PostListControllerStoreReason::Aspect {
            self.trace.record(PostListTraceEvent::AspectLateUpdate, || {
                trace_fields([
                    ("delta", PostListPerformanceTrace::decimal(f64::from(delta))),
                    ("visible", if is_visible { "1" } else { "0" }.to_owned()),
                ])
            });
        }
    }

    fn remove_row(&self, id: &str) {
        self.table.borrow_mut().remove(id);
        self.table_serials.borrow_mut().remove(id);
        self.engine_versions.borrow_mut().remove(id);
        self.row_context_generations.borrow_mut().remove(id);
        self.stale_set.borrow_mut().remove(id);
        self.idle_only_stale.borrow_mut().remove(id);
        self.aspect_stale.borrow_mut().remove(id);
        self.cached_prepared_end.set(None);
    }

    /// drops rows that left the page and that no transaction can still ask for
    fn drop_unused_rows(&self, cx: &mut App) {
        let retained = self.reconciler.retained_ids();
        let rows = self.current_rows();
        let unused: Vec<String> = self
            .table
            .borrow()
            .keys()
            .filter(|id| !rows.index_by_presentation_id.contains_key(*id) && !retained.contains(*id))
            .cloned()
            .collect();
        let removed = !unused.is_empty();
        for id in unused {
            self.remove_row(&id);
        }
        if removed {
            let stale_set = self.stale_set.borrow();
            self.stale_order.borrow_mut().retain(|id| stale_set.contains(id));
        }
        let presentation_ids: HashSet<String> = self.table.borrow().keys().cloned().collect();
        self.measurer.retain_requests(&presentation_ids, cx);
    }

    fn mark_stale(&self, id: &str, idle_only: bool) {
        if self.stale_set.borrow_mut().insert(id.to_owned()) {
            self.stale_order.borrow_mut().push(id.to_owned());
        }
        if idle_only {
            self.idle_only_stale.borrow_mut().insert(id.to_owned());
        }
    }

    fn store_reason(&self, id: &str) -> PostListControllerStoreReason {
        if self.idle_only_stale.borrow().contains(id) {
            return PostListControllerStoreReason::Spec;
        }
        if self.aspect_stale.borrow().contains(id) {
            return PostListControllerStoreReason::Aspect;
        }
        PostListControllerStoreReason::Prepared
    }

    /// engine ids from the window top, prepared rows up to the
    /// first unprepared one plus rows already presented past it
    /// (a middle insert waits and removes nothing)
    fn desired_ids(&self) -> Vec<String> {
        let rows = self.current_rows();
        if rows.count() == 0 {
            return Vec::new();
        }
        let presented = self.reconciler.presented_set();
        let lower = self.lower.get();
        let table = self.table.borrow();
        let mut ids = Vec::new();
        if let Some(header) = &self.configuration.header
            && (lower == 0 || presented.contains(&header.id))
        {
            ids.push(header.id.clone());
        }
        let mut reached_gap = false;
        for index in lower.min(rows.count())..rows.count() {
            if !rows.is_engine_row(index) {
                continue;
            }
            let id = &rows.presentation_ids[index];
            if !table.contains_key(id) {
                reached_gap = true;
                continue;
            }
            if !reached_gap || presented.contains(id) {
                ids.push(id.clone());
            }
        }
        let header_id = self.configuration.header.as_ref().map(|header| header.id.as_str());
        let has_engine_row = ids.iter().any(|id| Some(id.as_str()) != header_id);
        if reached_gap && !has_engine_row {
            let presented_ids = self.reconciler.presented_ids();
            if presented_ids.iter().any(|id| Some(id.as_str()) != header_id) {
                return presented_ids.to_vec();
            }
        }
        ids
    }

    fn contiguous_prepared_end(&self) -> usize {
        if let Some(cached_prepared_end) = self.cached_prepared_end.get() {
            return cached_prepared_end;
        }
        let rows = self.current_rows();
        let table = self.table.borrow();
        let mut index = self.lower.get().min(rows.count());
        while index < rows.count() && (!rows.is_engine_row(index) || table.contains_key(&rows.presentation_ids[index])) {
            index += 1;
        }
        self.cached_prepared_end.set(Some(index));
        index
    }

    /// prepared heights above the window
    /// nearest first, up to the first unprepared row
    fn prepared_heights_above(&self) -> Vec<f32> {
        let rows = self.current_rows();
        let table = self.table.borrow();
        let mut heights = Vec::new();
        let mut index = self.lower.get().min(rows.count());
        while index > 0 && heights.len() < ProfilePostListWindowPolicy::MAXIMUM_REVEAL_ROWS_PER_COMMIT {
            index -= 1;
            if rows.is_engine_row(index) {
                let Some(row) = table.get(&rows.presentation_ids[index]) else {
                    break;
                };
                heights.push(row.height());
            } else {
                heights.push(0.0);
            }
        }
        heights
    }

    fn pump_preparation(&self, cx: &mut App) {
        if self.is_stopped.get() || !self.did_start_first_commit.get() {
            return;
        }
        let Some(environment) = self.ensure_environment(cx) else {
            return;
        };
        let mut budget = Self::SEEDS_PER_TURN;
        let mut stored_synchronously = false;
        let rows = self.current_rows();

        let refresh_is_free = !self.in_flight.borrow().contains_key(&PostListControllerPrepareSlot::Refresh);
        if refresh_is_free && !self.stale_set.borrow().is_empty() {
            let visible: Vec<String> = {
                let stale_set = self.stale_set.borrow();
                self.list_view.read(cx).visible_ids().iter().filter(|id| stale_set.contains(*id)).cloned().collect()
            };
            let visible_set: HashSet<String> = visible.iter().cloned().collect();
            {
                let stale_set = self.stale_set.borrow();
                self.stale_order.borrow_mut().retain(|id| stale_set.contains(id));
            }
            let remaining: Vec<String> =
                self.stale_order.borrow().iter().filter(|id| !visible_set.contains(*id)).cloned().collect();
            let mut inputs: Vec<PostRowPrepareInput> = Vec::new();
            for id in visible.into_iter().chain(remaining) {
                if budget <= 0 {
                    break;
                }
                let Some(index) = rows.index_by_presentation_id.get(&id).copied() else {
                    self.stale_set.borrow_mut().remove(&id);
                    self.idle_only_stale.borrow_mut().remove(&id);
                    self.aspect_stale.borrow_mut().remove(&id);
                    continue;
                };
                budget -= 1;
                let post_input = self.preparer.input(&rows.items[index], cx);
                if let Some(cached) = self.preparer.cached_row(&post_input, &environment) {
                    self.store(cached, self.store_reason(&id), cx);
                    stored_synchronously = true;
                } else {
                    inputs.push(post_input);
                }
            }
            self.start_preparation(PostListControllerPrepareSlot::Refresh, inputs, &environment, cx);
        }

        let is_in_window = self.is_in_window.get();
        let suffix_is_free = !self.in_flight.borrow().contains_key(&PostListControllerPrepareSlot::Suffix);
        let scroll_metrics = self.list_view.read(cx).scroll_metrics();
        if is_in_window
            && suffix_is_free
            && budget > 0
            && self.is_suffix_caught_up()
            && self.needs_suffix_rows(&scroll_metrics)
        {
            let mut inputs: Vec<PostRowPrepareInput> = Vec::new();
            for index in self.contiguous_prepared_end()..rows.count() {
                if budget <= 0 {
                    break;
                }
                if !rows.is_engine_row(index) || self.has_row(&rows.presentation_ids[index]) {
                    continue;
                }
                budget -= 1;
                let post_input = self.preparer.input(&rows.items[index], cx);
                if let Some(cached) = self.preparer.cached_row(&post_input, &environment) {
                    self.store(cached, PostListControllerStoreReason::Prepared, cx);
                    stored_synchronously = true;
                } else {
                    inputs.push(post_input);
                }
            }
            self.start_preparation(PostListControllerPrepareSlot::Suffix, inputs, &environment, cx);
        }

        self.reveal.borrow_mut().settle_if_no_transition_session(cx);
        let above_is_free = !self.in_flight.borrow().contains_key(&PostListControllerPrepareSlot::Above);
        let lower = self.lower.get();
        let open_settled = self.reveal.borrow().open_settled();
        if is_in_window && above_is_free && budget > 0 && self.reconciler.has_applied() && lower > 0 && open_settled {
            let mut inputs: Vec<PostRowPrepareInput> = Vec::new();
            let floor = lower.saturating_sub(PostListRevealController::WARM_ROWS_ABOVE);
            for index in (floor..lower.min(rows.count())).rev() {
                if budget <= 0 {
                    break;
                }
                if !rows.is_engine_row(index) || self.has_row(&rows.presentation_ids[index]) {
                    continue;
                }
                budget -= 1;
                let post_input = self.preparer.input(&rows.items[index], cx);
                if let Some(cached) = self.preparer.cached_row(&post_input, &environment) {
                    self.store(cached, PostListControllerStoreReason::Prepared, cx);
                    stored_synchronously = true;
                } else {
                    inputs.push(post_input);
                }
            }
            self.start_preparation(PostListControllerPrepareSlot::Above, inputs, &environment, cx);
        }

        if stored_synchronously {
            self.reconciler.set_needs_commit(cx);
        }
    }

    fn start_preparation(
        &self,
        slot: PostListControllerPrepareSlot,
        inputs: Vec<PostRowPrepareInput>,
        environment: &PostRowPrepareEnvironment,
        cx: &mut App,
    ) {
        if inputs.is_empty() {
            return;
        }
        let token = self.next_prepare_token.get().wrapping_add(1);
        self.next_prepare_token.set(token);
        let generation = self.environment_generation.get();
        let priority = if slot == PostListControllerPrepareSlot::Above && !self.reveal.borrow().user_scrolled_down() {
            PostRowPreparePriority::Warm
        } else {
            PostRowPreparePriority::Visible
        };
        let row_count = inputs.len();
        let span = self.trace.begin(PostListTraceSpan::SnapshotPrepare, || {
            trace_fields([("slot", slot.name().to_owned()), ("rows", row_count.to_string())])
        });
        let lookup = self.measurer.hosted_height_lookup();
        let prepare = self.preparer.prepare(inputs, environment, generation, priority, Some(lookup), cx);
        let owner = self.this.clone();
        let task = cx.spawn(async move |cx: &mut AsyncApp| {
            let batch = prepare.await;
            cx.update(|cx| {
                if let Some(controller) = owner.upgrade() {
                    controller.preparation_did_finish(batch, slot, token, cx);
                }
            });
        });
        tracing::warn!(target: "riseonly::X272", "prepare start slot={} rows={row_count} token={token}", slot.name());
        self.in_flight.borrow_mut().insert(slot, PostListControllerInFlight { token, _task: task, span });
    }

    fn preparation_did_finish(
        &self,
        batch: PostRowPrepareBatch,
        slot: PostListControllerPrepareSlot,
        token: u64,
        cx: &mut App,
    ) {
        let entry = {
            let mut in_flight = self.in_flight.borrow_mut();
            if in_flight.get(&slot).is_none_or(|entry| entry.token != token) {
                tracing::warn!(target: "riseonly::X272", "prepare finish slot={} token={token} superseded", slot.name());
                return;
            }
            in_flight.remove(&slot)
        };
        let Some(entry) = entry else {
            return;
        };
        tracing::warn!(
            target: "riseonly::X272",
            "prepare finish slot={} token={token} rows={} complete={} stopped={} env_current={}",
            slot.name(),
            batch.rows.len(),
            batch.is_complete,
            self.is_stopped.get(),
            batch.generation == self.environment_generation.get()
                && self.environment.borrow().as_ref() == Some(&batch.environment)
        );
        self.trace.end(entry.span, 0.0, || {
            trace_fields([
                ("prepared", batch.rows.len().to_string()),
                ("cache_hits", batch.cache_hits.to_string()),
                ("complete", if batch.is_complete { "1" } else { "0" }.to_owned()),
            ])
        });
        if self.is_stopped.get() {
            return;
        }
        let is_current_environment = batch.generation == self.environment_generation.get()
            && self.environment.borrow().as_ref() == Some(&batch.environment);
        if !is_current_environment {
            self.schedule_evaluation(cx);
            return;
        }
        let rows = self.current_rows();
        let mut hosted: Vec<Arc<PreparedPostRow>> = Vec::new();
        for row in &batch.rows {
            let id = &row.presentation_id;
            let Some(index) = rows.index_by_presentation_id.get(id).copied() else {
                continue;
            };
            if rows.items[index] != row.post {
                if self.has_row(id) {
                    self.mark_stale(id, false);
                }
                continue;
            }
            self.store(row.clone(), self.store_reason(id), cx);
            if row.route.is_hosted_fallback() {
                hosted.push(row.clone());
            }
        }
        if !hosted.is_empty() {
            let nearest = self.nearest_first(hosted, cx);
            self.measurer.submit(&nearest, cx);
        }
        if PostHostedRowMeasurer::is_parity_audit_enabled() {
            self.measurer.submit_parity_audit(&batch.rows, cx);
        }
        self.reconciler.set_needs_commit(cx);
        self.schedule_evaluation(cx);
    }

    fn cancel_preparation(&self) {
        let entries: Vec<PostListControllerInFlight> =
            self.in_flight.borrow_mut().drain().map(|(_, entry)| entry).collect();
        for entry in entries {
            self.trace.end(entry.span, 0.0, || trace_fields([("cancelled", "1".to_owned())]));
        }
    }

    fn nearest_first(&self, mut rows_to_sort: Vec<Arc<PreparedPostRow>>, cx: &App) -> Vec<Arc<PreparedPostRow>> {
        let rows = self.current_rows();
        let anchor = self
            .list_view
            .read(cx)
            .visible_ids()
            .first()
            .and_then(|id| rows.index_by_presentation_id.get(id).copied())
            .unwrap_or(self.lower.get());
        rows_to_sort.sort_by_key(|row| {
            rows.index_by_presentation_id
                .get(&row.presentation_id)
                .map_or(usize::MAX, |index| index.abs_diff(anchor))
        });
        rows_to_sort
    }

    /// true when every prepared suffix row is presented and no transaction waits in the engine
    fn is_suffix_caught_up(&self) -> bool {
        if self.reconciler.has_enqueued_transaction() {
            return false;
        }
        self.presented_end() >= self.contiguous_prepared_end()
    }

    /// one past the last presented row, lower before the first commit
    fn presented_end(&self) -> usize {
        let rows = self.current_rows();
        let presented_ids = self.reconciler.presented_ids();
        presented_ids
            .last()
            .and_then(|id| rows.index_by_presentation_id.get(id).copied())
            .map_or(self.lower.get(), |index| index + 1)
    }

    fn needs_suffix_rows(&self, metrics: &RiseManualListScrollMetrics) -> bool {
        if !self.reconciler.has_applied() || self.contiguous_prepared_end() >= self.current_rows().count() {
            return false;
        }
        if self.holds_replaced_rows() {
            return true;
        }
        PostListPaginationPolicy::needs_suffix_rows(
            metrics.distance_from_bottom.as_f32(),
            metrics.viewport_size.height.as_f32(),
            metrics.velocity_y.as_f32(),
        )
    }

    fn holds_replaced_rows(&self) -> bool {
        let rows = self.current_rows();
        let header_id = self.configuration.header.as_ref().map(|header| header.id.as_str());
        let presented_ids = self.reconciler.presented_ids();
        let mut engine_ids = presented_ids.iter().filter(|id| Some(id.as_str()) != header_id).peekable();
        let has_engine_row = engine_ids.peek().is_some();
        has_engine_row && engine_ids.all(|id| !rows.index_by_presentation_id.contains_key(id))
    }

    fn aspects_did_resolve(&self, ids: &HashSet<String>, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        for id in ids {
            if self.has_row(id) {
                self.mark_stale(id, false);
                self.aspect_stale.borrow_mut().insert(id.clone());
            }
        }
        self.schedule_evaluation(cx);
    }

    fn premium_badges_may_have_changed(&self, ids: &HashSet<String>, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let mut changed = false;
        for id in ids {
            let Some(row) = self.row(id) else {
                continue;
            };
            if row.route.is_hosted_fallback() {
                continue;
            }
            let badge = PostRowPremiumBadgeResolver::resolve(&row.seed.author, cx);
            if badge == row.seed.author.premium_badge {
                continue;
            }
            self.mark_stale(id, false);
            changed = true;
        }
        if changed {
            self.schedule_evaluation(cx);
        }
    }

    fn caption_expansion_did_change(&self, presentation_id: &str, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let Some(row) = self.row(presentation_id) else {
            return;
        };
        if row.route.is_hosted_fallback() {
            return;
        }
        let (visible_top, anchor) = {
            let list = self.list_view.read(cx);
            (f32::from(list.viewport_insets().top), list.capture_viewport_anchor(presentation_id))
        };
        if !PostCaptionExpansion::is_expanded(presentation_id, cx)
            && let Some(anchor) = anchor
            && anchor.viewport_offset < visible_top
        {
            self.reconciler.pin_row_top_on_next_height_change(presentation_id, visible_top);
        }
        self.mark_stale(presentation_id, false);
        self.schedule_evaluation(cx);
    }

    fn hosted_heights_did_resolve(&self, resolutions: &[PostHostedRowMeasurerResolution], cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let mut changed = false;
        for resolution in resolutions {
            let Some(row) = self.row(&resolution.presentation_id) else {
                continue;
            };
            if row.key != resolution.key || !row.route.is_hosted_fallback() {
                continue;
            }
            let tolerance = PostHostedRowMeasurePolicy::height_tolerance(row.key.spec.display_scale);
            if !row.is_height_estimated && (row.height() - resolution.row_height).abs() <= tolerance {
                continue;
            }
            let measured = Arc::new(PreparedPostRow::new(
                row.seed.clone(),
                row.route.clone(),
                row.post.clone(),
                None,
                Some(resolution.row_height),
                row.content_version,
                false,
                row.key.clone(),
            ));
            self.store(measured, PostListControllerStoreReason::Measured, cx);
            changed = true;
        }
        if changed {
            self.reconciler.set_needs_commit(cx);
        }
    }

    fn context_did_change(&self, ids: &HashSet<String>, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        *self.row_context.borrow_mut() = None;
        for id in ids {
            let Some(row) = self.row(id) else {
                continue;
            };
            let mut generations = self.row_context_generations.borrow_mut();
            let generation = generations.entry(id.clone()).or_insert(0);
            *generation = generation.wrapping_add(1);
            drop(generations);
            if !row.route.is_hosted_fallback() && self.reconciler.is_retained(id) {
                self.reconciler.mark_dirty(id, false, false);
            }
        }
        self.reconciler.set_needs_commit(cx);
    }

    fn global_context_did_change(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        *self.row_context.borrow_mut() = None;
        self.global_context_generation.set(self.global_context_generation.get().wrapping_add(1));
        let presented: Vec<String> = self.reconciler.presented_ids().to_vec();
        for id in presented {
            if self.row(&id).is_some_and(|row| !row.route.is_hosted_fallback()) {
                self.reconciler.mark_dirty(&id, false, false);
            }
        }
        self.reconciler.set_needs_commit(cx);
    }

    fn focus_did_change(&self, ids: &HashSet<String>, cx: &mut App) {
        self.context_did_change(ids, cx);
        self.leaf_queue.update(cx, |queue, cx| queue.rows_did_change(ids, cx));
    }

    fn commit_did_apply(&self, commit: &PostListCommitReconcilerCommit, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let mut native_count = 0;
        let mut fallback_count = 0;
        for id in &commit.inserted_ids {
            let Some(row) = self.row(id) else {
                continue;
            };
            if row.route.is_hosted_fallback() {
                fallback_count += 1;
            } else {
                native_count += 1;
            }
        }
        self.trace.count_inserted_rows(native_count, fallback_count);
        let first_presented = self.reconciler.presented_ids().first().cloned();
        let presented_from_top = if let Some(header) = &self.configuration.header {
            first_presented.as_deref() == Some(header.id.as_str())
        } else {
            let rows = self.current_rows();
            first_presented.and_then(|id| rows.index_by_presentation_id.get(&id).copied()) == Some(0)
        };
        self.presented_from_top.set(presented_from_top);
        if self.reveal.borrow().restores_departed_top() {
            let rows = self.current_rows();
            let top = rows.presentation_ids.get(self.lower.get());
            if top.is_none_or(|id| self.reconciler.presented_set().contains(id)) {
                self.reveal.borrow_mut().restored_departed_top_did_present();
            }
        }
        let returning = self.reveal.borrow().returning_departed_id().map(str::to_owned);
        if let Some(returning) = returning {
            let rows = self.current_rows();
            let presentation_id = rows.first_index_by_id.get(&returning).and_then(|index| rows.presentation_ids.get(*index));
            if presentation_id.is_none_or(|id| self.reconciler.presented_set().contains(id)) {
                self.reveal.borrow_mut().returning_departed_did_present();
            }
        }
        self.update_destination_layout();
        self.record_first_viewport_if_ready(cx);
        if !commit.removed_ids.is_empty() {
            let presented_set = self.reconciler.presented_set().clone();
            self.playback_focus.update(cx, |focus, cx| focus.retain_tokens(&presented_set, cx));
            self.drop_unused_rows(cx);
        }
        self.playback_focus.update(cx, |focus, cx| focus.report_frames(cx));
        self.tracker.update(cx, |tracker, cx| tracker.evaluate(cx));
        self.schedule_evaluation(cx);
    }

    fn record_first_viewport_if_ready(&self, cx: &App) {
        if self.did_record_first_viewport.get() {
            return;
        }
        let Some(first_row_id) = self.first_row_id.borrow().clone() else {
            return;
        };
        if self.list_view.read(cx).mounted_view(&first_row_id).is_none() {
            return;
        }
        self.did_record_first_viewport.set(true);
        self.trace.record_first_viewport(self.first_commit_started_at.get());
    }

    fn schedule_evaluation(&self, cx: &mut App) {
        if self.is_stopped.get() || self.is_evaluation_scheduled.get() {
            return;
        }
        self.is_evaluation_scheduled.set(true);
        let token = self.evaluation_token.get().wrapping_add(1);
        self.evaluation_token.set(token);
        self.on_next_main_turn(cx, move |controller, window, cx| {
            if controller.evaluation_token.get() != token {
                return;
            }
            controller.is_evaluation_scheduled.set(false);
            controller.evaluate(window, cx);
        });
    }

    fn evaluate(&self, window: &mut Window, cx: &mut App) {
        if self.is_stopped.get() || !self.did_start_first_commit.get() {
            return;
        }
        self.pump_preparation(cx);
        self.reconciler.commit_now(window, cx);
        let metrics = self.list_view.read(cx).scroll_metrics();
        self.evaluate_scroll_driven(metrics, window, cx);
    }

    fn visible_ids_did_change(&self, ids: &[String], cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        self.leaf_queue.update(cx, |queue, cx| queue.visible_ids_did_change(ids, cx));
        if self.is_visible_flush_scheduled.get() {
            return;
        }
        self.is_visible_flush_scheduled.set(true);
        let token = self.visible_flush_token.get().wrapping_add(1);
        self.visible_flush_token.set(token);
        self.on_next_main_turn(cx, move |controller, _, cx| {
            if controller.visible_flush_token.get() != token {
                return;
            }
            controller.is_visible_flush_scheduled.set(false);
            controller.flush_visible_ids(cx);
        });
    }

    fn flush_visible_ids(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let ids = self.list_view.read(cx).visible_ids().to_vec();
        self.tracker.update(cx, |tracker, cx| tracker.visible_ids_did_change(&ids, cx));
        self.observation.update(cx, |observation, cx| observation.tracked_rows_did_change(cx));
        self.measurer.set_needs_pump(cx);
        let rows = self.current_rows();
        if let Some(last_index) = ids.iter().filter_map(|id| rows.index_by_presentation_id.get(id).copied()).max() {
            let lower = self.lower.get();
            self.warmups.row_did_appear(last_index, lower, cx);
        }
        self.record_first_viewport_if_ready(cx);
    }

    fn scroll_metrics_did_change(&self, metrics: RiseManualListScrollMetrics, window: &mut Window, cx: &mut App) {
        if self.is_stopped.get() || !self.did_start_first_commit.get() || Some(metrics) == self.last_metrics.get() {
            return;
        }
        self.evaluate_scroll_driven(metrics, window, cx);
    }

    /// cost is per visible row on each coalesced metrics change, enqueues nothing synchronously
    fn evaluate_scroll_driven(&self, metrics: RiseManualListScrollMetrics, window: &mut Window, cx: &mut App) {
        let previous_metrics = self.last_metrics.get();
        self.last_metrics.set(Some(metrics));
        self.reveal.borrow_mut().observe(&metrics);
        if let Some(pull_down) = self.pull_down.borrow().as_ref() {
            pull_down.scroll_metrics_did_change(&metrics);
        }
        self.update_refresh_enabled(cx);
        if let Some(refresh_control) = &self.refresh_control {
            refresh_control.update(cx, |control, cx| control.observe(&metrics, window, cx));
        }
        self.begin_refresh_after_remount(window, cx);
        self.layout_load_more_footer(cx);
        let is_scrolling = self.list_view.read(cx).is_scroll_in_progress();
        if self.was_scrolling.get() && !is_scrolling {
            self.list_did_become_idle(cx);
        }
        if is_scrolling
            && self.last_reported_anchor_id.borrow().is_none()
            && metrics.distance_from_top > px(1.0)
        {
            self.report_visible_anchor(cx);
        }
        self.was_scrolling.set(is_scrolling);
        if is_scrolling
            && let Some(previous_metrics) = previous_metrics
            && (previous_metrics.is_tracking || previous_metrics.is_dragging || previous_metrics.is_decelerating)
            && !metrics.is_tracking
            && !metrics.is_dragging
            && !metrics.is_decelerating
        {
            self.schedule_evaluation(cx);
        }
        self.tracker.update(cx, |tracker, cx| tracker.evaluate(cx));
        self.playback_focus.update(cx, |focus, cx| focus.report_frames(cx));
        self.evaluate_pagination(&metrics, cx);
        self.evaluate_reveal(&metrics, cx);
        let suffix_is_free = !self.in_flight.borrow().contains_key(&PostListControllerPrepareSlot::Suffix);
        if self.is_in_window.get()
            && self.environment.borrow().is_some()
            && suffix_is_free
            && self.is_suffix_caught_up()
            && self.needs_suffix_rows(&metrics)
        {
            self.schedule_evaluation(cx);
        }
    }

    fn list_did_become_idle(&self, cx: &mut App) {
        self.report_visible_anchor(cx);
        self.measurer.set_needs_pump(cx);
        if self.reconciler.has_deferred_changes() {
            self.reconciler.set_needs_commit(cx);
        }
        self.schedule_evaluation(cx);
    }

    fn land_at_top_after_refresh(&self, window: &mut Window, cx: &mut App) {
        if self.is_stopped.get() || self.lower.get() != 0 {
            return;
        }
        let list = self.list_view.read(cx);
        let settle_distance = px(PostListRefreshControl::HELD_INSET + PostListRefreshControl::TRIGGER_DISTANCE);
        if list.is_scroll_in_progress() || list.scroll_metrics().distance_from_top > settle_distance {
            return;
        }
        let metrics = list.scroll_metrics();
        let is_overscrolled = metrics.content_offset_y < metrics.minimum_content_offset_y - px(0.5);
        if metrics.distance_from_top <= px(0.5) && !is_overscrolled {
            return;
        }
        let minimum_offset_y = metrics.minimum_content_offset_y;
        tracing::warn!(
            target: "riseonly::X1993",
            "land after refresh overscrolled={} offset={} min={} distance={}",
            is_overscrolled,
            metrics.content_offset_y.as_f32(),
            minimum_offset_y.as_f32(),
            metrics.distance_from_top.as_f32()
        );
        self.list_view.update(cx, |list, cx| {
            if is_overscrolled {
                list.set_content_offset_y(minimum_offset_y, false, true, window, cx);
            } else {
                list.smart_scroll_to_top(false, window, cx);
            }
        });
    }

    pub fn refresh_feed_from_tab(&self, window: &mut Window, cx: &mut App) -> bool {
        if !self.configuration.is_feed() || self.is_stopped.get() {
            return false;
        }
        let Some(refresh) = self.current_input().and_then(|input| input.refresh.clone()) else {
            return false;
        };
        if self.lower.get() == 0
            && self.list_view.read(cx).scroll_metrics().distance_from_top <= px(1.0)
            && let Some(refresh_control) = &self.refresh_control
            && refresh_control.read(cx).is_enabled()
        {
            refresh_control.update(cx, |control, cx| control.request_refresh(window, cx));
            return true;
        }
        if self.jump_to_feed_top_by_remount(cx) {
            if let Some(pending) = self.current_input().and_then(|input| input.refresh_after_remount.clone()) {
                pending.set(true);
                return true;
            }
        } else {
            self.list_view.update(cx, |list, cx| list.smart_scroll_to_top(true, window, cx));
            if let Some(refresh_control) = &self.refresh_control
                && refresh_control.read(cx).is_enabled()
            {
                refresh_control.update(cx, |control, cx| control.request_refresh(window, cx));
                return true;
            }
        }
        let refreshing = refresh(cx);
        cx.foreground_executor().spawn(refreshing).detach();
        true
    }

    fn begin_refresh_after_remount(&self, window: &mut Window, cx: &mut App) {
        let Some(refresh_control) = &self.refresh_control else {
            return;
        };
        let Some(pending) = self.current_input().and_then(|input| input.refresh_after_remount.clone()) else {
            return;
        };
        if !pending.get() || self.is_remounting_at_top.get() || self.lower.get() != 0 {
            return;
        }
        if !refresh_control.read(cx).is_enabled() {
            return;
        }
        pending.set(false);
        refresh_control.update(cx, |control, cx| control.begin_refresh_from_tab(window, cx));
    }

    /// for a re-tap of the open tab or the Home key
    /// a feed restored mid page reveals rows above in batches so the window top isnt the feed top
    /// so the list is remounted from row 0 in one landing, false leaves the jump to the list
    pub fn jump_to_feed_top_by_remount(&self, cx: &mut App) -> bool {
        if !self.configuration.is_feed() || self.is_stopped.get() || self.lower.get() == 0 {
            return false;
        }
        let Some(remount) = self.current_input().and_then(|input| input.remount_at_top.clone()) else {
            return false;
        };
        self.is_remounting_at_top.set(true);
        self.report_visible_anchor(cx);
        remount(cx);
        true
    }

    /// reports the first visible post for feeds
    /// none at the very top, so a remounted tab reopens there
    fn report_visible_anchor(&self, cx: &mut App) {
        if !self.configuration.is_feed() {
            return;
        }
        let Some(report) = self.current_input().and_then(|input| input.visible_anchor_did_change.clone()) else {
            return;
        };
        let list = self.list_view.read(cx);
        let id = if self.is_remounting_at_top.get() || list.scroll_metrics().distance_from_top <= px(1.0) {
            None
        } else {
            let rows = self.current_rows();
            list.visible_ids().iter().find(|id| rows.index_by_presentation_id.contains_key(*id)).cloned()
        };
        if self.has_reported_anchor.get() && *self.last_reported_anchor_id.borrow() == id {
            return;
        }
        self.has_reported_anchor.set(true);
        *self.last_reported_anchor_id.borrow_mut() = id.clone();
        report(id.as_deref(), cx);
    }

    fn evaluate_pagination(&self, metrics: &RiseManualListScrollMetrics, cx: &mut App) {
        let Some(input) = self.current_input() else {
            return;
        };
        if !self.is_active.get() || !self.reconciler.has_applied() {
            return;
        }
        let threshold = usize::try_from(PostListPaginationPolicy::ROW_THRESHOLD).unwrap_or(0);
        let mut tail_heights: Vec<f32> = Vec::with_capacity(threshold);
        {
            let presented = self.reconciler.presented_ids();
            let table = self.table.borrow();
            for id in presented.iter().rev() {
                if tail_heights.len() >= threshold {
                    break;
                }
                tail_heights.push(table.get(id).map_or(0.0, |row| row.height()));
            }
        }
        let unpresented_tail_count = self.current_rows().count().saturating_sub(self.presented_end());
        self.trace_pagination_state(metrics, &input, unpresented_tail_count, cx);
        let should_request = self.pagination.borrow_mut().evaluate(
            metrics,
            &input.page,
            &tail_heights,
            i64::try_from(unpresented_tail_count).unwrap_or(i64::MAX),
            PostRowLayout::INTER_ITEM_SPACING,
        );
        if !should_request {
            return;
        }
        self.trace.record_page_request(metrics.distance_from_bottom.as_f32(), metrics.velocity_y.as_f32());
        (input.load_more)(cx);
    }

    fn trace_pagination_state(
        &self,
        metrics: &RiseManualListScrollMetrics,
        input: &PostListInput,
        unpresented_tail_count: usize,
        cx: &App,
    ) {
        let viewport_height = metrics.viewport_size.height.as_f32();
        let distance_from_bottom = metrics.distance_from_bottom.as_f32();
        if viewport_height <= 0.0 || distance_from_bottom > viewport_height * 2.0 {
            return;
        }
        let rows = self.current_rows();
        let (suffix_in_flight, above_in_flight) = {
            let in_flight = self.in_flight.borrow();
            (
                in_flight.contains_key(&PostListControllerPrepareSlot::Suffix),
                in_flight.contains_key(&PostListControllerPrepareSlot::Above),
            )
        };
        let footer = self.load_more_footer.read(cx);
        let list = self.list_view.read(cx);
        let signature = format!(
            "items {} rows {} lower {} presented_end {} prepared_end {} unpresented {} table {} \
             has_more {} loading_more {} refreshing {} error {} suffix_in_flight {} above_in_flight {} \
             enqueued {} deferred {} list_pending {} scrolling {} footer_reserved {} footer_waiting {} \
             distance_bucket {} max_offset {:.0} content_height {:.0} viewport {:.0} insets_bottom {:.0} \
             motion {:?} {}",
            input.page.items.len(),
            rows.count(),
            self.lower.get(),
            self.presented_end(),
            self.contiguous_prepared_end(),
            unpresented_tail_count,
            self.table.borrow().len(),
            input.page.has_more,
            input.page.is_loading_more,
            input.page.is_refreshing,
            input.page.error_message.is_some(),
            suffix_in_flight,
            above_in_flight,
            self.reconciler.has_enqueued_transaction(),
            self.reconciler.has_deferred_changes(),
            list.has_pending_transaction(),
            list.is_scroll_in_progress(),
            footer.is_reserved(),
            footer.is_waiting_retry(),
            (distance_from_bottom / 100.0).floor(),
            metrics.maximum_content_offset_y.as_f32(),
            metrics.content_height.as_f32(),
            viewport_height,
            list.viewport_insets().bottom.as_f32(),
            list.scroll_motion_flags(),
            self.pagination.borrow().diagnostic_summary(),
        );
        if *self.last_pagination_diagnostic.borrow() == signature {
            return;
        }
        tracing::warn!(target: "riseonly::post_list_pagination", "{signature}");
        *self.last_pagination_diagnostic.borrow_mut() = signature;
    }

    fn evaluate_reveal(&self, metrics: &RiseManualListScrollMetrics, cx: &mut App) {
        let lower = self.lower.get();
        let user_scrolled_down = self.reveal.borrow().user_scrolled_down();
        if lower == 0 || !self.is_in_window.get() || !self.reconciler.has_applied() || !user_scrolled_down {
            return;
        }
        let is_dismissing = self.pull_down_is_dismissing();
        let is_in_navigation_transition = self.is_in_navigation_transition(cx);
        let heights_above = self.prepared_heights_above();
        let decision = self.reveal.borrow_mut().decide(
            lower,
            metrics,
            is_dismissing,
            is_in_navigation_transition,
            self.reconciler.is_commit_in_flight(),
            &heights_above,
            PostRowLayout::INTER_ITEM_SPACING,
            cx,
        );
        match decision {
            PostListRevealControllerDecision::Idle => {
                *self.last_reveal_block.borrow_mut() = None;
            }
            PostListRevealControllerDecision::Blocked { reason } => {
                if self.last_reveal_block.borrow().as_deref() != Some(reason.as_str()) {
                    *self.last_reveal_block.borrow_mut() = Some(reason.clone());
                    self.trace.record_reveal_blocked(&reason);
                }
                let above_is_free = !self.in_flight.borrow().contains_key(&PostListControllerPrepareSlot::Above);
                if reason == "not_prepared" && above_is_free {
                    self.schedule_evaluation(cx);
                }
            }
            PostListRevealControllerDecision::Reveal { count } => {
                *self.last_reveal_block.borrow_mut() = None;
                let rows = self.current_rows();
                self.reveal.borrow_mut().commit_reveal(&rows, lower, count);
                let lower = self.reveal.borrow().lower(&rows);
                self.lower.set(lower);
                self.cached_prepared_end.set(None);
                self.warmups.window_top_did_move(lower, cx);
                self.reconciler.set_needs_commit(cx);
            }
        }
    }

    fn can_measure_hosted_rows(&self, cx: &mut App) -> bool {
        if self.is_stopped.get()
            || !self.is_in_window.get()
            || self.list_view.read(cx).is_scroll_in_progress()
            || self.pull_down_is_dismissing()
        {
            return false;
        }
        self.reveal.borrow_mut().settle_if_no_transition_session(cx);
        let open_settled = self.reveal.borrow().open_settled();
        open_settled && !self.is_in_navigation_transition(cx)
    }

    /// rows inserted during a push or pop slide would lay out under a moving page
    fn is_in_navigation_transition(&self, cx: &App) -> bool {
        self.input
            .borrow()
            .as_ref()
            .is_some_and(|input| input.navigation_store.read(cx).navigation_window_has_running_transition(cx))
    }

    fn warm_diamond_if_needed(&self, cx: &mut App) {
        if self.did_warm_diamond.get() || !self.is_in_window.get() {
            return;
        }
        self.did_warm_diamond.set(true);
        PostRowPremiumDiamond::warm(self.display_scale.get(), cx);
    }

    fn router_row(&self, post_id: &str, _cx: &mut App) -> Option<Arc<PreparedPostRow>> {
        let rows = self.current_rows();
        if let Some(index) = rows.first_index_by_id.get(post_id).copied() {
            let presentation_id = &rows.presentation_ids[index];
            if let Some(row) = self.row(presentation_id) {
                let row_index = rows.index_by_presentation_id.get(presentation_id).copied().unwrap_or(index);
                return Some(PostRowPreparation::adopting(&rows.items[row_index], &row));
            }
        }
        let table = self.table.borrow();
        let found = table.values().find(|row| row.post.id == post_id).cloned();
        x251_mark(|| {
            format!(
                "x251 router_row fallback post={post_id} found={} rows={} in_rows={} table={} stopped={}",
                found.is_some(),
                rows.count(),
                rows.first_index_by_id.contains_key(post_id),
                table.len(),
                self.is_stopped.get()
            )
        });
        found
    }

    fn observed_rows(&self, cx: &App) -> Vec<PostListObservedRow> {
        let table = self.table.borrow();
        self.list_view
            .read(cx)
            .mounted_ids()
            .into_iter()
            .filter_map(|id| {
                table.get(&id).map(|row| PostListObservedRow {
                    author_id: row.seed.author.id.clone(),
                    is_author_deleted: row.seed.author.is_deleted,
                    premium_badge: row.seed.author.premium_badge.clone(),
                    presentation_id: id,
                })
            })
            .collect()
    }

    fn needs_leaves(&self, presentation_id: &str) -> bool {
        let Some(row) = self.row(presentation_id) else {
            return false;
        };
        match &row.route {
            PostRowRoute::HostedFallback(_) => false,
            PostRowRoute::Native(PostRowNativeKind::VideoGallery | PostRowNativeKind::RiffCard) => true,
            PostRowRoute::Native(_) => {
                row.seed.counts.is_reposted
                    || (row.seed.body == PostRowBodyKind::Standard && row.seed.has_ambient_music)
                    || row.seed.author.premium_badge.custom_emoji_id().is_some()
            }
        }
    }

    fn page_by_space(&self, is_upward: bool, window: &mut Window, cx: &mut App) {
        self.list_view.update(cx, |list, cx| {
            let insets = list.viewport_insets();
            let metrics = list.scroll_metrics();
            let page_step = (metrics.viewport_size.height - insets.top - insets.bottom).max(px(1.0));
            let target = if is_upward {
                metrics.content_offset_y - page_step
            } else {
                metrics.content_offset_y + page_step
            };
            if list.set_content_offset_y(target, true, true, window, cx) {
                cx.notify();
            }
        });
    }
}

impl PostListWindowOwner for PostListController {
    fn open_did_settle(&self, cx: &mut App) {
        self.reveal.borrow_mut().mark_open_settled();
        self.on_next_main_turn(cx, |controller, _, cx| {
            if controller.is_stopped.get() {
                return;
            }
            controller.measurer.set_needs_pump(cx);
            controller.warm_diamond_if_needed(cx);
            controller.schedule_evaluation(cx);
        });
    }

    fn set_needs_reveal_evaluation(&self, cx: &mut App) {
        self.schedule_evaluation(cx);
    }
}

/// the view half of the controller, what gpui draws
/// list, footer
/// refresh indicator and host probe are stacked on top of each other
pub struct PostListControllerView {
    controller: Weak<PostListController>,
    focus_handle: FocusHandle,
    tab_host_observer: Option<Subscription>,
    left_with_tab: bool,
}

impl PostListControllerView {
    fn observe_tab_host_if_needed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_host_observer.is_some() {
            return;
        }
        let Some(host) = env::<MainTabPageHostKey>(cx) else {
            return;
        };
        let selection = host.selection.clone();
        self.tab_host_observer = Some(cx.observe_in(&selection, window, move |this: &mut Self, _, window, cx| {
            let shown = host.is_shown(cx);
            this.host_tab_did_change(shown, window, cx);
        }));
    }

    fn host_tab_did_change(&mut self, shown: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(controller) = self.controller.upgrade() else {
            return;
        };
        if shown {
            if std::mem::take(&mut self.left_with_tab) {
                controller.did_move_to_window(true, window, cx);
            }
        } else if controller.is_in_window.get() {
            self.left_with_tab = true;
            controller.did_move_to_window(false, window, cx);
        }
    }
}

impl Render for PostListControllerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.observe_tab_host_if_needed(window, cx);
        let root = div().relative().size_full();
        let Some(controller) = self.controller.upgrade() else {
            return root.into_any_element();
        };
        let mut root = match controller.background.get() {
            Some(background) => root.bg(background),
            None => root,
        };
        let weak = self.controller.clone();
        let viewport_clip = controller.tracker.read(cx).viewport_clip();
        let layout_reader = canvas(
            move |bounds, window, cx| {
                viewport_clip.set(Some(window.content_mask().bounds.intersect(&bounds)));
                let Some(controller) = weak.upgrade() else {
                    return;
                };
                if controller.is_stopped.get() {
                    return;
                }
                let joins_window = !controller.is_in_window.get();
                let bounds_changed = controller.bounds.get() != bounds;
                if !joins_window && !bounds_changed {
                    return;
                }
                let owner = weak.clone();
                window.defer(cx, move |window, cx| {
                    let Some(controller) = owner.upgrade() else {
                        return;
                    };
                    controller.layout_subviews(bounds, window, cx);
                    controller.did_move_to_window(true, window, cx);
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let home_owner = self.controller.clone();
        let space_owner = self.controller.clone();
        let focus_handle = self.focus_handle.clone();
        let space_focus = focus_handle.clone();
        root = root
            .capture_key_down(move |event: &KeyDownEvent, _, cx| {
                if event.keystroke.key != "home" {
                    return;
                }
                if home_owner.upgrade().is_some_and(|controller| controller.jump_to_feed_top_by_remount(cx)) {
                    cx.stop_propagation();
                }
            })
            .track_focus(&focus_handle)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key != "space" || !space_focus.is_focused(window) {
                    return;
                }
                let Some(controller) = space_owner.upgrade() else {
                    return;
                };
                controller.page_by_space(event.keystroke.modifiers.shift, window, cx);
                cx.stop_propagation();
            })
            .child(layout_reader)
            .child(controller.measurer.display_link())
            .child(controller.list_view.clone().cached(StyleRefinement::default().size_full()))
            .child(
                controller
                    .load_more_footer
                    .clone()
                    .cached(StyleRefinement::default().absolute().top_0().left_0().size_full()),
            );
        if let Some(indicator) = &controller.refresh_indicator {
            root = root.child(indicator.clone());
        }
        if !controller.is_stopped.get()
            && let Some(destination) = controller.destination()
        {
            root = root.child(SharedElementHostProbe::destination(destination as Rc<dyn SharedElementDestinationContainer>));
        }
        let leaving_owner = self.controller.clone();
        root.on_disappear(("post-list-controller-view", cx.entity_id()), move |window, cx| {
            if let Some(controller) = leaving_owner.upgrade() {
                match window {
                    Some(window) => controller.did_move_to_window(false, window, cx),
                    None => controller.window_did_close(cx),
                }
            }
        })
        .into_any_element()
    }
}

struct PostListRowPlaceholderView {
    background: Hsla,
    separator: Hsla,
}

impl Render for PostListRowPlaceholderView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().relative().size_full().bg(self.background).child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(PostRowLayout::SEPARATOR_THICKNESS))
                .bg(self.separator),
        )
    }
}

fn x251_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X251", "{}", stage());
}
