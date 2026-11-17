use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{AnyWindowHandle, App, AppContext, Bounds, Context, Entity, Pixels, Subscription, Task, WeakEntity, Window, px};

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseSubscription, UIApplication};
use crate::core::desktop_platform::rise_app_lifecycle::RiseAppLifecycle;
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;
use crate::modules::post::components::post_view_dwell_session::PostViewDwellSession;
use crate::modules::post::engine::rise_post_engine_models::RisePostViewSurface;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowActionRouting, PostRowLayout, PreparedPostRow,
};
use crate::modules::post::stores::post_services::post_watch_services::PostWatchServicesStore;

/// what the dwell session closure reads from the tracker
struct PostListViewTrackerFacts {
    includes_hosted_rows: Cell<bool>,
    list_view: WeakEntity<RiseManualListView>,
    window_handle: AnyWindowHandle,
    row_provider: Rc<dyn Fn(&str, &App) -> Option<Arc<PreparedPostRow>>>,
    visible_ids: RefCell<Vec<String>>,
    viewport_clip: Rc<Cell<Option<Bounds<Pixels>>>>,
    is_suspended: Cell<bool>,
    scene_is_deactivating: Cell<bool>,
    is_stopped: Cell<bool>,
}

impl PostListViewTrackerFacts {
    fn tracks(&self, row: &PreparedPostRow) -> bool {
        self.includes_hosted_rows.get() || !row.route.is_hosted_fallback()
    }

    fn is_qualified(&self, presentation_id: &str, cx: &App) -> bool {
        if self.is_stopped.get() || !self.visible_ids.borrow().iter().any(|id| id == presentation_id) {
            return false;
        }
        let Some(row) = (self.row_provider)(presentation_id, cx) else {
            return false;
        };
        if !self.tracks(&row) {
            return false;
        }
        self.qualifying_viewport(cx).is_some_and(|viewport| self.qualifies(presentation_id, &row, viewport, cx))
    }

    /// clipped list viewport in window coords, none when nothing can count
    fn qualifying_viewport(&self, cx: &App) -> Option<Bounds<Pixels>> {
        if self.is_suspended.get() || self.scene_is_deactivating.get() {
            return None;
        }
        if !RiseAppLifecycle::is_foreground_active(self.window_handle, cx) {
            return None;
        }
        let list_view = self.list_view.upgrade()?;
        let list_view = list_view.read(cx);
        let list_frame = list_view.bounds();
        if !list_view.is_in_window()
            || f32::from(list_frame.size.width) <= 1.0
            || f32::from(list_frame.size.height) <= 1.0
        {
            return None;
        }
        let viewport = match self.viewport_clip.get() {
            Some(clip) => {
                if !list_frame.intersects(&clip) {
                    return None;
                }
                list_frame.intersect(&clip)
            }
            None => list_frame,
        };
        (!viewport.is_empty()).then_some(viewport)
    }

    fn qualifies(&self, presentation_id: &str, row: &PreparedPostRow, viewport: Bounds<Pixels>, cx: &App) -> bool {
        let Some(list_view) = self.list_view.upgrade() else {
            return false;
        };
        let list_view = list_view.read(cx);
        let Some(layout_frame) = list_view.viewport_frame(presentation_id) else {
            return false;
        };
        let content_height = row.layout.as_ref().map_or_else(
            || {
                (f32::from(row.height())
                    - f32::from(PostRowLayout::SEPARATOR_GAP)
                    - f32::from(PostRowLayout::SEPARATOR_THICKNESS))
                .max(1.0)
            },
            |layout| f32::from(layout.content_height),
        );
        let mut content = layout_frame + list_view.bounds().origin;
        content.size.height = px(f32::from(content.size.height).min(content_height));
        if content.is_empty() || !content.intersects(&viewport) {
            return false;
        }
        let area = |frame: Bounds<Pixels>| f32::from(frame.size.width) * f32::from(frame.size.height);
        let qualifying_area = area(content).min(area(viewport));
        if qualifying_area <= PostListViewTracker::MINIMUM_QUALIFYING_AREA {
            return false;
        }
        area(content.intersect(&viewport)) / qualifying_area >= PostListViewTracker::QUALIFYING_FRACTION
    }
}

/// counts views and impressions for the post list
/// seen when the window is active and half the smaller area is in the viewport
/// one dwell session per visible row owns retries
pub struct PostListViewTracker {
    facts: Rc<PostListViewTrackerFacts>,
    router: Weak<dyn PostRowActionRouting>,
    surface: RisePostViewSurface,
    opened_post_id: Option<String>,

    sessions: HashMap<String, Entity<PostViewDwellSession>>,
    impressed_ids: HashSet<String>,
    pending_visible_ids: Option<Vec<String>>,
    is_evaluate_scheduled: bool,
    watch_owner: String,
    notifications: Vec<RiseSubscription>,
    _scene_listener: Option<Task<()>>,
    _window_activation: Option<Subscription>,
}

impl PostListViewTracker {
    pub const QUALIFYING_FRACTION: f32 = 0.5;
    pub const MINIMUM_QUALIFYING_AREA: f32 = 4.0;

    /// row_provider turns an engine id into its latest prepared row
    pub fn new(
        list_view: &Entity<RiseManualListView>,
        router: Weak<dyn PostRowActionRouting>,
        surface: RisePostViewSurface,
        opened_post_id: Option<String>,
        row_provider: Rc<dyn Fn(&str, &App) -> Option<Arc<PreparedPostRow>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut tracker = Self {
            facts: Rc::new(PostListViewTrackerFacts {
                includes_hosted_rows: Cell::new(true),
                list_view: list_view.downgrade(),
                window_handle: window.window_handle(),
                row_provider,
                visible_ids: RefCell::new(Vec::new()),
                viewport_clip: Rc::new(Cell::new(None)),
                is_suspended: Cell::new(false),
                scene_is_deactivating: Cell::new(false),
                is_stopped: Cell::new(false),
            }),
            router,
            surface,
            opened_post_id,
            sessions: HashMap::new(),
            impressed_ids: HashSet::new(),
            pending_visible_ids: None,
            is_evaluate_scheduled: false,
            watch_owner: format!("post-list-{}", cx.entity_id().as_u64()),
            notifications: Vec::new(),
            _scene_listener: None,
            _window_activation: None,
        };
        tracker.install_scene_observers(window, cx);
        tracker
    }

    pub fn set_includes_hosted_rows(&mut self, includes_hosted_rows: bool) {
        self.facts.includes_hosted_rows.set(includes_hosted_rows);
    }

    /// the list host writes the content mask bounds here from prepaint, gpui has no clipping walk
    pub fn viewport_clip(&self) -> Rc<Cell<Option<Bounds<Pixels>>>> {
        self.facts.viewport_clip.clone()
    }

    pub fn visible_ids_did_change(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if self.facts.is_stopped.get() {
            return;
        }
        self.pending_visible_ids = Some(ids.to_vec());
        self.schedule_evaluate(cx);
    }

    /// runs on a coalesced tick from metrics
    /// commit, window or layout change
    pub fn evaluate(&mut self, cx: &mut Context<Self>) {
        if self.facts.is_stopped.get() {
            return;
        }
        self.schedule_evaluate(cx);
    }

    /// call when a pushed page covers the screen or the list vanished without leaving the window
    pub fn set_suspended(&mut self, suspended: bool, cx: &mut Context<Self>) {
        if self.facts.is_suspended.get() == suspended {
            return;
        }
        self.facts.is_suspended.set(suspended);
        self.evaluate(cx);
    }

    /// ends every session, reports due skips and stops observing
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.facts.is_stopped.get() {
            return;
        }
        for session in self.sessions.values() {
            session.update(cx, |session, cx| session.end(cx));
        }
        self.facts.is_stopped.set(true);
        let watch_owner = self.watch_owner.clone();
        PostWatchServicesStore::shared(cx)
            .update(cx, |store, cx| store.set_desired(Vec::<String>::new(), &watch_owner, cx));
        self.sessions.clear();
        self.facts.visible_ids.borrow_mut().clear();
        self.impressed_ids.clear();
        self.pending_visible_ids = None;
        self.notifications.clear();
        self._scene_listener = None;
        self._window_activation = None;
    }

    fn schedule_evaluate(&mut self, cx: &mut Context<Self>) {
        if self.is_evaluate_scheduled {
            return;
        }
        self.is_evaluate_scheduled = true;
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = this.update(cx, |tracker, cx| {
                tracker.is_evaluate_scheduled = false;
                match tracker.pending_visible_ids.take() {
                    Some(ids) => tracker.apply_visible_ids(ids, cx),
                    None => tracker.perform_evaluate(cx),
                }
            });
        });
    }

    fn apply_visible_ids(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        if self.facts.is_stopped.get() {
            return;
        }
        let next: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let leaving: Vec<String> = self.sessions.keys().filter(|id| !next.contains(id.as_str())).cloned().collect();
        for id in &leaving {
            self.end_session(id, cx);
        }

        let desired: Vec<String> = ids
            .iter()
            .filter_map(|id| (self.facts.row_provider)(id.as_str(), &**cx).map(|row| row.seed.post_id.clone()))
            .collect();
        let watch_owner = self.watch_owner.clone();
        PostWatchServicesStore::shared(cx).update(cx, |store, cx| store.set_desired(desired, &watch_owner, cx));

        if let Some(list_view) = self.facts.list_view.upgrade() {
            let list_view = list_view.read(cx);
            self.impressed_ids.retain(|id| next.contains(id.as_str()) || list_view.mounted_view(id).is_some());
        }
        for id in &ids {
            if self.impressed_ids.contains(id) {
                continue;
            }
            let Some(row) = (self.facts.row_provider)(id.as_str(), &**cx).filter(|row| self.facts.tracks(row)) else {
                continue;
            };
            self.impressed_ids.insert(id.clone());
            if let Some(router) = self.router.upgrade() {
                router.record_impression(&row.id, cx);
            }
        }
        drop(next);
        *self.facts.visible_ids.borrow_mut() = ids;
        self.perform_evaluate(cx);
    }

    fn perform_evaluate(&mut self, cx: &mut Context<Self>) {
        if self.facts.is_stopped.get() {
            return;
        }
        let ids = self.facts.visible_ids.borrow().clone();
        if ids.is_empty() {
            return;
        }
        let viewport = self.facts.qualifying_viewport(cx);
        for id in &ids {
            let Some(row) = (self.facts.row_provider)(id.as_str(), &**cx).filter(|row| self.facts.tracks(row)) else {
                self.end_session(id, cx);
                continue;
            };
            let qualified = viewport.is_some_and(|viewport| self.facts.qualifies(id, &row, viewport, cx));
            let session = self.session(id, &row, cx);
            session.update(cx, |session, cx| session.visibility_changed(qualified, cx));
        }
    }

    fn end_session(&mut self, presentation_id: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.sessions.remove(presentation_id) {
            session.update(cx, |session, cx| session.end(cx));
        }
    }

    fn session(
        &mut self,
        presentation_id: &str,
        row: &PreparedPostRow,
        cx: &mut Context<Self>,
    ) -> Entity<PostViewDwellSession> {
        let post_id = row.seed.view_target_post_id.clone();
        let row_surface = if self.opened_post_id.as_deref() == Some(row.seed.post_id.as_str()) {
            RisePostViewSurface::PostDetail
        } else {
            self.surface
        };
        if let Some(existing) = self.sessions.get(presentation_id) {
            if existing.read(cx).post_id() != post_id {
                existing.update(cx, |session, cx| session.reset_scope(&post_id, row_surface, cx));
            }
            return existing.clone();
        }
        let facts = Rc::downgrade(&self.facts);
        let owned_presentation_id = presentation_id.to_owned();
        let created = cx.new(|_| {
            PostViewDwellSession::new(post_id, row_surface, move |cx: &App| {
                facts.upgrade().is_some_and(|facts| facts.is_qualified(&owned_presentation_id, cx))
            })
        });
        self.sessions.insert(presentation_id.to_owned(), created.clone());
        created
    }

    fn install_scene_observers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (sender, mut inbox) = mpsc::unbounded::<bool>();
        let activations = [
            (UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, false),
            (UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION, true),
            (UIApplication::DID_ENTER_BACKGROUND_NOTIFICATION, true),
        ];
        for (name, deactivates) in activations {
            let sender = sender.clone();
            self.notifications.push(RiseAppEvents::default_center().observe(name, move |_| {
                let _ = sender.unbounded_send(deactivates);
            }));
        }
        self._scene_listener = Some(cx.spawn(async move |this, cx| {
            while let Some(deactivates) = inbox.next().await {
                if this.update(cx, |tracker, cx| tracker.scene_activation_changed(deactivates, cx)).is_err() {
                    break;
                }
            }
        }));
        self._window_activation = Some(cx.observe_window_activation(window, |tracker, window, cx| {
            tracker.scene_activation_changed(!window.is_window_active(), cx);
        }));
    }

    fn scene_activation_changed(&mut self, deactivates: bool, cx: &mut Context<Self>) {
        if self.facts.is_stopped.get() {
            return;
        }
        self.facts.scene_is_deactivating.set(deactivates);
        self.evaluate(cx);
    }
}
