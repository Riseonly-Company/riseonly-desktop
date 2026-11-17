use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use gpui::{
    App, Bounds, Context, Entity, EntityId, IntoElement, Pixels, Styled, Subscription, WeakEntity, canvas,
};

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListFocusReporting, PostListPlaybackToken,
};
use crate::modules::shorts::components::shorts_inline_feed_cell::ShortsFeedFocusCoordinator;

/// the surfaces window frame and the row that holds it
pub struct PostListFocusFrameView {
    row_view: EntityId,
    painted: Cell<Option<Bounds<Pixels>>>,
}

impl PostListFocusFrameView {
    /// row_view is the mounted row entity the surface draws in
    pub fn new(row_view: EntityId) -> Self {
        Self { row_view, painted: Cell::new(None) }
    }

    /// called from the surface prepaint with the frame in window coordinates
    pub fn set_painted(&self, frame: Bounds<Pixels>) {
        self.painted.set(Some(frame));
    }

    /// probe a surface draws over its frame
    /// writes the frame from prepaint and keeps it when a cached row replays moved without a prepaint
    pub fn probe(self: &Rc<Self>) -> impl IntoElement + use<> {
        let frame_view = self.clone();
        canvas(
            move |bounds, window, _| {
                frame_view.set_painted(bounds);
                let moved = frame_view.clone();
                window.on_replayed_move(move |offset, _, _, _| {
                    moved.set_painted(bounds + offset);
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    /// the surface stopped drawing, window is gone or its hidden
    pub fn clear_painted(&self) {
        self.painted.set(None);
    }
}

struct PostListPlaybackFocusEntry {
    view: Weak<PostListFocusFrameView>,
    last_reported_frame: Option<Bounds<Pixels>>,
}

/// engine driven focus reports for video, riff and ambient music surfaces
pub struct PostListPlaybackFocus {
    pub session: u64,
    pub on_focus_changed: Option<Rc<dyn Fn(&HashSet<String>, &mut App)>>,

    list_view: Option<WeakEntity<RiseManualListView>>,
    coordinator: Entity<ShortsFeedFocusCoordinator>,
    entries: HashMap<PostListPlaybackToken, PostListPlaybackFocusEntry>,
    tokens_by_raw_value: HashMap<String, PostListPlaybackToken>,
    observed_active_token: Option<String>,
    is_report_scheduled: bool,
    is_suspended: bool,
    is_stopped: bool,
    _coordinator_observer: Option<Subscription>,
}

impl PostListPlaybackFocus {
    /// with list_view a frame view is reported only while it sits inside its rows mounted view
    pub fn new(session: u64, list_view: Option<WeakEntity<RiseManualListView>>, cx: &mut Context<Self>) -> Self {
        let coordinator = ShortsFeedFocusCoordinator::shared(cx);
        let observed_active_token = coordinator.read(cx).active_token().map(str::to_owned);
        let observer = cx.observe(&coordinator, |this, _, cx| this.active_token_did_change(cx));
        Self {
            session,
            on_focus_changed: None,
            list_view,
            coordinator,
            entries: HashMap::new(),
            tokens_by_raw_value: HashMap::new(),
            observed_active_token,
            is_report_scheduled: false,
            is_suspended: false,
            is_stopped: false,
            _coordinator_observer: Some(observer),
        }
    }

    pub fn begin_focus_tracking(
        &mut self,
        token: &PostListPlaybackToken,
        frame_view: &Rc<PostListFocusFrameView>,
        cx: &mut Context<Self>,
    ) {
        if self.is_stopped {
            return;
        }
        let is_same_view =
            self.entries.get(token).is_some_and(|entry| std::ptr::eq(entry.view.as_ptr(), Rc::as_ptr(frame_view)));
        if !is_same_view {
            self.entries.insert(
                token.clone(),
                PostListPlaybackFocusEntry { view: Rc::downgrade(frame_view), last_reported_frame: None },
            );
            self.tokens_by_raw_value.insert(token.raw_value(), token.clone());
        }
        self.schedule_report(cx);
    }

    pub fn end_focus_tracking(&mut self, token: &PostListPlaybackToken, cx: &mut Context<Self>) {
        let raw_value = token.raw_value();
        self.entries.remove(token);
        self.tokens_by_raw_value.remove(&raw_value);
        self.remove_from_coordinator(&raw_value, cx);
    }

    pub fn is_focused(&self, token: &PostListPlaybackToken, cx: &App) -> bool {
        self.coordinator.read(cx).active_token() == Some(token.raw_value().as_str())
    }

    /// an inactive pager page gives up focus since the coordinator ranks frames by height only
    /// reports again once it rests
    pub fn set_suspended(&mut self, suspended: bool, cx: &mut Context<Self>) {
        if self.is_stopped || self.is_suspended == suspended {
            return;
        }
        self.is_suspended = suspended;
        if suspended {
            let tokens: Vec<PostListPlaybackToken> = self.entries.keys().cloned().collect();
            for token in &tokens {
                if let Some(entry) = self.entries.get_mut(token) {
                    entry.last_reported_frame = None;
                }
                self.remove_from_coordinator(&token.raw_value(), cx);
            }
        } else {
            self.report_frames(cx);
        }
    }

    /// coalesced metrics tick, commit or window change
    /// reports moved frames only
    pub fn report_frames(&mut self, cx: &mut Context<Self>) {
        if self.is_stopped || self.entries.is_empty() {
            return;
        }
        self.schedule_report(cx);
    }

    pub fn end_focus_tracking_for_presentation_id(&mut self, presentation_id: &str, cx: &mut Context<Self>) {
        let tokens: Vec<PostListPlaybackToken> =
            self.entries.keys().filter(|token| token.presentation_id == presentation_id).cloned().collect();
        for token in &tokens {
            self.end_focus_tracking(token, cx);
        }
    }

    pub fn retain_tokens(&mut self, presentation_ids: &HashSet<String>, cx: &mut Context<Self>) {
        let tokens: Vec<PostListPlaybackToken> =
            self.entries.keys().filter(|token| !presentation_ids.contains(&token.presentation_id)).cloned().collect();
        for token in &tokens {
            self.end_focus_tracking(token, cx);
        }
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        self.is_stopped = true;
        self._coordinator_observer = None;
        let tokens: Vec<PostListPlaybackToken> = self.entries.keys().cloned().collect();
        for token in &tokens {
            self.remove_from_coordinator(&token.raw_value(), cx);
        }
        self.entries.clear();
        self.tokens_by_raw_value.clear();
        self.on_focus_changed = None;
    }

    fn remove_from_coordinator(&self, raw_value: &str, cx: &mut Context<Self>) {
        self.coordinator.update(cx, |coordinator, cx| coordinator.remove(raw_value, cx));
    }

    fn schedule_report(&mut self, cx: &mut Context<Self>) {
        if self.is_report_scheduled {
            return;
        }
        self.is_report_scheduled = true;
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = this.update(cx, |focus, cx| {
                focus.is_report_scheduled = false;
                focus.report_all(cx);
            });
        });
    }

    fn report_all(&mut self, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        let tokens: Vec<PostListPlaybackToken> = self.entries.keys().cloned().collect();
        for token in &tokens {
            self.report(token, cx);
        }
    }

    fn report(&mut self, token: &PostListPlaybackToken, cx: &mut Context<Self>) {
        if self.is_suspended {
            return;
        }
        let Some(entry) = self.entries.get(token) else {
            return;
        };
        let Some(view) = entry.view.upgrade() else {
            self.end_focus_tracking(token, cx);
            return;
        };
        let last_reported_frame = entry.last_reported_frame;
        let frame = view.painted.get().filter(|frame| self.is_reportable(&view, *frame, &token.presentation_id, cx));
        let Some(frame) = frame else {
            if last_reported_frame.is_some() {
                if let Some(entry) = self.entries.get_mut(token) {
                    entry.last_reported_frame = None;
                }
                self.remove_from_coordinator(&token.raw_value(), cx);
            }
            return;
        };
        let (min_x, min_y, height) =
            (f32::from(frame.origin.x), f32::from(frame.origin.y), f32::from(frame.size.height));
        if !min_x.is_finite() || !min_y.is_finite() || !height.is_finite() {
            return;
        }
        let is_unmoved = last_reported_frame.is_some_and(|last| {
            (f32::from(last.origin.y) - min_y).abs() < 0.5
                && (f32::from(last.origin.x) - min_x).abs() < 0.5
                && (f32::from(last.size.height) - height).abs() < 0.5
        });
        if is_unmoved {
            return;
        }
        if let Some(entry) = self.entries.get_mut(token) {
            entry.last_reported_frame = Some(frame);
        }
        let raw_value = token.raw_value();
        self.coordinator.update(cx, |coordinator, cx| coordinator.report(&raw_value, frame, cx));
    }

    fn is_reportable(
        &self,
        view: &PostListFocusFrameView,
        frame: Bounds<Pixels>,
        presentation_id: &str,
        cx: &App,
    ) -> bool {
        if f32::from(frame.size.height) <= 1.0 {
            return false;
        }
        let Some(list_view) = self.list_view.as_ref().and_then(WeakEntity::upgrade) else {
            return true;
        };
        let list_view = list_view.read(cx);
        list_view.is_in_window()
            && list_view.mounted_view(presentation_id).is_some_and(|row_view| row_view.entity_id() == view.row_view)
    }

    fn active_token_did_change(&mut self, cx: &mut Context<Self>) {
        if self.is_stopped {
            return;
        }
        let current = self.coordinator.read(cx).active_token().map(str::to_owned);
        if self.observed_active_token == current {
            return;
        }
        let previous = std::mem::replace(&mut self.observed_active_token, current.clone());
        let mut ids = HashSet::new();
        for raw_value in [previous, current].into_iter().flatten() {
            if let Some(token) = self.tokens_by_raw_value.get(&raw_value) {
                ids.insert(token.presentation_id.clone());
            }
        }
        if ids.is_empty() {
            return;
        }
        let Some(callback) = self.on_focus_changed.clone() else {
            return;
        };
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            if this.upgrade().is_some_and(|focus| !focus.read(cx).is_stopped) {
                callback(&ids, cx);
            }
        });
    }
}

impl PostListFocusReporting for Entity<PostListPlaybackFocus> {
    fn begin_focus_tracking(
        &self,
        token: &PostListPlaybackToken,
        frame_view: &Rc<PostListFocusFrameView>,
        cx: &mut App,
    ) {
        self.update(cx, |focus, cx| focus.begin_focus_tracking(token, frame_view, cx));
    }

    fn end_focus_tracking(&self, token: &PostListPlaybackToken, cx: &mut App) {
        self.update(cx, |focus, cx| focus.end_focus_tracking(token, cx));
    }

    fn is_focused(&self, token: &PostListPlaybackToken, cx: &App) -> bool {
        self.read(cx).is_focused(token, cx)
    }
}
