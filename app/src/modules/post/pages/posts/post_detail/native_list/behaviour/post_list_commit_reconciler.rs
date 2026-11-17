use std::cell::{Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use gpui::{AnyView, AnyWindowHandle, App, AppContext, EmptyView, Entity, WeakEntity, Window};

use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_types::{
    RiseManualListItem, RiseManualListItemMountPolicy, RiseManualListItemProvider, RiseManualListTransaction,
    RiseManualListTransactionAnchor, RiseManualListTransactionEdgeMutation, RiseManualListTransactionUpdateAnimation,
};
use crate::modules::chat::components::chat::async_render::list_engine::rise_manual_list_view::RiseManualListView;

const UNWIRED_REUSE_IDENTIFIER: &str = "post.unwired";
const TOP_ANCHOR_TOLERANCE: f32 = 0.5;
const UPDATE_ANIMATION_DURATION: f64 = 0.34;
const UPDATE_ANIMATION_DAMPING_RATIO: f32 = 0.86;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostListCommitReconcilerPlanKind {
    Initial,
    Edge,
    Full,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PostListCommitReconcilerPlan {
    pub ids: Vec<String>,
    pub kind: PostListCommitReconcilerPlanKind,
    pub edge_mutation: Option<RiseManualListTransactionEdgeMutation>,
    pub updated_ids: HashSet<String>,
    pub allows_application_during_scroll: bool,
    pub defers_to_idle: bool,
}

/// an applied transaction, reported from its completion
#[derive(Clone, Debug)]
pub struct PostListCommitReconcilerCommit {
    pub plan: PostListCommitReconcilerPlan,
    pub previous_ids: Vec<String>,
    pub inserted_ids: HashSet<String>,
    pub removed_ids: HashSet<String>,
    pub height_changed_ids: HashSet<String>,
}

struct PostListCommitReconcilerDirtyEntry {
    generation: u64,
    height_changed: bool,
    idle_only: bool,
}

struct PostListCommitReconcilerPending {
    token: u64,
    plan: PostListCommitReconcilerPlan,
    previous_ids: Vec<String>,
    captured_generations: HashMap<String, u64>,
    height_changed_ids: HashSet<String>,
}

#[derive(Default)]
struct PostListCommitReconcilerState {
    presented_ids: Vec<String>,
    presented_set: HashSet<String>,
    has_applied: bool,
    dirty: HashMap<String, PostListCommitReconcilerDirtyEntry>,
    dirty_generation: u64,
    pending: Option<PostListCommitReconcilerPending>,
    next_token: u64,
    is_commit_scheduled: bool,
    schedule_token: u64,
    defers_to_idle: bool,
    is_stopped: bool,
    pinned_row_top: Option<(String, f32)>,
}

pub type PostListCommitReconcilerDesiredIds = Rc<dyn Fn(&App) -> Vec<String>>;
pub type PostListCommitReconcilerExpectedWidth = Rc<dyn Fn(&App) -> Option<f32>>;
pub type PostListCommitReconcilerCondition = Rc<dyn Fn(&App) -> bool>;
pub type PostListCommitReconcilerReturningRow = Rc<dyn Fn(&App) -> Option<(String, String)>>;
pub type PostListCommitReconcilerOnApplied = Rc<dyn Fn(&PostListCommitReconcilerCommit, &mut Window, &mut App)>;

pub struct PostListCommitReconciler {
    this: Weak<Self>,
    list_view: WeakEntity<RiseManualListView>,
    window_handle: AnyWindowHandle,
    state: RefCell<PostListCommitReconcilerState>,

    pub provider: RefCell<RiseManualListItemProvider>,
    pub desired_ids: RefCell<PostListCommitReconcilerDesiredIds>,
    pub expected_width: RefCell<PostListCommitReconcilerExpectedWidth>,
    pub can_commit: RefCell<PostListCommitReconcilerCondition>,
    pub on_applied: RefCell<Option<PostListCommitReconcilerOnApplied>>,
    pub prefers_top_anchor: RefCell<PostListCommitReconcilerCondition>,
    pub returning_row: RefCell<PostListCommitReconcilerReturningRow>,
    last_trace: RefCell<String>,
}

impl PostListCommitReconciler {
    pub fn new(list_view: &Entity<RiseManualListView>, cx: &App) -> Rc<Self> {
        let window_handle = list_view.read(cx).window_handle();
        let list_view = list_view.downgrade();
        Rc::new_cyclic(|this| Self {
            this: this.clone(),
            list_view,
            window_handle,
            state: RefCell::new(PostListCommitReconcilerState::default()),
            provider: RefCell::new(Self::unwired_provider()),
            desired_ids: RefCell::new(Rc::new(|_: &App| Vec::new())),
            expected_width: RefCell::new(Rc::new(|_: &App| None)),
            can_commit: RefCell::new(Rc::new(|_: &App| false)),
            on_applied: RefCell::new(None),
            prefers_top_anchor: RefCell::new(Rc::new(|_: &App| false)),
            returning_row: RefCell::new(Rc::new(|_: &App| None)),
            last_trace: RefCell::new(String::new()),
        })
    }

    fn unwired_provider() -> RiseManualListItemProvider {
        Rc::new(|_id: &str, _width: f32, _window: &mut Window, _cx: &mut App| {
            RiseManualListItem::new(
                1.0,
                UNWIRED_REUSE_IDENTIFIER.to_owned(),
                0,
                RiseManualListItemMountPolicy::Immediate,
                None,
                None,
                None,
                None,
                Rc::new(|reused: Option<AnyView>, _window: &mut Window, cx: &mut App| {
                    reused.unwrap_or_else(|| cx.new(|_| EmptyView).into())
                }),
            )
        })
    }

    pub fn presented_ids(&self) -> Ref<'_, [String]> {
        Ref::map(self.state.borrow(), |state| state.presented_ids.as_slice())
    }

    pub fn presented_set(&self) -> Ref<'_, HashSet<String>> {
        Ref::map(self.state.borrow(), |state| &state.presented_set)
    }

    pub fn has_applied(&self) -> bool {
        self.state.borrow().has_applied
    }

    /// a transaction is enqueued and not applied yet, or one is scheduled for the next turn
    pub fn is_commit_in_flight(&self) -> bool {
        let state = self.state.borrow();
        state.pending.is_some() || state.is_commit_scheduled
    }

    /// a transaction sits in the engine pending slot
    pub fn has_enqueued_transaction(&self) -> bool {
        self.state.borrow().pending.is_some()
    }

    /// part of the desired state waits for the scroll to end
    pub fn has_deferred_changes(&self) -> bool {
        self.state.borrow().defers_to_idle
    }

    /// ids the provider may still be asked for
    /// presented or in the enqueued transaction
    pub fn is_retained(&self, id: &str) -> bool {
        let state = self.state.borrow();
        state.presented_set.contains(id)
            || state.pending.as_ref().is_some_and(|pending| pending.plan.ids.iter().any(|pending_id| pending_id == id))
    }

    pub fn retained_ids(&self) -> HashSet<String> {
        Self::retained_ids_of(&self.state.borrow())
    }

    fn retained_ids_of(state: &PostListCommitReconcilerState) -> HashSet<String> {
        let mut retained = state.presented_set.clone();
        if let Some(pending) = state.pending.as_ref() {
            retained.extend(pending.plan.ids.iter().cloned());
        }
        retained
    }

    /// providers row for id changed
    /// a retained row is re provided by the next commit
    pub fn mark_dirty(&self, id: &str, height_changed: bool, idle_only: bool) {
        let mut state = self.state.borrow_mut();
        if state.is_stopped {
            return;
        }
        state.dirty_generation = state.dirty_generation.wrapping_add(1);
        let generation = state.dirty_generation;
        if let Some(entry) = state.dirty.get_mut(id) {
            entry.generation = generation;
            entry.height_changed = entry.height_changed || height_changed;
            entry.idle_only = entry.idle_only || idle_only;
        } else {
            state
                .dirty
                .insert(id.to_owned(), PostListCommitReconcilerDirtyEntry { generation, height_changed, idle_only });
        }
    }

    pub fn pin_row_top_on_next_height_change(&self, id: &str, viewport_offset: f32) {
        let mut state = self.state.borrow_mut();
        if state.is_stopped {
            return;
        }
        state.pinned_row_top = Some((id.to_owned(), viewport_offset));
    }

    /// reconciles on the next main turn, calls within a turn coalesce
    pub fn set_needs_commit(&self, cx: &mut App) {
        let token = {
            let mut state = self.state.borrow_mut();
            if state.is_stopped || state.is_commit_scheduled {
                return;
            }
            state.is_commit_scheduled = true;
            state.schedule_token = state.schedule_token.wrapping_add(1);
            state.schedule_token
        };
        let this = self.this.clone();
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let Some(this) = this.upgrade() else { return };
                {
                    let mut state = this.state.borrow_mut();
                    if state.schedule_token != token {
                        return;
                    }
                    state.is_commit_scheduled = false;
                }
                this.commit_now(window, cx);
            });
        });
    }

    /// reconciles now, may apply synchronously
    /// list entity must not be under update
    /// from one of its own callbacks use set_needs_commit
    pub fn commit_now(&self, window: &mut Window, cx: &mut App) {
        let Some(list_view) = self.list_view.upgrade() else { return };
        if self.state.borrow().is_stopped {
            return;
        }
        let can_commit = self.can_commit.borrow().clone();
        if !can_commit(cx) {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            if state.is_commit_scheduled {
                state.is_commit_scheduled = false;
                state.schedule_token = state.schedule_token.wrapping_add(1);
            }
        }
        let (is_scrolling, visible_ids, distance_from_top) = {
            let list = list_view.read(cx);
            (
                list.is_scroll_in_progress(),
                list.visible_ids().iter().cloned().collect::<HashSet<String>>(),
                list.scroll_metrics().distance_from_top.as_f32(),
            )
        };
        self.drop_stale_dirty_entries();
        let desired_ids = self.desired_ids.borrow().clone();
        let desired = desired_ids(cx);

        let (plan, previous_ids, presented_set, inserted_ids, captured, height_changed) = {
            let mut state = self.state.borrow_mut();
            let idle_only: HashSet<String> =
                state.dirty.iter().filter(|(_, entry)| entry.idle_only).map(|(id, _)| id.clone()).collect();
            let dirty: HashSet<String> = state.dirty.keys().cloned().collect();
            let Some(plan) = Self::plan(&state.presented_ids, &desired, &dirty, &idle_only, is_scrolling) else {
                state.defers_to_idle = is_scrolling && (desired != state.presented_ids || !idle_only.is_empty());
                let line = format!(
                    "commit none scrolling={is_scrolling} presented={} desired={} same={} deferred={}",
                    state.presented_ids.len(),
                    desired.len(),
                    desired == state.presented_ids,
                    state.defers_to_idle
                );
                drop(state);
                self.trace_x272(line);
                return;
            };
            state.defers_to_idle = plan.defers_to_idle;
            let line = format!(
                "commit plan kind={:?} scrolling={is_scrolling} presented={} desired={} ids={} edge={:?} scroll_safe={} same_pending={}",
                plan.kind,
                state.presented_ids.len(),
                desired.len(),
                plan.ids.len(),
                plan.edge_mutation.as_ref().map(|edge| (
                    edge.removed_prefix_count,
                    edge.removed_suffix_count,
                    edge.inserted_prefix_count,
                    edge.inserted_suffix_count
                )),
                plan.allows_application_during_scroll,
                state.pending.as_ref().is_some_and(|pending| pending.plan == plan)
            );
            if state.pending.as_ref().is_some_and(|pending| pending.plan == plan) {
                drop(state);
                self.trace_x272(line);
                return;
            }
            self.trace_x272(line);
            let inserted_ids = Self::inserted_ids(&plan, &state.presented_set);
            let mut captured: HashMap<String, u64> = HashMap::new();
            let mut height_changed: HashSet<String> = HashSet::new();
            for id in plan.updated_ids.union(&inserted_ids) {
                let Some(entry) = state.dirty.get(id) else { continue };
                captured.insert(id.clone(), entry.generation);
                if entry.height_changed && plan.updated_ids.contains(id) {
                    height_changed.insert(id.clone());
                }
            }
            (plan, state.presented_ids.clone(), state.presented_set.clone(), inserted_ids, captured, height_changed)
        };

        let removed_ids: HashSet<String> = {
            let plan_set: HashSet<&String> = plan.ids.iter().collect();
            presented_set.iter().filter(|id| !plan_set.contains(id)).cloned().collect()
        };
        let animation = if is_scrolling {
            None
        } else {
            Self::animation(&visible_ids, &inserted_ids, &removed_ids, &height_changed)
        };

        let removes_top_row_on_screen =
            previous_ids.first().is_some_and(|first| removed_ids.contains(first) && visible_ids.contains(first));
        let prefers_top_anchor = self.prefers_top_anchor.borrow().clone();
        let anchors_at_top = plan.kind == PostListCommitReconcilerPlanKind::Initial
            || (!is_scrolling && prefers_top_anchor(cx) && distance_from_top <= TOP_ANCHOR_TOLERANCE)
            || (!is_scrolling && removes_top_row_on_screen);

        let mut anchor =
            if anchors_at_top { RiseManualListTransactionAnchor::Top } else { RiseManualListTransactionAnchor::PreserveVisible };
        {
            let mut state = self.state.borrow_mut();
            let pin_changed = state.pinned_row_top.as_ref().is_some_and(|(id, _)| height_changed.contains(id));
            let pin = if pin_changed { state.pinned_row_top.take() } else { None };
            if let Some((id, viewport_offset)) = pin.filter(|_| !anchors_at_top && !is_scrolling) {
                anchor = RiseManualListTransactionAnchor::Item { id, viewport_offset };
            }
        }
        if !anchors_at_top && !is_scrolling && matches!(anchor, RiseManualListTransactionAnchor::PreserveVisible) {
            let returning_row = self.returning_row.borrow().clone();
            if let Some((returning, successor)) = returning_row(cx)
                && inserted_ids.contains(&returning)
                && let Some(successor_anchor) = list_view.read(cx).capture_viewport_anchor(&successor)
            {
                anchor = RiseManualListTransactionAnchor::Item { id: returning, viewport_offset: successor_anchor.viewport_offset };
            }
        }

        let expected_width = self.expected_width.borrow().clone();
        let transaction = RiseManualListTransaction::new(
            plan.ids.clone(),
            anchor,
            expected_width(cx),
            Some(plan.updated_ids.clone()),
            animation,
            None,
            plan.edge_mutation,
            plan.allows_application_during_scroll,
            HashSet::new(),
            self.provider.borrow().clone(),
        );

        let token = {
            let mut state = self.state.borrow_mut();
            state.next_token = state.next_token.wrapping_add(1);
            let token = state.next_token;
            state.pending = Some(PostListCommitReconcilerPending {
                token,
                plan,
                previous_ids,
                captured_generations: captured,
                height_changed_ids: height_changed,
            });
            token
        };
        let on_complete = self.this.clone();
        let on_discard = self.this.clone();
        list_view.update(cx, |list, cx| {
            list.enqueue(
                transaction,
                Some(Box::new(move |window: &mut Window, cx: &mut App| {
                    if let Some(this) = on_complete.upgrade() {
                        this.did_apply(token, window, cx);
                    }
                })),
                Some(Box::new(move |_window: &mut Window, cx: &mut App| {
                    if let Some(this) = on_discard.upgrade() {
                        this.did_discard(token, cx);
                    }
                })),
                window,
                cx,
            );
        });
    }

    pub fn stop(&self) {
        {
            let mut state = self.state.borrow_mut();
            if state.is_stopped {
                return;
            }
            state.is_stopped = true;
            state.schedule_token = state.schedule_token.wrapping_add(1);
            state.is_commit_scheduled = false;
            state.pending = None;
            state.dirty.clear();
            state.pinned_row_top = None;
        }
        *self.on_applied.borrow_mut() = None;
        *self.provider.borrow_mut() = Self::unwired_provider();
        *self.desired_ids.borrow_mut() = Rc::new(|_: &App| Vec::new());
        *self.can_commit.borrow_mut() = Rc::new(|_: &App| false);
        *self.prefers_top_anchor.borrow_mut() = Rc::new(|_: &App| false);
        *self.returning_row.borrow_mut() = Rc::new(|_: &App| None);
    }

    /// next transaction from presented towards desired, or none when nothing can go in now
    pub fn plan(
        presented: &[String],
        desired: &[String],
        dirty: &HashSet<String>,
        idle_only_dirty: &HashSet<String>,
        is_scrolling: bool,
    ) -> Option<PostListCommitReconcilerPlan> {
        if presented.is_empty() {
            if desired.is_empty() {
                return None;
            }
            return Some(PostListCommitReconcilerPlan {
                ids: desired.to_vec(),
                kind: PostListCommitReconcilerPlanKind::Initial,
                edge_mutation: None,
                updated_ids: HashSet::new(),
                allows_application_during_scroll: false,
                defers_to_idle: false,
            });
        }
        let presented_set: HashSet<&String> = presented.iter().collect();
        let desired_set: HashSet<&String> = desired.iter().collect();
        let updatable: HashSet<String> =
            dirty.iter().filter(|id| presented_set.contains(id) && desired_set.contains(id)).cloned().collect();
        let scroll_updatable: HashSet<String> = updatable.difference(idle_only_dirty).cloned().collect();
        let diff = RiseManualListTransactionEdgeMutation::diff(presented, desired);

        if let Some(diff) = diff
            && diff.removed_prefix_count == 0
            && diff.removed_suffix_count == 0
        {
            let updated = if is_scrolling { scroll_updatable } else { updatable.clone() };
            if desired == presented && updated.is_empty() {
                return None;
            }
            return Some(PostListCommitReconcilerPlan {
                ids: desired.to_vec(),
                kind: PostListCommitReconcilerPlanKind::Edge,
                edge_mutation: Some(diff),
                allows_application_during_scroll: updated.is_disjoint(idle_only_dirty),
                defers_to_idle: is_scrolling && updated.len() != updatable.len(),
                updated_ids: updated,
            });
        }

        if !is_scrolling {
            if let Some(diff) = diff {
                return Some(PostListCommitReconcilerPlan {
                    ids: desired.to_vec(),
                    kind: PostListCommitReconcilerPlanKind::Edge,
                    edge_mutation: Some(diff),
                    updated_ids: updatable,
                    allows_application_during_scroll: false,
                    defers_to_idle: false,
                });
            }
            return Some(PostListCommitReconcilerPlan {
                ids: desired.to_vec(),
                kind: PostListCommitReconcilerPlanKind::Full,
                edge_mutation: None,
                updated_ids: updatable,
                allows_application_during_scroll: false,
                defers_to_idle: false,
            });
        }

        let mut desired_index: HashMap<&String, usize> = HashMap::with_capacity(desired.len());
        for (index, id) in desired.iter().enumerate() {
            desired_index.entry(id).or_insert(index);
        }
        let mut prefix: Vec<String> = Vec::new();
        let mut suffix: Vec<String> = Vec::new();
        let first = presented.iter().find(|id| desired_index.contains_key(id));
        let last = presented.iter().rev().find(|id| desired_index.contains_key(id));
        if let (Some(first), Some(last)) = (first, last)
            && let (Some(first_index), Some(last_index)) = (desired_index.get(first), desired_index.get(last))
            && first_index <= last_index
        {
            prefix = desired[..*first_index].iter().filter(|id| !presented_set.contains(id)).cloned().collect();
            suffix = desired[*last_index + 1..].iter().filter(|id| !presented_set.contains(id)).cloned().collect();
        }
        if prefix.is_empty() && suffix.is_empty() && scroll_updatable.is_empty() {
            return None;
        }
        let edge_mutation = RiseManualListTransactionEdgeMutation::new(0, 0, prefix.len(), suffix.len());
        let mut ids = prefix;
        ids.extend_from_slice(presented);
        ids.append(&mut suffix);
        Some(PostListCommitReconcilerPlan {
            ids,
            kind: PostListCommitReconcilerPlanKind::Edge,
            edge_mutation: Some(edge_mutation),
            updated_ids: scroll_updatable,
            allows_application_during_scroll: true,
            defers_to_idle: true,
        })
    }

    fn trace_x272(&self, line: String) {
        let mut last = self.last_trace.borrow_mut();
        if *last != line {
            tracing::warn!(target: "riseonly::X272", "{line}");
            *last = line;
        }
    }

    fn inserted_ids(plan: &PostListCommitReconcilerPlan, presented: &HashSet<String>) -> HashSet<String> {
        plan.ids.iter().filter(|id| !presented.contains(*id)).cloned().collect()
    }

    /// idle only
    /// rows below a removed or resized visible row slide instead of jumping
    fn animation(
        visible_ids: &HashSet<String>,
        inserted_ids: &HashSet<String>,
        removed_ids: &HashSet<String>,
        height_changed_ids: &HashSet<String>,
    ) -> Option<RiseManualListTransactionUpdateAnimation> {
        let removed_visible: HashSet<String> = removed_ids.intersection(visible_ids).cloned().collect();
        if removed_visible.is_empty() && height_changed_ids.is_disjoint(visible_ids) {
            return None;
        }
        if removed_visible.len() > 1 && removed_visible.len() * 2 > visible_ids.len() {
            tracing::warn!(
                target: "riseonly::X1761",
                "update animation skipped removed_visible={} visible={}",
                removed_visible.len(),
                visible_ids.len()
            );
            return None;
        }
        Some(RiseManualListTransactionUpdateAnimation::new(
            UPDATE_ANIMATION_DURATION,
            UPDATE_ANIMATION_DAMPING_RATIO,
            visible_ids.union(inserted_ids).cloned().collect(),
            inserted_ids.clone(),
            removed_visible,
            HashSet::new(),
        ))
    }

    fn drop_stale_dirty_entries(&self) {
        let mut state = self.state.borrow_mut();
        if state.dirty.is_empty() {
            return;
        }
        let retained = Self::retained_ids_of(&state);
        state.dirty.retain(|id, _| retained.contains(id));
    }

    fn did_apply(&self, token: u64, window: &mut Window, cx: &mut App) {
        let commit = {
            let mut state = self.state.borrow_mut();
            if state.is_stopped || state.pending.as_ref().is_none_or(|pending| pending.token != token) {
                return;
            }
            let Some(pending) = state.pending.take() else { return };
            state.presented_ids = pending.plan.ids.clone();
            state.presented_set = pending.plan.ids.iter().cloned().collect();
            state.has_applied = true;
            for (id, generation) in &pending.captured_generations {
                if state.dirty.get(id).is_some_and(|entry| entry.generation == *generation) {
                    state.dirty.remove(id);
                }
            }
            let previous_set: HashSet<String> = pending.previous_ids.iter().cloned().collect();
            PostListCommitReconcilerCommit {
                inserted_ids: state.presented_set.difference(&previous_set).cloned().collect(),
                removed_ids: previous_set.difference(&state.presented_set).cloned().collect(),
                plan: pending.plan,
                previous_ids: pending.previous_ids,
                height_changed_ids: pending.height_changed_ids,
            }
        };
        let this = self.this.clone();
        window.defer(cx, move |window, cx| {
            let Some(this) = this.upgrade() else { return };
            let on_applied = this.on_applied.borrow().clone();
            if let Some(on_applied) = on_applied {
                on_applied(&commit, window, cx);
            }
        });
        self.set_needs_commit(cx);
    }

    fn did_discard(&self, token: u64, cx: &mut App) {
        self.trace_x272(format!("commit discarded token={token}"));
        {
            let mut state = self.state.borrow_mut();
            if state.is_stopped || state.pending.as_ref().is_none_or(|pending| pending.token != token) {
                return;
            }
            state.pending = None;
        }
        self.set_needs_commit(cx);
    }
}
