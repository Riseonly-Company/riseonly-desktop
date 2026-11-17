use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{AnyView, AnyWeakView, AnyWindowHandle, App, Context, Entity, Global, WeakEntity, Window};

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::{
    RiseManualListMountEnvelopePolicy, RiseManualListView,
};
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::PostListReusableRow;
use crate::modules::post::pages::posts::post_detail::native_list::post_list_performance_trace::{
    PostListPerformanceTrace, PostListTraceSpan,
};

/// a mounted row is an AnyView so a row type registers itself in its new, idempotently
#[derive(Default)]
pub struct PostListReusableRowConformances {
    by_type: HashMap<TypeId, PostListReusableRowConformance>,
}

impl Global for PostListReusableRowConformances {}

#[derive(Clone, Copy)]
struct PostListReusableRowConformance {
    configured_presentation_id: fn(&AnyView, &App) -> Option<String>,
    set_hosted_leaves_active: fn(&AnyView, bool, &mut App),
}

impl PostListReusableRowConformances {
    pub fn register<T: PostListReusableRow>(cx: &mut App) {
        fn configured_presentation_id<T: PostListReusableRow>(view: &AnyView, cx: &App) -> Option<String> {
            let entity = view.clone().downcast::<T>().ok()?;
            entity.read(cx).configured_presentation_id().map(str::to_owned)
        }
        fn set_hosted_leaves_active<T: PostListReusableRow>(view: &AnyView, is_active: bool, cx: &mut App) {
            if let Ok(entity) = view.clone().downcast::<T>() {
                entity.update(cx, |row, cx| row.set_hosted_leaves_active(is_active, cx));
            }
        }
        cx.default_global::<Self>().by_type.insert(
            TypeId::of::<T>(),
            PostListReusableRowConformance {
                configured_presentation_id: configured_presentation_id::<T>,
                set_hosted_leaves_active: set_hosted_leaves_active::<T>,
            },
        );
    }

    fn lookup(view: &AnyView, cx: &App) -> Option<PostListReusableRowConformance> {
        cx.try_global::<Self>()?.by_type.get(&view.entity_type()).copied()
    }

    /// presentation id of a reusable row, none for a view that is not one
    fn configured_presentation_id(view: &AnyView, cx: &App) -> Option<String> {
        (Self::lookup(view, cx)?.configured_presentation_id)(view, cx)
    }

    fn set_hosted_leaves_active(view: &AnyView, is_active: bool, cx: &mut App) {
        if let Some(conformance) = Self::lookup(view, cx) {
            (conformance.set_hosted_leaves_active)(view, is_active, cx);
        }
    }
}

/// mounts and unmounts hosted leaves of visible rows outside the scroll callback
/// at most one leaf operation per display frame
/// mounts wait while velocity is above the fast scroll threshold
pub struct PostListLeafMountQueue {
    pub needs_leaves: Option<Rc<dyn Fn(&str, &App) -> bool>>,

    list_view: WeakEntity<RiseManualListView>,
    window_handle: AnyWindowHandle,
    trace: Option<Rc<PostListPerformanceTrace>>,
    is_display_link_scheduled: bool,
    is_display_link_paused: bool,

    visible: HashSet<String>,
    desired: HashSet<String>,
    active: HashMap<String, AnyWeakView>,
    pending: Vec<String>,
    pending_set: HashSet<String>,
    is_stopped: bool,
}

impl PostListLeafMountQueue {
    pub fn new(
        list_view: &Entity<RiseManualListView>,
        trace: Option<Rc<PostListPerformanceTrace>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            needs_leaves: None,
            list_view: list_view.downgrade(),
            window_handle: list_view.read(cx).window_handle(),
            trace,
            is_display_link_scheduled: false,
            is_display_link_paused: false,
            visible: HashSet::new(),
            desired: HashSet::new(),
            active: HashMap::new(),
            pending: Vec::new(),
            pending_set: HashSet::new(),
            is_stopped: false,
        }
    }

    pub fn active_presentation_ids(&self) -> HashSet<String> {
        self.active.keys().cloned().collect()
    }

    pub fn has_pending_work(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn visible_ids_did_change(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        let next: HashSet<String> = ids.iter().cloned().collect();
        let left: Vec<String> = self.visible.difference(&next).cloned().collect();
        self.visible = next;
        for id in &left {
            let was_desired = self.desired.remove(id);
            if was_desired || self.active.contains_key(id) {
                self.enqueue(id, cx);
            }
        }
        for id in ids {
            if self.desired.contains(id) || !self.row_needs_leaves(id, cx) {
                continue;
            }
            self.desired.insert(id.clone());
            self.enqueue(id, cx);
        }
    }

    pub fn rows_did_change(&mut self, ids: &HashSet<String>, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        for id in ids {
            if !self.visible.contains(id) {
                continue;
            }
            if self.row_needs_leaves(id, cx) {
                self.desired.insert(id.clone());
            } else {
                self.desired.remove(id);
            }
            self.enqueue(id, cx);
        }
    }

    pub fn row_did_configure(&mut self, presentation_id: &str, cx: &mut Context<Self>) {
        self.rows_did_change(&HashSet::from([presentation_id.to_owned()]), cx);
    }

    /// resumes a queue paused while the list was off window
    pub fn resume(&mut self, cx: &mut Context<Self>) {
        if self.is_stopped || self.pending.is_empty() {
            return;
        }
        self.start_display_link_if_needed(cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        self.is_stopped = true;
        self.is_display_link_paused = true;
        self.pending.clear();
        self.pending_set.clear();
        let list_view = self.list_view.upgrade();
        for (id, reference) in std::mem::take(&mut self.active) {
            let Some(view) = reference.upgrade() else {
                continue;
            };
            if PostListReusableRowConformances::configured_presentation_id(&view, cx).as_deref() != Some(id.as_str()) {
                continue;
            }
            if let Some(list_view) = &list_view {
                let mounted = list_view.read(cx).mounted_view(&id);
                if mounted.map(|mounted| mounted.entity_id()) != Some(view.entity_id()) {
                    continue;
                }
            }
            PostListReusableRowConformances::set_hosted_leaves_active(&view, false, cx);
        }
        self.desired.clear();
        self.visible.clear();
        self.needs_leaves = None;
    }

    fn row_needs_leaves(&self, id: &str, cx: &App) -> bool {
        self.needs_leaves.as_ref().is_none_or(|needs_leaves| needs_leaves(id, cx))
    }

    fn tick(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.is_display_link_scheduled = false;
        if self.is_display_link_paused {
            return;
        }
        self.drain_one(cx);
        if !self.is_stopped && !self.is_display_link_paused && !self.pending.is_empty() {
            self.is_display_link_scheduled = true;
            Self::schedule_tick(cx.weak_entity(), window);
        }
    }

    fn drain_one(&mut self, cx: &mut Context<Self>) {
        if self.is_stopped {
            self.pause();
            return;
        }
        let Some(list_view) = self.list_view.upgrade().filter(|list_view| list_view.read(cx).is_in_window()) else {
            self.pause();
            return;
        };
        let mut index = 0;
        while index < self.pending.len() {
            let id = self.pending[index].clone();
            let mounted = Self::mounted_row(&id, &list_view, cx);
            if self.desired.contains(&id) {
                let Some(mounted) = mounted else {
                    self.remove_pending(index);
                    self.active.remove(&id);
                    continue;
                };
                let active_view = self.active.get(&id).and_then(AnyWeakView::upgrade);
                if active_view.map(|view| view.entity_id()) == Some(mounted.entity_id()) {
                    self.remove_pending(index);
                    continue;
                }
                if !Self::can_mount(&list_view, cx) {
                    index += 1;
                    continue;
                }
                self.remove_pending(index);
                self.set_leaves_active(&mounted, true, cx);
                self.active.insert(id, mounted.downgrade());
                return;
            }

            self.remove_pending(index);
            let Some(reference) = self.active.remove(&id) else {
                continue;
            };
            let Some(mounted) = mounted else {
                continue;
            };
            if reference.upgrade().map(|view| view.entity_id()) != Some(mounted.entity_id()) {
                continue;
            }
            self.set_leaves_active(&mounted, false, cx);
            return;
        }
        if self.pending.is_empty() {
            self.pause();
        }
    }

    fn set_leaves_active(&self, row: &AnyView, is_active: bool, cx: &mut Context<Self>) {
        let trace = self.trace.as_deref();
        let span = trace.and_then(|trace| {
            trace.begin(PostListTraceSpan::LeafMount, || {
                HashMap::from([("op".to_owned(), if is_active { "mount" } else { "unmount" }.to_owned())])
            })
        });
        PostListReusableRowConformances::set_hosted_leaves_active(row, is_active, cx);
        if let Some(trace) = trace {
            trace.end(span, 0.5, HashMap::new);
        }
    }

    fn mounted_row(id: &str, list_view: &Entity<RiseManualListView>, cx: &App) -> Option<AnyView> {
        let view = list_view.read(cx).mounted_view(id)?;
        let configured = PostListReusableRowConformances::configured_presentation_id(&view, cx)?;
        (configured == id).then_some(view)
    }

    fn can_mount(list_view: &Entity<RiseManualListView>, cx: &App) -> bool {
        let list_view = list_view.read(cx);
        if !list_view.is_scroll_in_progress() {
            return true;
        }
        f32::from(list_view.scroll_metrics().velocity_y).abs()
            < RiseManualListMountEnvelopePolicy::FAST_SCROLL_VELOCITY_THRESHOLD
    }

    fn enqueue(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        if self.pending_set.insert(id.to_owned()) {
            self.pending.push(id.to_owned());
        }
        self.start_display_link_if_needed(cx);
    }

    fn remove_pending(&mut self, index: usize) {
        let id = self.pending.remove(index);
        self.pending_set.remove(&id);
    }

    fn start_display_link_if_needed(&mut self, cx: &mut Context<Self>) {
        self.is_display_link_paused = false;
        if self.is_display_link_scheduled {
            return;
        }
        self.is_display_link_scheduled = true;
        let this = cx.weak_entity();
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            let tick_owner = this.clone();
            let scheduled = window_handle.update(cx, |_, window, _| Self::schedule_tick(tick_owner, window));
            if scheduled.is_err() {
                let _ = this.update(cx, |queue, _| {
                    queue.is_display_link_scheduled = false;
                    queue.pause();
                });
            }
        });
    }

    fn schedule_tick(this: WeakEntity<Self>, window: &Window) {
        window.on_next_frame(move |window, cx| {
            let _ = this.update(cx, |queue, cx| queue.tick(window, cx));
        });
    }

    fn pause(&mut self) {
        self.is_display_link_paused = true;
    }
}
