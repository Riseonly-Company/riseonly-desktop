use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AnyElement, App, ElementId, Entity, EntityId, Global, SharedString, Task, Window};
use rise_canon::rise_sync::{RETRY_BASE_MS, RETRY_EXPONENT_MAXIMUM, RETRY_MAXIMUM_ATTEMPTS};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::{
    SaiWsPaginationEdge, SaiWsPaginationFailureKind, SaiWsPaginationState,
};

#[derive(Clone, PartialEq, Eq, Hash)]
struct Lane {
    cache_key: String,
    edge: SaiWsPaginationEdge,
    state_id: EntityId,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct RetryLane {
    cache_key: String,
    edge: SaiWsPaginationEdge,
}

#[derive(Default)]
struct SaiPaginationPrefetchGate {
    claimed_token_by_lane: HashMap<Lane, String>,
    retry_task_by_lane: HashMap<RetryLane, Task<()>>,
    retry_attempts_by_lane: HashMap<RetryLane, u64>,
}

impl Global for SaiPaginationPrefetchGate {}

impl SaiPaginationPrefetchGate {
    fn claim(
        cache_key: &str,
        edge: SaiWsPaginationEdge,
        state: &Entity<SaiWsPaginationState>,
        token: String,
        cx: &mut App,
    ) -> bool {
        let scoped_cache_key = SaiAccountCacheScope::scoped_key(cache_key);
        let state_id = state.entity_id();
        let gate = cx.default_global::<Self>();
        gate.claimed_token_by_lane
            .retain(|lane, _| lane.cache_key != scoped_cache_key || lane.edge != edge || lane.state_id == state_id);
        let lane = Lane { cache_key: scoped_cache_key, edge, state_id };
        if gate.claimed_token_by_lane.get(&lane) == Some(&token) {
            return false;
        }
        gate.claimed_token_by_lane.insert(lane, token);
        true
    }

    fn rearm(cache_key: &str, edge: SaiWsPaginationEdge, cx: &mut App) {
        let scoped_cache_key = SaiAccountCacheScope::scoped_key(cache_key);
        cx.default_global::<Self>()
            .claimed_token_by_lane
            .retain(|lane, _| lane.cache_key != scoped_cache_key || lane.edge != edge);
    }

    fn schedule_transport_retry(cache_key: &str, edge: SaiWsPaginationEdge, cx: &mut App) {
        let lane = RetryLane { cache_key: SaiAccountCacheScope::scoped_key(cache_key), edge };
        let gate = cx.default_global::<Self>();
        if gate.retry_task_by_lane.contains_key(&lane) {
            return;
        }
        let attempt = gate.retry_attempts_by_lane.get(&lane).copied().unwrap_or(0);
        if attempt + 1 >= RETRY_MAXIMUM_ATTEMPTS {
            return;
        }
        gate.retry_attempts_by_lane.insert(lane.clone(), attempt + 1);
        let delay_ms = RETRY_BASE_MS << attempt.min(RETRY_EXPONENT_MAXIMUM);
        let cache_key = cache_key.to_string();
        let task_lane = lane.clone();
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(delay_ms)).await;
            cx.update(|cx| {
                if let Some(task) = cx.default_global::<Self>().retry_task_by_lane.remove(&task_lane) {
                    task.detach();
                }
                let state = SaiWs::pagination_state(&cache_key, cx);
                if state.read(cx).state(edge).failure_kind != Some(SaiWsPaginationFailureKind::Transport) {
                    return;
                }
                Self::rearm(&cache_key, edge, cx);
                state.update(cx, |state, cx| state.clear_error(edge, cx));
            });
        });
        cx.default_global::<Self>().retry_task_by_lane.insert(lane, task);
    }

    fn reset_transport_retries(cache_key: &str, edge: SaiWsPaginationEdge, cx: &mut App) {
        let lane = RetryLane { cache_key: SaiAccountCacheScope::scoped_key(cache_key), edge };
        cx.default_global::<Self>().retry_attempts_by_lane.remove(&lane);
    }
}

pub type SaiPaginationLoadPage = Rc<dyn Fn(&mut Window, &mut App)>;

/// args of the pagination prefetch modifier, new takes the required ones and setters the rest
#[derive(Clone)]
pub struct SaiPaginationPrefetchOptions {
    index: usize,
    item_count: usize,
    cache_key: SharedString,
    relative_id: Option<String>,
    has_more: bool,
    is_enabled: bool,
    edge: SaiWsPaginationEdge,
    threshold: usize,
}

impl SaiPaginationPrefetchOptions {
    pub fn new(
        index: usize,
        item_count: usize,
        cache_key: impl Into<SharedString>,
        relative_id: Option<String>,
        has_more: bool,
    ) -> Self {
        Self {
            index,
            item_count,
            cache_key: cache_key.into(),
            relative_id,
            has_more,
            is_enabled: true,
            edge: SaiWsPaginationEdge::Trailing,
            threshold: 8,
        }
    }

    /// one probe row that asks for the next page when a client side filter left nothing to show
    pub fn drain_filtered_empty(
        filtered_item_count: usize,
        cache_key: impl Into<SharedString>,
        relative_id: Option<String>,
        has_more: bool,
    ) -> Self {
        let item_count = if filtered_item_count == 0 { 1 } else { 0 };
        Self::new(0, item_count, cache_key, relative_id, has_more).threshold(1)
    }

    pub fn is_enabled(mut self, is_enabled: bool) -> Self {
        self.is_enabled = is_enabled;
        self
    }

    pub fn edge(mut self, edge: SaiWsPaginationEdge) -> Self {
        self.edge = edge;
        self
    }

    pub fn threshold(mut self, threshold: usize) -> Self {
        self.threshold = threshold;
        self
    }

    fn is_within_threshold(&self) -> bool {
        let threshold = self.threshold.max(1);
        match self.edge {
            SaiWsPaginationEdge::Leading => self.index < threshold,
            SaiWsPaginationEdge::Trailing => self.item_count.saturating_sub(self.index) <= threshold,
        }
    }

    fn is_eligible(&self) -> bool {
        self.is_enabled
            && self.item_count > 0
            && self.index < self.item_count
            && self.has_more
            && self.is_within_threshold()
    }

    fn handle_appearance(&self, load_page: &SaiPaginationLoadPage, window: &mut Window, cx: &mut App) {
        if !self.is_eligible() {
            return;
        }
        let pagination_state = SaiWs::pagination_state(&self.cache_key, cx);
        if pagination_state.read(cx).state(self.edge).failure_kind == Some(SaiWsPaginationFailureKind::Transport) {
            SaiPaginationPrefetchGate::rearm(&self.cache_key, self.edge, cx);
            pagination_state.update(cx, |state, cx| state.clear_error(self.edge, cx));
        }
        self.evaluate(load_page, window, cx);
    }

    fn evaluate(&self, load_page: &SaiPaginationLoadPage, window: &mut Window, cx: &mut App) {
        if !self.is_eligible() {
            return;
        }
        let Some(token) = normalized(self.relative_id.as_deref()) else {
            return;
        };
        let pagination_state = SaiWs::pagination_state(&self.cache_key, cx);
        let (can_request, head_generation) = {
            let state = pagination_state.read(cx);
            (state.can_request_automatically(self.edge), state.head_generation())
        };
        if !can_request
            || !SaiPaginationPrefetchGate::claim(
                &self.cache_key,
                self.edge,
                &pagination_state,
                format!("{head_generation}|{token}"),
                cx,
            )
        {
            return;
        }
        load_page(window, cx);
    }
}

fn normalized(value: Option<&str>) -> Option<String> {
    let value = trimming_whitespaces_and_newlines(value?);
    if value.is_empty() { None } else { Some(value.to_string()) }
}

#[derive(IntoElement)]
pub struct SaiPaginationPrefetchModifier {
    id: ElementId,
    child: AnyElement,
    options: SaiPaginationPrefetchOptions,
    load_page: SaiPaginationLoadPage,
}

impl RenderOnce for SaiPaginationPrefetchModifier {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, child, options, load_page } = self;
        let watches_edge = options.is_enabled && options.has_more && options.is_within_threshold();
        let pagination_state = watches_edge.then(|| SaiWs::pagination_state(&options.cache_key, cx));
        let (edge_error, can_request_automatically, edge_failure_kind) = match &pagination_state {
            Some(state) => {
                let state = state.read(cx);
                let edge_state = state.state(options.edge);
                (edge_state.error.clone(), state.can_request_automatically(options.edge), edge_state.failure_kind)
            }
            None => (None, false, None),
        };
        let is_socket_connected = watches_edge && RiseTransportStatus::shown(cx).is_connected();

        let child_id = |name: &'static str| ElementId::from((id.clone(), name));

        {
            let (options, load_page) = (options.clone(), load_page.clone());
            on_change(child_id("is_enabled"), options.is_enabled, false, window, cx, move |_, is_enabled, window, cx| {
                if *is_enabled {
                    options.handle_appearance(&load_page, window, cx);
                }
            });
        }
        {
            let (options, load_page) = (options.clone(), load_page.clone());
            on_change(child_id("item_count"), options.item_count, false, window, cx, move |_, _, window, cx| {
                options.evaluate(&load_page, window, cx);
            });
        }
        {
            let (options, load_page) = (options.clone(), load_page.clone());
            on_change(
                child_id("relative_id"),
                options.relative_id.clone(),
                false,
                window,
                cx,
                move |_, _, window, cx| {
                    if watches_edge {
                        SaiPaginationPrefetchGate::reset_transport_retries(&options.cache_key, options.edge, cx);
                    }
                    options.evaluate(&load_page, window, cx);
                },
            );
        }
        {
            let (options, load_page) = (options.clone(), load_page.clone());
            on_change(child_id("has_more"), options.has_more, false, window, cx, move |_, has_more, window, cx| {
                if options.is_enabled && !*has_more {
                    SaiPaginationPrefetchGate::rearm(&options.cache_key, options.edge, cx);
                }
                options.evaluate(&load_page, window, cx);
            });
        }
        {
            let (options, load_page) = (options.clone(), load_page.clone());
            on_change(
                child_id("can_request_automatically"),
                can_request_automatically,
                false,
                window,
                cx,
                move |_, can_request, window, cx| {
                    if *can_request {
                        options.evaluate(&load_page, window, cx);
                    }
                },
            );
        }
        {
            let (options, load_page) = (options.clone(), load_page.clone());
            on_change(child_id("edge_error"), edge_error, false, window, cx, move |previous_error, error, window, cx| {
                if watches_edge && previous_error.is_some() && error.is_none() {
                    SaiPaginationPrefetchGate::rearm(&options.cache_key, options.edge, cx);
                    options.evaluate(&load_page, window, cx);
                }
                if watches_edge && error.is_some() && edge_failure_kind == Some(SaiWsPaginationFailureKind::Transport) {
                    SaiPaginationPrefetchGate::schedule_transport_retry(&options.cache_key, options.edge, cx);
                }
            });
        }
        {
            let options = options.clone();
            on_change(
                child_id("is_socket_connected"),
                is_socket_connected,
                false,
                window,
                cx,
                move |_, connected, _, cx| {
                    let Some(pagination_state) = pagination_state.filter(|_| *connected) else {
                        return;
                    };
                    if pagination_state.read(cx).state(options.edge).failure_kind
                        != Some(SaiWsPaginationFailureKind::Transport)
                    {
                        return;
                    }
                    SaiPaginationPrefetchGate::reset_transport_retries(&options.cache_key, options.edge, cx);
                    SaiPaginationPrefetchGate::rearm(&options.cache_key, options.edge, cx);
                    pagination_state.update(cx, |state, cx| state.clear_error(options.edge, cx));
                },
            );
        }

        child.on_appear(id, move |window, cx| options.handle_appearance(&load_page, window, cx))
    }
}

/// pagination prefetch modifiers for views
/// the id names the row, pass the domain id of the row
pub trait SaiPaginationPrefetchModifiers: IntoElement + Sized {
    fn sai_pagination_prefetch(
        self,
        id: impl Into<ElementId>,
        options: SaiPaginationPrefetchOptions,
        load_page: impl Fn(&mut Window, &mut App) + 'static,
    ) -> SaiPaginationPrefetchModifier {
        SaiPaginationPrefetchModifier {
            id: id.into(),
            child: self.into_any_element(),
            options,
            load_page: Rc::new(load_page),
        }
    }

    fn sai_pagination_drain_filtered_empty(
        self,
        id: impl Into<ElementId>,
        options: SaiPaginationPrefetchOptions,
        load_page: impl Fn(&mut Window, &mut App) + 'static,
    ) -> SaiPaginationPrefetchModifier {
        self.sai_pagination_prefetch(id, options, load_page)
    }
}

impl<E: IntoElement> SaiPaginationPrefetchModifiers for E {}
