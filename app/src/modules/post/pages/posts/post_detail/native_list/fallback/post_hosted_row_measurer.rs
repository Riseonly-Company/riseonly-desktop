use std::cell::{Cell, RefCell};
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::rc::{Rc, Weak};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gpui::{
    AnyView, AnyWindowHandle, App, AppContext, AvailableSpace, Canvas, Entity, EntityId, IntoElement, Size, Styled, Task, Window, canvas, px,
};

use rise_core::foundation::foundation_date::RiseClock;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::chat::components::chat::async_render::chat_performance_trace::ChatPerformanceTrace;
use crate::modules::chat::components::chat::async_render::rise_manual_message_hosting_view::RiseHostedRowAuditPolicy;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListCachePurge, PostListCachePurgeRegistration, PostListHostedEnvironment, PostRowBodyKind, PostRowLayoutSpec,
    PreparedPostRow, PreparedPostRowKey,
};
use crate::modules::post::pages::posts::post_detail::native_list::fallback::post_hosted_row_height_cache::PostHostedRowHeightCache;
use crate::modules::post::pages::posts::post_detail::native_list::fallback::post_hosted_row_view::{
    PostHostedRowView, PostHostedRowViewRole,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_preparer::PostRowHostedHeightLookup;

/// pacing and tolerances for hosted row measuring
pub enum PostHostedRowMeasurePolicy {}

impl PostHostedRowMeasurePolicy {
    pub const MAXIMUM_QUEUED_ROWS: usize = 96;
    pub const MAXIMUM_PARITY_QUEUE: usize = 64;
    pub const MAXIMUM_AUDITED_PARITY_KEYS: usize = 1_024;
    pub const BLOCKED_RETRY_DELAY: f64 = RiseHostedRowAuditPolicy::SCROLLING_RETRY_DELAY;
    pub const STALLED_RETRY_DELAY: f64 = 0.5;
    pub const MAXIMUM_STALLED_RETRIES: u32 = 6;
    pub const SCRATCH_INITIAL_HEIGHT: f32 = 64.0;
    pub const PARITY_REPORT_THRESHOLD: f32 = 0.5;

    pub fn height_tolerance(display_scale: f32) -> f32 {
        let scale = if display_scale.is_finite() { display_scale.max(1.0) } else { 1.0 };
        0.5_f32.max(1.0 / scale)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PostHostedRowMeasurerResolution {
    pub presentation_id: String,
    pub key: PreparedPostRowKey,
    pub row_height: f32,
    pub committed_row_height: f32,
    pub changes_committed_height: bool,
}

#[derive(Clone)]
struct PostHostedRowMeasurerRequest {
    prepared: Arc<PreparedPostRow>,
    committed_row_height: f32,
}

impl PostHostedRowMeasurerRequest {
    fn new(prepared: &Arc<PreparedPostRow>) -> Self {
        Self { prepared: prepared.clone(), committed_row_height: prepared.height() }
    }

    fn presentation_id(&self) -> &str {
        &self.prepared.key.presentation_id
    }
}

type PostHostedRowCanMeasure = Rc<dyn Fn(&mut App) -> bool>;
type PostHostedRowVisibleIds = Rc<dyn Fn(&App) -> Vec<String>>;
type PostHostedRowMountedRow = Rc<dyn Fn(&str, &App) -> Option<Entity<PostHostedRowView>>>;
type PostHostedRowHostedEnvironment = Rc<dyn Fn(&App) -> Option<PostListHostedEnvironment>>;
type PostHostedRowHeightsResolved = Rc<dyn Fn(&[PostHostedRowMeasurerResolution], &mut App)>;

/// measures whole row hosted fallback heights off the visible path
/// at most one measure per frame and only while can_measure holds
pub struct PostHostedRowMeasurer {
    this: Weak<Self>,
    pub session: u64,
    window_handle: AnyWindowHandle,

    can_measure: RefCell<PostHostedRowCanMeasure>,
    visible_ids: RefCell<PostHostedRowVisibleIds>,
    mounted_row: RefCell<PostHostedRowMountedRow>,
    hosted_environment: RefCell<PostHostedRowHostedEnvironment>,
    on_heights_resolved: RefCell<Option<PostHostedRowHeightsResolved>>,

    hosted_queue: RefCell<Vec<PostHostedRowMeasurerRequest>>,
    parity_queue: RefCell<Vec<PostHostedRowMeasurerRequest>>,
    audited_parity_keys: RefCell<HashSet<PreparedPostRowKey>>,
    scratch_host: RefCell<Option<Entity<PostHostedRowView>>>,
    scratch_has_content: Cell<bool>,
    link: Cell<bool>,
    link_host: Cell<Option<EntityId>>,
    last_link_fire: Cell<Option<Duration>>,
    retry_work_item: RefCell<Option<Task<()>>>,
    stalled_retries: Cell<u32>,
    pending_resolutions: RefCell<HashMap<String, PostHostedRowMeasurerResolution>>,
    pending_resolution_order: RefCell<Vec<String>>,
    is_resolution_flush_scheduled: Cell<bool>,
    purge_registration: RefCell<Option<PostListCachePurgeRegistration>>,
    is_stopped: Cell<bool>,
}

impl PostHostedRowMeasurer {
    const LINK_MINIMUM_INTERVAL: Duration = Duration::from_micros(16_000);

    pub fn new(session: u64, window_handle: AnyWindowHandle, cx: &mut App) -> Rc<Self> {
        PostHostedRowHeightCache::register_purge_if_needed(cx);
        let measurer = Rc::new_cyclic(|this: &Weak<Self>| Self {
            this: this.clone(),
            session,
            window_handle,
            can_measure: RefCell::new(Rc::new(|_: &mut App| false)),
            visible_ids: RefCell::new(Rc::new(|_: &App| Vec::new())),
            mounted_row: RefCell::new(Rc::new(|_: &str, _: &App| None)),
            hosted_environment: RefCell::new(Rc::new(|_: &App| None)),
            on_heights_resolved: RefCell::new(None),
            hosted_queue: RefCell::new(Vec::new()),
            parity_queue: RefCell::new(Vec::new()),
            audited_parity_keys: RefCell::new(HashSet::new()),
            scratch_host: RefCell::new(None),
            scratch_has_content: Cell::new(false),
            link: Cell::new(false),
            link_host: Cell::new(None),
            last_link_fire: Cell::new(None),
            retry_work_item: RefCell::new(None),
            stalled_retries: Cell::new(0),
            pending_resolutions: RefCell::new(HashMap::new()),
            pending_resolution_order: RefCell::new(Vec::new()),
            is_resolution_flush_scheduled: Cell::new(false),
            purge_registration: RefCell::new(None),
            is_stopped: Cell::new(false),
        });
        let this = Rc::downgrade(&measurer);
        let registration = PostListCachePurge::register(
            move |cx: &mut App| {
                if let Some(measurer) = this.upgrade() {
                    measurer.purge(cx);
                }
            },
            cx,
        );
        *measurer.purge_registration.borrow_mut() = Some(registration);
        measurer
    }

    /// idle gate, no tracking
    /// no deceleration
    /// no transition, not dismissing
    pub fn set_can_measure(&self, can_measure: impl Fn(&mut App) -> bool + 'static) {
        *self.can_measure.borrow_mut() = Rc::new(can_measure);
    }

    pub fn set_visible_ids(&self, visible_ids: impl Fn(&App) -> Vec<String> + 'static) {
        *self.visible_ids.borrow_mut() = Rc::new(visible_ids);
    }

    /// mounted configured hosted row for an engine id
    pub fn set_mounted_row(
        &self,
        mounted_row: impl Fn(&str, &App) -> Option<Entity<PostHostedRowView>> + 'static,
    ) {
        *self.mounted_row.borrow_mut() = Rc::new(mounted_row);
    }

    /// hosted environment of the list, the scratch host drops its shared element surface
    pub fn set_hosted_environment(
        &self,
        hosted_environment: impl Fn(&App) -> Option<PostListHostedEnvironment> + 'static,
    ) {
        *self.hosted_environment.borrow_mut() = Rc::new(hosted_environment);
    }

    pub fn set_on_heights_resolved(
        &self,
        on_heights_resolved: Option<PostHostedRowHeightsResolved>,
    ) {
        *self.on_heights_resolved.borrow_mut() = on_heights_resolved;
    }

    /// pass this to the row preparer as the hosted height lookup
    pub fn hosted_height_lookup(&self) -> PostRowHostedHeightLookup {
        PostHostedRowHeightCache::shared().lookup()
    }

    /// hosted rows the next commits need
    /// nearest first, non hosted rows are ignored
    /// rows with an exact cached height resolve right away, the rest wait for idle frames
    pub fn submit(&self, rows: &[Arc<PreparedPostRow>], cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        let cache = PostHostedRowHeightCache::shared();
        let mut did_queue = false;
        for row in rows {
            if !row.route.is_hosted_fallback() {
                continue;
            }
            cache.register_hosted_content(&row.post);
            let committed_row_height = row.height();
            if let Some(exact) = cache.exact_height(&row.key) {
                self.remove_hosted_request(&row.key.presentation_id);
                if row.is_height_estimated || !Self::matches(exact, committed_row_height, &row.key.spec) {
                    self.enqueue_resolution(row.key.clone(), exact, committed_row_height, cx);
                }
                continue;
            }
            self.upsert_hosted_request(PostHostedRowMeasurerRequest::new(row));
            did_queue = true;
        }
        self.hosted_queue.borrow_mut().truncate(PostHostedRowMeasurePolicy::MAXIMUM_QUEUED_ROWS);
        if did_queue {
            self.set_needs_pump(cx);
        }
    }

    pub fn note_mounted_estimated_row(&self, prepared: &Arc<PreparedPostRow>, cx: &mut App) {
        if self.is_stopped.get() || !prepared.route.is_hosted_fallback() {
            return;
        }
        let is_queued = self
            .hosted_queue
            .borrow()
            .iter()
            .any(|request| request.presentation_id() == prepared.key.presentation_id && request.prepared.key == prepared.key);
        if is_queued {
            self.set_needs_pump(cx);
            return;
        }
        self.submit(std::slice::from_ref(prepared), cx);
    }

    pub fn retain_requests(&self, presentation_ids: &HashSet<String>, cx: &mut App) {
        self.hosted_queue.borrow_mut().retain(|request| presentation_ids.contains(request.presentation_id()));
        self.parity_queue.borrow_mut().retain(|request| presentation_ids.contains(request.presentation_id()));
        if !self.has_work() {
            self.stop_pumping(cx);
        }
    }

    /// restarts pacing now, call when the list goes idle
    /// a transition ends or visible ids change
    pub fn set_needs_pump(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        self.retry_work_item.borrow_mut().take();
        self.stalled_retries.set(0);
        if !self.has_work() {
            self.stop_pumping(cx);
            return;
        }
        self.start_link_if_needed(cx);
    }

    /// teardown, stops pacing
    /// frees the scratch graph and unregisters the purge
    pub fn stop(&self, cx: &mut App) {
        if self.is_stopped.get() {
            return;
        }
        self.is_stopped.set(true);
        self.stop_link();
        self.retry_work_item.borrow_mut().take();
        self.hosted_queue.borrow_mut().clear();
        self.parity_queue.borrow_mut().clear();
        self.audited_parity_keys.borrow_mut().clear();
        self.pending_resolutions.borrow_mut().clear();
        self.pending_resolution_order.borrow_mut().clear();
        *self.on_heights_resolved.borrow_mut() = None;
        self.set_can_measure(|_| false);
        self.set_visible_ids(|_| Vec::new());
        self.set_mounted_row(|_, _| None);
        self.set_hosted_environment(|_| None);
        if let Some(registration) = self.purge_registration.borrow_mut().take() {
            PostListCachePurge::unregister(registration);
        }
        let scratch_host = self.scratch_host.borrow_mut().take();
        if let Some(scratch_host) = scratch_host {
            scratch_host.update(cx, |host, cx| host.release_measurement_content(cx));
        }
        self.scratch_has_content.set(false);
    }

    pub fn live_row_did_measure(
        &self,
        key: &PreparedPostRowKey,
        row_height: f32,
        committed_row_height: f32,
        cx: &mut App,
    ) {
        if self.is_stopped.get() {
            return;
        }
        {
            let mut hosted_queue = self.hosted_queue.borrow_mut();
            if let Some(index) = hosted_queue.iter().position(|request| request.prepared.key == *key) {
                hosted_queue.remove(index);
            }
        }
        self.enqueue_resolution(key.clone(), row_height, committed_row_height, cx);
    }

    /// true only in debug builds with the parity audit flag
    pub fn is_parity_audit_enabled() -> bool {
        static IS_ENABLED: OnceLock<bool> = OnceLock::new();
        *IS_ENABLED.get_or_init(|| {
            cfg!(debug_assertions) && rise_platform::process_info::arguments().into_iter().any(|argument| argument == "-risePostListParityAudit")
        })
    }

    pub fn submit_parity_audit(&self, rows: &[Arc<PreparedPostRow>], cx: &mut App) {
        if !Self::is_parity_audit_enabled() || self.is_stopped.get() {
            return;
        }
        let mut did_queue = false;
        for row in rows {
            if row.layout.is_none()
                || row.is_height_estimated
                || row.seed.body == PostRowBodyKind::Reel
                || self.audited_parity_keys.borrow().contains(&row.key)
                || self.parity_queue.borrow().iter().any(|request| request.prepared.key == row.key)
            {
                continue;
            }
            self.parity_queue.borrow_mut().push(PostHostedRowMeasurerRequest::new(row));
            did_queue = true;
        }
        {
            let mut parity_queue = self.parity_queue.borrow_mut();
            let overflow = parity_queue.len().saturating_sub(PostHostedRowMeasurePolicy::MAXIMUM_PARITY_QUEUE);
            parity_queue.drain(..overflow);
        }
        if did_queue {
            self.set_needs_pump(cx);
        }
    }

    /// one sizing pass rounded up to whole pixels
    /// only works inside a frame, gpui has no layout pass outside one
    pub fn natural_content_height(
        hosted_view: &AnyView,
        width: f32,
        display_scale: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<f32> {
        if !width.is_finite() || width <= 1.0 {
            return None;
        }
        let mut element = hosted_view.clone().into_any_element();
        let raw_height = f32::from(
            element
                .layout_as_root(
                    Size { width: AvailableSpace::Definite(px(width)), height: AvailableSpace::MinContent },
                    window,
                    cx,
                )
                .height,
        );
        Self::pixel_ceiled_height(raw_height, display_scale)
    }

    /// usable height rule of natural_content_height, shared with the live row probe
    pub(crate) fn pixel_ceiled_height(raw_height: f32, display_scale: f32) -> Option<f32> {
        if !raw_height.is_finite() || raw_height <= 0.0 || raw_height >= 100_000.0 {
            return None;
        }
        let scale = if display_scale.is_finite() { display_scale.max(1.0) } else { 1.0 };
        Some((raw_height * scale).ceil() / scale)
    }

    /// place this once in the list tree
    /// it paces in the frame prepaint, the only place the scratch graph can be laid out
    /// and asks for the next frame while running
    pub fn display_link(&self) -> Canvas<()> {
        let this = self.this.clone();
        canvas(
            move |_bounds, window, cx| {
                if let Some(measurer) = this.upgrade() {
                    measurer.link_host.set(Some(window.current_view()));
                    measurer.display_link_tick(window, cx);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_0()
    }

    fn display_link_tick(&self, window: &mut Window, cx: &mut App) {
        if !self.link.get() {
            return;
        }
        let now = RiseClock::monotonic();
        let is_paced_out = self
            .last_link_fire
            .get()
            .is_some_and(|last_fire| now.saturating_sub(last_fire) < Self::LINK_MINIMUM_INTERVAL);
        if !is_paced_out {
            self.last_link_fire.set(Some(now));
            self.display_link_did_fire(window, cx);
        }
        if self.link.get() {
            window.request_animation_frame();
        }
    }

    fn display_link_did_fire(&self, window: &mut Window, cx: &mut App) {
        if self.is_stopped.get() {
            self.stop_link();
            return;
        }
        if !self.has_work() {
            self.stop_pumping(cx);
            return;
        }
        let can_measure = self.can_measure.borrow().clone();
        if !can_measure(cx) {
            self.stop_link();
            self.schedule_retry(PostHostedRowMeasurePolicy::BLOCKED_RETRY_DELAY, false, cx);
            return;
        }
        let visible_ids = self.visible_ids.borrow().clone();
        let visible: HashSet<String> = visible_ids(cx).into_iter().collect();
        if self.measure_next_hosted(&visible, window, cx) || self.measure_next_parity(&visible, window, cx) {
            self.stalled_retries.set(0);
            return;
        }
        self.stop_link();
        if self.hosted_queue.borrow().is_empty() && self.parity_queue.borrow().is_empty() {
            self.release_scratch_content(cx);
        } else {
            self.schedule_retry(PostHostedRowMeasurePolicy::STALLED_RETRY_DELAY, true, cx);
        }
    }

    fn measure_next_hosted(&self, visible: &HashSet<String>, window: &mut Window, cx: &mut App) -> bool {
        let cache = PostHostedRowHeightCache::shared();
        let mounted_row = self.mounted_row.borrow().clone();
        let hosted_environment = self.hosted_environment.borrow().clone();
        let mut index = 0;
        loop {
            let Some(request) = self.hosted_queue.borrow().get(index).cloned() else {
                return false;
            };
            let key = &request.prepared.key;
            if let Some(exact) = cache.exact_height(key) {
                self.hosted_queue.borrow_mut().remove(index);
                self.enqueue_resolution(key.clone(), exact, request.committed_row_height, cx);
                continue;
            }
            if let Some(row) = mounted_row(request.presentation_id(), cx)
                && row.read(cx).can_audit_live(key)
            {
                if row.update(cx, |row, cx| row.audit_natural_row_height(cx)).is_some() {
                    return true;
                }
                index += 1;
                continue;
            }
            let environment = if visible.contains(request.presentation_id())
                || request.prepared.seed.body == PostRowBodyKind::Reel
            {
                None
            } else {
                hosted_environment(cx)
            };
            let Some(environment) = environment else {
                index += 1;
                continue;
            };
            self.hosted_queue.borrow_mut().remove(index);
            let Some(row_height) = self.scratch_measure(&request, &environment, "hosted", window, cx) else {
                return true;
            };
            cache.insert(row_height, key, &request.prepared.post);
            self.enqueue_resolution(key.clone(), row_height, request.committed_row_height, cx);
            return true;
        }
    }

    fn measure_next_parity(&self, visible: &HashSet<String>, window: &mut Window, cx: &mut App) -> bool {
        if !Self::is_parity_audit_enabled() {
            return false;
        }
        let Some(index) =
            self.parity_queue.borrow().iter().position(|request| !visible.contains(request.presentation_id()))
        else {
            return false;
        };
        let hosted_environment = self.hosted_environment.borrow().clone();
        let Some(environment) = hosted_environment(cx) else {
            return false;
        };
        let request = self.parity_queue.borrow_mut().remove(index);
        self.remember_parity_key(&request.prepared.key);
        let Some(row_height) = self.scratch_measure(&request, &environment, "parity", window, cx) else {
            return true;
        };
        self.report_parity(&request, row_height);
        true
    }

    fn scratch_measure(
        &self,
        request: &PostHostedRowMeasurerRequest,
        environment: &PostListHostedEnvironment,
        purpose: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<f32> {
        let spec = &request.prepared.key.spec;
        let width = spec.width;
        if !width.is_finite() || width <= 1.0 {
            return None;
        }
        let existing_host = self.scratch_host.borrow().clone();
        let trace_token = ChatPerformanceTrace::shared().begin_operation(
            "post.fallback.measure",
            HashMap::from([
                ("surface".to_owned(), "post_list".to_owned()),
                ("purpose".to_owned(), purpose.to_owned()),
                ("cold".to_owned(), if existing_host.is_none() { "1" } else { "0" }.to_owned()),
            ]),
        );
        let host = match existing_host {
            Some(host) => host,
            None => {
                let host = cx.new(|cx| PostHostedRowView::new(PostHostedRowViewRole::Measurement, cx));
                *self.scratch_host.borrow_mut() = Some(host.clone());
                host
            }
        };
        let measurement_environment = Self::measurement_environment(environment);
        let hosted_content_view = host.update(cx, |host, cx| {
            host.configure_for_measurement(&request.prepared.post, &measurement_environment, cx);
            host.prepare_measurement_frame(width);
            host.hosted_content_view()
        });
        self.scratch_has_content.set(true);
        let natural = hosted_content_view.and_then(|hosted_content_view| {
            Self::natural_content_height(&hosted_content_view, width, spec.display_scale, window, cx)
        });
        ChatPerformanceTrace::shared().end_operation(
            trace_token,
            1.0,
            HashMap::from([("result".to_owned(), if natural.is_none() { "none" } else { "ok" }.to_owned())]),
        );
        natural.map(PostHostedRowView::row_height_for_content)
    }

    /// without a surface the scratch graph registers no shared element anchors over the live rows
    fn measurement_environment(environment: &PostListHostedEnvironment) -> PostListHostedEnvironment {
        PostListHostedEnvironment {
            repost_transition_namespace: None,
            shorts_zoom_namespace: None,
            shared_element_surface: None,
            ..environment.clone()
        }
    }

    fn has_work(&self) -> bool {
        !self.hosted_queue.borrow().is_empty()
            || (Self::is_parity_audit_enabled() && !self.parity_queue.borrow().is_empty())
    }

    fn start_link_if_needed(&self, cx: &mut App) {
        if self.link.get() {
            return;
        }
        self.link.set(true);
        let this = self.this.clone();
        let window_handle = self.window_handle;
        let link_host = self.link_host.get();
        cx.defer(move |cx| {
            if let Some(link_host) = link_host {
                cx.notify(link_host);
                return;
            }
            let refreshed = window_handle.update(cx, |_, window, _| window.refresh());
            if refreshed.is_err()
                && let Some(measurer) = this.upgrade()
            {
                measurer.stop_link();
            }
        });
    }

    fn stop_link(&self) {
        self.link.set(false);
    }

    fn stop_pumping(&self, cx: &mut App) {
        self.stop_link();
        self.retry_work_item.borrow_mut().take();
        self.release_scratch_content(cx);
    }

    fn schedule_retry(&self, delay: f64, is_stall: bool, cx: &mut App) {
        if is_stall {
            self.stalled_retries.set(self.stalled_retries.get() + 1);
            if self.stalled_retries.get() > PostHostedRowMeasurePolicy::MAXIMUM_STALLED_RETRIES {
                self.release_scratch_content(cx);
                return;
            }
        }
        let this = self.this.clone();
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            cx.update(|cx| {
                let Some(measurer) = this.upgrade() else {
                    return;
                };
                if measurer.is_stopped.get() || !measurer.has_work() {
                    return;
                }
                measurer.start_link_if_needed(cx);
            });
        });
        *self.retry_work_item.borrow_mut() = Some(task);
    }

    fn release_scratch_content(&self, cx: &mut App) {
        if !self.scratch_has_content.get() {
            return;
        }
        self.scratch_has_content.set(false);
        let scratch_host = self.scratch_host.borrow().clone();
        if let Some(scratch_host) = scratch_host {
            scratch_host.update(cx, |host, cx| host.release_measurement_content(cx));
        }
    }

    fn upsert_hosted_request(&self, request: PostHostedRowMeasurerRequest) {
        let mut hosted_queue = self.hosted_queue.borrow_mut();
        match hosted_queue.iter().position(|queued| queued.presentation_id() == request.presentation_id()) {
            Some(index) => hosted_queue[index] = request,
            None => hosted_queue.push(request),
        }
    }

    fn remove_hosted_request(&self, presentation_id: &str) {
        self.hosted_queue.borrow_mut().retain(|request| request.presentation_id() != presentation_id);
    }

    fn remember_parity_key(&self, key: &PreparedPostRowKey) {
        let mut audited_parity_keys = self.audited_parity_keys.borrow_mut();
        if audited_parity_keys.len() >= PostHostedRowMeasurePolicy::MAXIMUM_AUDITED_PARITY_KEYS {
            audited_parity_keys.clear();
        }
        audited_parity_keys.insert(key.clone());
    }

    fn purge(&self, cx: &mut App) {
        let user_id = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let is_current =
            |key: &PreparedPostRowKey| key.account_user_id == user_id && key.account_epoch == epoch;
        self.hosted_queue.borrow_mut().retain(|request| is_current(&request.prepared.key));
        self.parity_queue.borrow_mut().retain(|request| is_current(&request.prepared.key));
        self.audited_parity_keys.borrow_mut().clear();
        {
            let mut pending_resolutions = self.pending_resolutions.borrow_mut();
            pending_resolutions.retain(|_, resolution| is_current(&resolution.key));
            self.pending_resolution_order.borrow_mut().retain(|id| pending_resolutions.contains_key(id));
        }
        self.stop_link();
        self.release_scratch_content(cx);
        if self.has_work() {
            self.schedule_retry(PostHostedRowMeasurePolicy::BLOCKED_RETRY_DELAY, false, cx);
        }
    }

    fn enqueue_resolution(&self, key: PreparedPostRowKey, row_height: f32, committed_row_height: f32, cx: &mut App) {
        let presentation_id = key.presentation_id.clone();
        let changes_committed_height = !Self::matches(row_height, committed_row_height, &key.spec);
        let resolution = PostHostedRowMeasurerResolution {
            presentation_id: presentation_id.clone(),
            key,
            row_height,
            committed_row_height,
            changes_committed_height,
        };
        let replaced = self.pending_resolutions.borrow_mut().insert(presentation_id.clone(), resolution);
        if replaced.is_none() {
            self.pending_resolution_order.borrow_mut().push(presentation_id);
        }
        if self.is_resolution_flush_scheduled.get() {
            return;
        }
        self.is_resolution_flush_scheduled.set(true);
        let this = self.this.clone();
        cx.defer(move |cx| {
            if let Some(measurer) = this.upgrade() {
                measurer.flush_resolutions(cx);
            }
        });
    }

    fn flush_resolutions(&self, cx: &mut App) {
        self.is_resolution_flush_scheduled.set(false);
        if self.is_stopped.get() || self.pending_resolution_order.borrow().is_empty() {
            return;
        }
        let order = std::mem::take(&mut *self.pending_resolution_order.borrow_mut());
        let mut pending_resolutions = std::mem::take(&mut *self.pending_resolutions.borrow_mut());
        let resolutions: Vec<PostHostedRowMeasurerResolution> =
            order.iter().filter_map(|id| pending_resolutions.remove(id)).collect();
        if resolutions.is_empty() {
            return;
        }
        let on_heights_resolved = self.on_heights_resolved.borrow().clone();
        if let Some(on_heights_resolved) = on_heights_resolved {
            on_heights_resolved(&resolutions, cx);
        }
    }

    fn matches(lhs: f32, rhs: f32, spec: &PostRowLayoutSpec) -> bool {
        (lhs - rhs).abs() <= PostHostedRowMeasurePolicy::height_tolerance(spec.display_scale)
    }

    fn report_parity(&self, request: &PostHostedRowMeasurerRequest, hosted_row_height: f32) {
        let Some(layout) = request.prepared.layout.as_ref() else {
            return;
        };
        let native_height = layout.total_height;
        let delta = hosted_row_height - native_height;
        if delta.abs() <= PostHostedRowMeasurePolicy::PARITY_REPORT_THRESHOLD {
            return;
        }
        let spec = &request.prepared.key.spec;
        fn points(value: Option<f32>) -> String {
            value.map_or_else(|| "-".to_owned(), |value| format!("{value:.2}"))
        }
        let inputs = &layout.inputs;
        tracing::debug!(
            target: "PostListParity",
            row = %Self::fingerprint(request.presentation_id()),
            session = %Self::fingerprint(&self.session.to_string()),
            route = PostHostedRowView::route_name(&request.prepared.route),
            delta = delta,
            native = native_height,
            hosted = hosted_row_height,
            header = %points(Some(f32::from(layout.header.frame.size.height))),
            banner = %points(layout.moderation_banner.as_ref().map(|part| f32::from(part.frame.size.height))),
            strip = %points(layout.music_strip.as_ref().map(|part| f32::from(part.frame.size.height))),
            media = %points(layout.media_box.as_ref().map(|part| f32::from(part.frame.size.height))),
            caption = %points(layout.caption.as_ref().map(|part| f32::from(part.frame.size.height))),
            actions = %points(Some(f32::from(layout.actions.frame.size.height))),
            likes = %points(layout.likes_summary.as_ref().map(|part| f32::from(part.frame.size.height))),
            badges = %format!("{}{}", u8::from(inputs.has_official_badge), u8::from(inputs.has_premium_badge)),
            track = u8::from(inputs.has_header_track_line),
            hashtags = inputs.hashtags.len(),
            width = spec.width,
            scale = spec.display_scale,
            text_delta = spec.text_delta,
            bold = spec.is_bold_text_enabled,
            "post_list.parity"
        );
    }

    /// salted per process, never the raw id
    fn fingerprint(value: &str) -> String {
        static SALT: OnceLock<RandomState> = OnceLock::new();
        let hash = SALT.get_or_init(RandomState::new).hash_one(("post-list-parity", value));
        format!("{hash:x}")
    }
}
