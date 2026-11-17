use std::collections::HashMap;
use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, Global, IntoElement, RenderOnce, Subscription, Window};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change};

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

struct ChatProfilePaginationPrefetchGateClaim {
    request_key: String,
    failed_cursor: Option<String>,
}

#[derive(Default)]
struct ChatProfilePaginationPrefetchGate {
    claim_by_scope: HashMap<String, ChatProfilePaginationPrefetchGateClaim>,
    order: Vec<String>,
}

impl Global for ChatProfilePaginationPrefetchGate {}

impl ChatProfilePaginationPrefetchGate {
    fn limit() -> i64 {
        RiseLimits::client_value("chat_profile_pagination_prefetch_gate_limit")
    }

    fn claim(scope: &str, cursor: &str, revision: u64, has_error: bool, automatic: bool, cx: &mut App) -> bool {
        let key = SaiAccountCacheScope::scoped_key(scope);
        let gate = cx.default_global::<Self>();
        let previous = gate.claim_by_scope.get(&key);
        let request_key = format!("{revision}|{cursor}");
        if previous.is_some_and(|previous| previous.request_key == request_key) {
            return false;
        }
        if automatic && has_error && previous.is_some_and(|previous| previous.failed_cursor.as_deref() == Some(cursor))
        {
            return false;
        }
        gate.claim_by_scope.insert(
            key.clone(),
            ChatProfilePaginationPrefetchGateClaim {
                request_key,
                failed_cursor: has_error.then(|| cursor.to_owned()),
            },
        );
        gate.order.retain(|existing| *existing != key);
        gate.order.push(key);
        let limit = usize::try_from(Self::limit()).unwrap_or(0);
        while gate.order.len() > limit {
            let oldest = gate.order.remove(0);
            gate.claim_by_scope.remove(&oldest);
        }
        true
    }

    fn release(scope: &str, cx: &mut App) {
        let key = SaiAccountCacheScope::scoped_key(scope);
        let gate = cx.default_global::<Self>();
        gate.claim_by_scope.remove(&key);
        gate.order.retain(|existing| *existing != key);
    }
}

type ChatProfilePaginationLoadPage = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
struct ChatProfilePaginationPrefetchEvaluation {
    index: i64,
    item_count: i64,
    scope: String,
    relative_id: Option<String>,
    has_more: bool,
    revision: u64,
    has_error: bool,
    threshold: i64,
    load_page: ChatProfilePaginationLoadPage,
}

impl ChatProfilePaginationPrefetchEvaluation {
    fn is_within_threshold(&self) -> bool {
        self.item_count > 0
            && self.index >= 0
            && self.index < self.item_count
            && self.item_count - self.index <= self.threshold.max(1)
    }

    fn evaluate(&self, automatic: bool, window: &mut Window, cx: &mut App) {
        if !self.is_within_threshold() || !self.has_more {
            return;
        }
        let Some(cursor) = self.relative_id.as_deref().map(trimming_whitespaces_and_newlines) else {
            return;
        };
        if cursor.is_empty() {
            return;
        }
        if !ChatProfilePaginationPrefetchGate::claim(
            &self.scope,
            cursor,
            self.revision,
            self.has_error,
            automatic,
            cx,
        ) {
            return;
        }
        (self.load_page)(window, cx);
    }
}

#[derive(IntoElement)]
struct ChatProfilePaginationPrefetchModifier {
    id: ElementId,
    child: AnyElement,
    evaluation: ChatProfilePaginationPrefetchEvaluation,
}

impl RenderOnce for ChatProfilePaginationPrefetchModifier {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, child, evaluation } = self;
        let child_id = |name: &'static str| ElementId::from((id.clone(), name));

        let watches_socket = evaluation.has_more && evaluation.is_within_threshold();
        let is_socket_connected = if watches_socket {
            let host = window.current_view();
            let _socket_observer = window.use_keyed_state(child_id("socket-observer"), cx, move |_, cx| -> Subscription {
                let shown = RiseTransportStatus::shown_store(cx);
                App::observe(cx, &shown, move |_, cx: &mut App| cx.notify(host))
            });
            RiseTransportStatus::shown(cx).is_connected()
        } else {
            false
        };

        {
            let evaluation = evaluation.clone();
            on_change(child_id("item-count"), evaluation.item_count, false, window, cx, move |_, _, window, cx| {
                evaluation.evaluate(true, window, cx);
            });
        }
        {
            let evaluation = evaluation.clone();
            on_change(
                child_id("relative-id"),
                evaluation.relative_id.clone(),
                false,
                window,
                cx,
                move |_, _, window, cx| {
                    evaluation.evaluate(true, window, cx);
                },
            );
        }
        {
            let evaluation = evaluation.clone();
            on_change(child_id("has-more"), evaluation.has_more, false, window, cx, move |_, _, window, cx| {
                evaluation.evaluate(true, window, cx);
            });
        }
        {
            let evaluation = evaluation.clone();
            on_change(child_id("revision"), evaluation.revision, false, window, cx, move |_, _, window, cx| {
                evaluation.evaluate(true, window, cx);
            });
        }
        {
            let evaluation = evaluation.clone();
            on_change(
                child_id("is-socket-connected"),
                is_socket_connected,
                false,
                window,
                cx,
                move |_, connected, window, cx| {
                    if !*connected {
                        return;
                    }
                    ChatProfilePaginationPrefetchGate::release(&evaluation.scope, cx);
                    evaluation.evaluate(false, window, cx);
                },
            );
        }

        child.on_appear(id, move |window, cx| evaluation.evaluate(false, window, cx))
    }
}

/// adds the pagination prefetch modifier to rows, the id is the domain id of the row
pub(crate) trait ChatProfilePaginationPrefetchModifiers: IntoElement + Sized {
    const DEFAULT_THRESHOLD: i64 = 8;

    /// threshold none means the default one
    #[allow(clippy::too_many_arguments)]
    fn chat_profile_pagination_prefetch(
        self,
        id: impl Into<ElementId>,
        index: i64,
        item_count: usize,
        cache_id: &str,
        scope: &str,
        threshold: Option<i64>,
        load_page: impl Fn(&mut Window, &mut App) + 'static,
        cx: &mut App,
    ) -> AnyElement {
        let threshold = threshold.unwrap_or(Self::DEFAULT_THRESHOLD);
        let services = MessageServicesStore::shared(cx);
        let page = services.update(cx, |services, cx| services.tab_pagination(cache_id, scope, cx));
        if RiseConversationPresentation::shared(cx).read(cx).is_active() {
            let revision =
                services.update(cx, |services, cx| services.message_tab_presentation_revision(cache_id, scope, cx));
            let has_error = services.update(cx, |services, cx| services.message_tab_error(cache_id, scope, cx)).is_some();
            ChatProfilePaginationPrefetchModifier {
                id: id.into(),
                child: self.into_any_element(),
                evaluation: ChatProfilePaginationPrefetchEvaluation {
                    index,
                    item_count: i64::try_from(item_count).unwrap_or(i64::MAX),
                    scope: format!("{cache_id}:{scope}"),
                    relative_id: page.relative_id,
                    has_more: page.has_more,
                    revision,
                    has_error,
                    threshold,
                    load_page: Rc::new(load_page),
                },
            }
            .into_any_element()
        } else {
            let cache_key = services.read(cx).tab_cache_key(cache_id, scope);
            self.sai_pagination_prefetch(
                id,
                SaiPaginationPrefetchOptions::new(
                    usize::try_from(index).unwrap_or(usize::MAX),
                    item_count,
                    cache_key,
                    page.relative_id,
                    page.has_more,
                )
                .threshold(usize::try_from(threshold).unwrap_or(0)),
                load_page,
            )
            .into_any_element()
        }
    }
}

impl<E: IntoElement> ChatProfilePaginationPrefetchModifiers for E {}
