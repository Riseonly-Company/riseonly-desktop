use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Context, ElementId, Entity, EntityId, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, Styled, Subscription, Task, WeakEntity, Window, div,
};
use rise_canon::RiseLimits;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_entities::RiseEntityViewKey;
use rise_engine::rise_store::rise_store_models::RiseStoreSnapshotRequest;
use tokio::task::JoinHandle;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::chats::rise_chat_repository::RiseChatRepository;
use crate::modules::chat::engine::chats::rise_chat_viewport_repository::RiseChatViewportSnapshot;
use crate::modules::chat::engine::core::rise_conversation_presentation::{
    RiseChatListKind, RiseConversationPresentation,
};
use crate::modules::chat::engine::core::rise_conversation_store_layout::{
    RiseConversationStoreLayout, RiseConversationStoreLayoutChatView,
};
use crate::modules::chat::engine::rise_conversation_domain::{
    RiseConversationDomain, RiseConversationDomainChatPageLoad, RiseConversationDomainRegistry,
};
use crate::modules::chat::shared::utils::message_preview::MessagePreview;
use crate::modules::chat::stores::chats::chat_actions::chat_folder_types::ChatFolder;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;

fn chat_hash_value(chat: &ChatInfo) -> i64 {
    let mut hasher = DefaultHasher::new();
    format!("{chat:?}").hash(&mut hasher);
    hasher.finish() as i64
}

#[derive(Clone)]
struct RiseChatViewportQuery {
    kind: RiseChatListKind,
    folder: Option<Arc<ChatFolder>>,
    contacts: Arc<HashSet<String>>,
}

impl RiseChatViewportQuery {
    async fn fetch(
        &self,
        domain: &Arc<RiseConversationDomain>,
        anchor_chat_id: Option<String>,
        before_count: i64,
        count: i64,
    ) -> anyhow::Result<RiseChatViewportSnapshot> {
        let domain = domain.clone();
        let query = self.clone();
        RiseAccountRuntime::current()
            .spawn(async move {
                domain
                    .chats
                    .viewport(
                        query.kind,
                        query.folder.as_deref(),
                        &query.contacts,
                        anchor_chat_id.as_deref(),
                        before_count,
                        count,
                    )
                    .await
            })
            .await?
    }
}

async fn first_viewport_anchor(
    domain: &Arc<RiseConversationDomain>,
    kind: RiseChatListKind,
    ids: Vec<String>,
) -> anyhow::Result<Option<String>> {
    let domain = domain.clone();
    RiseAccountRuntime::current()
        .spawn(async move { domain.chats.first_viewport_anchor(kind, &ids).await })
        .await?
}

struct RiseChatViewportObservationGuard(JoinHandle<()>);

impl Drop for RiseChatViewportObservationGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// each mounted folder owns one bounded window and one cancellable observation
/// notifies after every published window
pub struct RiseChatViewport {
    committed_snapshot: Option<Rc<RiseChatViewportSnapshot>>,
    revision: u64,
    domain: Option<Arc<RiseConversationDomain>>,
    kind: RiseChatListKind,
    folder: Option<Arc<ChatFolder>>,
    contacts: Arc<HashSet<String>>,
    anchor_id: Option<String>,
    before_count: i64,
    count: i64,
    generation: u64,
    query_revision: u64,
    observation: Option<Task<()>>,
    request: Option<Task<()>>,
    active: bool,
    visible_ids: Vec<String>,
    pending_refresh: bool,
    pending_advance: bool,
    epoch: Option<String>,
    versions: HashMap<String, i64>,
    geometry_versions: HashMap<String, i64>,
    indices: HashMap<String, usize>,
    last_remote_attempt: Option<String>,
    remote_failures: i32,
    remote_retry: Option<Task<()>>,
    retries_after_request: bool,
    remote_loading: bool,
}

impl Default for RiseChatViewport {
    fn default() -> Self {
        Self::new()
    }
}

impl RiseChatViewport {
    pub fn new() -> Self {
        Self {
            committed_snapshot: None,
            revision: 0,
            domain: None,
            kind: RiseChatListKind::Inbox,
            folder: None,
            contacts: Arc::new(HashSet::new()),
            anchor_id: None,
            before_count: 0,
            count: RiseChatRepository::default_page_limit(),
            generation: 0,
            query_revision: 0,
            observation: None,
            request: None,
            active: false,
            visible_ids: Vec::new(),
            pending_refresh: false,
            pending_advance: false,
            epoch: None,
            versions: HashMap::new(),
            geometry_versions: HashMap::new(),
            indices: HashMap::new(),
            last_remote_attempt: None,
            remote_failures: 0,
            remote_retry: None,
            retries_after_request: false,
            remote_loading: false,
        }
    }

    fn prefetch_rows() -> i64 {
        RiseLimits::client_value("chat_viewport_prefetch_rows")
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn is_active_domain(domain: &Arc<RiseConversationDomain>, cx: &mut App) -> bool {
        RiseConversationDomainRegistry::shared(cx)
            .read(cx)
            .active()
            .is_some_and(|active| Arc::ptr_eq(&active, domain))
    }

    /// call through viewport update since the registry needs mutable app
    pub fn snapshot(&self, cx: &mut App) -> Option<Rc<RiseChatViewportSnapshot>> {
        let value = self.committed_snapshot.clone();
        let domain = self.domain.as_ref()?;
        if !Self::is_active_domain(domain, cx)
            || !SaiAccountCacheScope::is_current_account_epoch(Some(domain.account_id.as_str()), self.epoch.as_deref())
        {
            return None;
        }
        value
    }

    pub fn chats(&self, cx: &mut App) -> Vec<ChatInfo> {
        let Some(snapshot) = self.snapshot(cx) else {
            return Vec::new();
        };
        let services = ChatServicesStore::shared(cx);
        let deleting = &services.read(cx).pending_delete_chat_ids;
        snapshot
            .rows
            .iter()
            .filter(|row| deleting.is_empty() || !deleting.contains(&row.chat.id))
            .map(|row| row.chat.clone())
            .collect()
    }

    pub fn has_before(&self, cx: &mut App) -> bool {
        self.snapshot(cx).is_some_and(|snapshot| snapshot.has_before)
    }

    pub fn content_version(&self, chat: &ChatInfo) -> i64 {
        let mut hash = DefaultHasher::new();
        self.versions.get(&chat.id).copied().unwrap_or_else(|| chat_hash_value(chat)).hash(&mut hash);
        chat.is_muted.hash(&mut hash);
        chat.is_pinned.hash(&mut hash);
        chat.unread_count.hash(&mut hash);
        hash.finish() as i64
    }

    pub fn geometry_version(&self, chat: &ChatInfo) -> i64 {
        self.geometry_versions.get(&chat.id).copied().unwrap_or_else(|| self.content_version(chat))
    }

    pub fn cache_detail(&self, chat: &ChatInfo, cx: &mut App) {
        let Some(domain) = self.domain.clone() else {
            return;
        };
        if !self.is_current(self.generation, &domain, cx) {
            return;
        }
        let is_archived = self.kind == RiseChatListKind::Archive;
        RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| {
            presentation.publish_chat_detail_model(
                chat.clone(),
                &chat.id,
                &domain.account_id,
                Some(is_archived),
                cx,
            );
        });
    }

    pub fn observe(
        &mut self,
        kind: RiseChatListKind,
        folder: Option<ChatFolder>,
        contacts: Arc<HashSet<String>>,
        anchor_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.stop(cx);
        let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() else {
            return;
        };
        if !SaiAccountCacheScope::is_current_account(Some(domain.account_id.as_str())) {
            return;
        }
        self.domain = Some(domain.clone());
        self.kind = kind;
        self.folder = folder.map(Arc::new);
        self.contacts = contacts;
        self.epoch = SaiAccountCacheScope::active_epoch();
        self.count = RiseChatRepository::default_page_limit();
        let token = self.generation;
        let has_folder = self.folder.is_some();
        self.observation = Some(cx.spawn(async move |this, cx| {
            Self::run_observation(this.clone(), domain, token, kind, has_folder, anchor_ids, cx).await;
            let _ = this.update(cx, |this, cx| {
                if token == this.generation {
                    this.halt(cx);
                }
            });
        }));
    }

    async fn run_observation(
        this: WeakEntity<Self>,
        domain: Arc<RiseConversationDomain>,
        token: u64,
        kind: RiseChatListKind,
        has_folder: bool,
        anchor_ids: Vec<String>,
        cx: &mut AsyncApp,
    ) {
        let Ok(initial_anchor) = first_viewport_anchor(&domain, kind, anchor_ids).await else {
            return;
        };
        let is_current = this
            .update(cx, |this, cx| {
                if !this.is_current(token, &domain, cx) {
                    return false;
                }
                this.before_count = if initial_anchor.is_none() { 0 } else { this.count / 2 };
                this.anchor_id = initial_anchor;
                true
            })
            .unwrap_or(false);
        if !is_current {
            return;
        }
        let mut views: HashSet<RiseConversationStoreLayoutChatView> = HashSet::from([kind.store_view()]);
        if has_folder && kind == RiseChatListKind::Inbox {
            views.insert(RiseConversationStoreLayoutChatView::Archive);
        }
        let request = RiseStoreSnapshotRequest {
            entity_view_states: views
                .into_iter()
                .map(|view| RiseEntityViewKey::new(RiseConversationStoreLayout::chats(), view.raw_value()))
                .collect(),
            ..RiseStoreSnapshotRequest::default()
        };
        let (mut sender, mut receiver) = mpsc::channel::<()>(1);
        let engine = domain.engine.clone();
        let forwarder = RiseAccountRuntime::current().spawn(async move {
            let Ok(observation) = engine.observe(request).await else {
                return;
            };
            if sender.try_send(()).is_err() {
                return;
            }
            let mut updates = observation.updates.make_async_iterator();
            while updates.next().await.is_some() {
                if let Err(error) = sender.try_send(())
                    && error.is_disconnected()
                {
                    return;
                }
            }
        });
        let _guard = RiseChatViewportObservationGuard(forwarder);
        if receiver.next().await.is_none() {
            return;
        }
        let is_current = this.read_with(cx, |this, _| token == this.generation).unwrap_or(false);
        x253_mark(|| format!("viewport {} observing gen={token} current={is_current}", kind.store_view().raw_value()));
        if !is_current {
            return;
        }
        Self::refresh(this.clone(), token, cx).await;
        while receiver.next().await.is_some() {
            let decision = this
                .update(cx, |this, _| {
                    if token != this.generation {
                        return None;
                    }
                    let (active, blocks) = (this.active, this.blocks_refresh());
                    x253_mark(|| {
                        format!("viewport {} signal active={active} blocks={blocks}", kind.store_view().raw_value())
                    });
                    if !this.active || this.blocks_refresh() {
                        this.pending_refresh = true;
                        return Some(false);
                    }
                    Some(true)
                })
                .unwrap_or(None);
            match decision {
                None => {
                    x253_mark(|| format!("viewport {} signal stale gen={token}", kind.store_view().raw_value()));
                    return;
                }
                Some(false) => {}
                Some(true) => Self::refresh(this.clone(), token, cx).await,
            }
        }
    }

    pub fn set_active(&mut self, value: bool, cx: &mut Context<Self>) {
        let pending = self.pending_refresh;
        x253_mark(|| format!("viewport set_active={value} pending={pending}"));
        self.active = value;
        if !value {
            return;
        }
        self.last_remote_attempt = None;
        if self.pending_refresh {
            self.pending_refresh = false;
            self.spawn_refresh(cx);
        } else {
            self.advance_if_needed(cx);
        }
    }

    pub fn visible(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        self.visible_ids = ids;
        self.advance_if_needed(cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.observation = None;
        self.halt(cx);
    }

    fn halt(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        x253_mark(|| format!("viewport halt gen={generation}"));
        self.generation = self.generation.wrapping_add(1);
        self.request = None;
        self.domain = None;
        self.pending_refresh = false;
        self.pending_advance = false;
        self.visible_ids.clear();
        self.committed_snapshot = None;
        self.versions.clear();
        self.geometry_versions.clear();
        self.last_remote_attempt = None;
        self.remote_retry = None;
        self.remote_failures = 0;
        self.retries_after_request = false;
        self.remote_loading = false;
        self.indices.clear();
        cx.notify();
    }

    fn blocks_refresh(&self) -> bool {
        self.request.is_some() && !self.remote_loading
    }

    fn is_current(&self, token: u64, domain: &Arc<RiseConversationDomain>, cx: &mut App) -> bool {
        token == self.generation
            && Self::is_active_domain(domain, cx)
            && SaiAccountCacheScope::is_current_account_epoch(Some(domain.account_id.as_str()), self.epoch.as_deref())
    }

    fn query(&self) -> RiseChatViewportQuery {
        RiseChatViewportQuery { kind: self.kind, folder: self.folder.clone(), contacts: self.contacts.clone() }
    }

    fn spawn_refresh(&self, cx: &mut Context<Self>) {
        let token = self.generation;
        cx.spawn(async move |this, cx| Self::refresh(this, token, cx).await).detach();
    }

    async fn refresh(this: WeakEntity<Self>, token: u64, cx: &mut AsyncApp) {
        let start = this
            .update(cx, |this, cx| {
                let domain = this.domain.clone()?;
                if !this.is_current(token, &domain, cx) {
                    return None;
                }
                if this.blocks_refresh() {
                    this.pending_refresh = true;
                    return None;
                }
                this.query_revision = this.query_revision.wrapping_add(1);
                Some((
                    domain,
                    this.query_revision,
                    this.query(),
                    this.anchor_id.clone(),
                    this.before_count,
                    this.count,
                ))
            })
            .unwrap_or(None);
        let Some((domain, query_revision, query, anchor_id, before_count, count)) = start else {
            return;
        };
        let holds = |this: &WeakEntity<Self>, cx: &mut AsyncApp| {
            this.update(cx, |this, cx| this.is_current(token, &domain, cx) && query_revision == this.query_revision)
                .unwrap_or(false)
        };
        let Ok(mut next) = query.fetch(&domain, anchor_id, before_count, count).await else {
            return;
        };
        if !holds(&this, cx) {
            return;
        }
        let on_screen = this
            .read_with(cx, |this, _| {
                if this.anchor_id.is_none() {
                    return Vec::new();
                }
                let on_screen: Vec<String> =
                    this.visible_ids.iter().filter(|id| this.indices.contains_key(*id)).cloned().collect();
                if on_screen.iter().any(|id| next.rows.iter().any(|row| &row.chat.id == id)) {
                    return Vec::new();
                }
                on_screen
            })
            .unwrap_or_default();
        if !on_screen.is_empty() {
            let Ok(anchor) = first_viewport_anchor(&domain, query.kind, on_screen).await else {
                return;
            };
            if let Some(anchor) = anchor {
                let before = count / 2;
                let Ok(anchored) = query.fetch(&domain, Some(anchor.clone()), before, count).await else {
                    return;
                };
                next = anchored;
                if !holds(&this, cx) {
                    return;
                }
                let _ = this.update(cx, |this, _| {
                    this.anchor_id = Some(anchor);
                    this.before_count = before;
                });
            }
        }
        let messages: Vec<_> = next.rows.iter().filter_map(|row| row.chat.last_message.clone()).collect();
        MessagePreview::prepare_list_snippets(&messages, cx).await;
        if !holds(&this, cx) {
            return;
        }
        let rows = next.rows.len();
        let head = next.rows.first().map(|row| row.chat.id.clone()).unwrap_or_default();
        x253_mark(|| format!("viewport {} refresh rows={rows} head={head}", query.kind.store_view().raw_value()));
        let _ = this.update(cx, |this, cx| {
            let revision = this.revision;
            this.publish(next, cx);
            let published = this.revision != revision;
            x253_mark(|| format!("viewport publish changed={published}"));
            this.advance_if_needed(cx);
        });
    }

    fn publish(&mut self, next: RiseChatViewportSnapshot, cx: &mut Context<Self>) {
        if !next.has_before {
            self.anchor_id = None;
            self.before_count = 0;
        }
        if let Some(old) = self.snapshot(cx)
            && old.rows.len() == next.rows.len()
            && old
                .rows
                .iter()
                .zip(&next.rows)
                .all(|(old, next)| old.local_id == next.local_id && old.content_version == next.content_version)
            && old.has_before == next.has_before
            && old.has_after == next.has_after
            && old.has_remote_after == next.has_remote_after
            && old.cursor == next.cursor
        {
            return;
        }
        let chats: Vec<ChatInfo> = next.rows.iter().map(|row| row.chat.clone()).collect();
        self.versions = next.rows.iter().map(|row| (row.chat.id.clone(), row.content_version)).collect();
        self.geometry_versions = next.rows.iter().map(|row| (row.chat.id.clone(), row.geometry_version)).collect();
        self.indices = next.rows.iter().enumerate().map(|(index, row)| (row.chat.id.clone(), index)).collect();
        self.committed_snapshot = Some(Rc::new(next));
        RiseConversationPresentation::shared(cx)
            .update(cx, |presentation, cx| presentation.remember_favourites_chat(&chats, cx));
        self.revision = self.revision.wrapping_add(1);
        cx.notify();
    }

    fn advance_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.active {
            return;
        }
        let Some(domain) = self.domain.clone() else {
            return;
        };
        let Some(snapshot) = self.snapshot(cx) else {
            return;
        };
        if self.request.is_some() {
            self.pending_advance = true;
            return;
        }
        self.pending_advance = false;
        let rows = &snapshot.rows;
        let row_count = rows.len() as i64;
        let prefetch_rows = Self::prefetch_rows();
        let visible_indices = self.visible_ids.iter().filter_map(|id| self.indices.get(id).copied());
        let near_start = visible_indices.clone().min().is_some_and(|index| (index as i64) < prefetch_rows);
        let near_end = visible_indices.max().map_or(rows.is_empty(), |index| index as i64 >= row_count - prefetch_rows);
        let earlier = near_start && snapshot.has_before;
        if !earlier && !(near_end && (snapshot.has_after || snapshot.has_remote_after)) {
            return;
        }
        let needs_remote = !earlier && !snapshot.has_after && snapshot.has_remote_after;
        let remote_key = format!("{}|{}", snapshot.cursor.as_deref().unwrap_or("initial"), rows.last().map(|row| row.chat.id.as_str()).unwrap_or_default());
        if needs_remote {
            if self.last_remote_attempt.as_deref() == Some(remote_key.as_str()) {
                return;
            }
            self.last_remote_attempt = Some(remote_key);
        }
        let cap = RiseChatRepository::default_presentation_limit();
        let stride = RiseChatRepository::default_page_limit();
        let new_count = cap.min(self.count.max(row_count) + stride);
        let (next_anchor, next_before) = if earlier {
            (rows.first().map(|row| row.chat.id.clone()), stride)
        } else {
            let removed = (row_count + stride - cap).max(0) as usize;
            (rows.get(removed.min(rows.len().saturating_sub(1))).map(|row| row.chat.id.clone()), 0)
        };
        let token = self.generation;
        self.query_revision = self.query_revision.wrapping_add(1);
        let query_revision = self.query_revision;
        let query = self.query();
        self.remote_loading = needs_remote;
        self.request = Some(cx.spawn(async move |this, cx| {
            Self::run_advance(
                this.clone(),
                domain,
                token,
                query_revision,
                query,
                needs_remote,
                next_anchor,
                next_before,
                new_count,
                cx,
            )
            .await;
            let _ = this.update(cx, |this, cx| this.finish_request(token, cx));
        }));
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_advance(
        this: WeakEntity<Self>,
        domain: Arc<RiseConversationDomain>,
        token: u64,
        query_revision: u64,
        query: RiseChatViewportQuery,
        needs_remote: bool,
        next_anchor: Option<String>,
        next_before: i64,
        new_count: i64,
        cx: &mut AsyncApp,
    ) {
        let mut query_revision = query_revision;
        if needs_remote {
            let kind = query.kind;
            let loading = domain.clone();
            let outcome = RiseAccountRuntime::current()
                .spawn(async move { loading.load_more_chats(kind).await })
                .await
                .unwrap_or(RiseConversationDomainChatPageLoad::Failed);
            let is_current = this
                .update(cx, |this, cx| {
                    if !this.is_current(token, &domain, cx) {
                        return false;
                    }
                    this.remote_loading = false;
                    match outcome {
                        RiseConversationDomainChatPageLoad::Loaded => this.remote_failures = 0,
                        RiseConversationDomainChatPageLoad::Busy => this.last_remote_attempt = None,
                        RiseConversationDomainChatPageLoad::Failed => this.retry_remote(token, cx),
                    }
                    true
                })
                .unwrap_or(false);
            if !is_current {
                return;
            }
        }
        let may_query = this
            .update(cx, |this, cx| {
                if !this.is_current(token, &domain, cx) {
                    return false;
                }
                if !this.active {
                    this.pending_refresh = true;
                    return false;
                }
                if needs_remote {
                    this.query_revision = this.query_revision.wrapping_add(1);
                    query_revision = this.query_revision;
                }
                true
            })
            .unwrap_or(false);
        if !may_query {
            return;
        }
        let Ok(next) = query.fetch(&domain, next_anchor.clone(), next_before, new_count).await else {
            return;
        };
        let holds = this
            .update(cx, |this, cx| {
                if !this.is_current(token, &domain, cx) || query_revision != this.query_revision {
                    return false;
                }
                this.anchor_id = next_anchor;
                this.before_count = next_before;
                this.count = new_count;
                true
            })
            .unwrap_or(false);
        if !holds {
            return;
        }
        let messages: Vec<_> = next.rows.iter().filter_map(|row| row.chat.last_message.clone()).collect();
        MessagePreview::prepare_list_snippets(&messages, cx).await;
        let _ = this.update(cx, |this, cx| {
            if this.is_current(token, &domain, cx) && query_revision == this.query_revision {
                this.publish(next, cx);
            }
        });
    }

    fn finish_request(&mut self, token: u64, cx: &mut Context<Self>) {
        if token != self.generation {
            return;
        }
        if let Some(request) = self.request.take() {
            request.detach();
        }
        if self.pending_refresh {
            self.pending_refresh = false;
            self.spawn_refresh(cx);
        } else if self.pending_advance {
            self.advance_if_needed(cx);
        }
        if self.retries_after_request {
            self.retries_after_request = false;
            self.last_remote_attempt = None;
            self.advance_if_needed(cx);
        }
    }

    fn retry_remote(&mut self, token: u64, cx: &mut Context<Self>) {
        self.remote_failures += 1;
        self.retries_after_request = false;
        let delay = (2.0 * 2_f64.powi(self.remote_failures - 1)).min(30.0);
        self.remote_retry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
            let _ = this.update(cx, |this, cx| {
                if token != this.generation {
                    return;
                }
                if this.request.is_some() {
                    this.retries_after_request = true;
                    return;
                }
                this.last_remote_attempt = None;
                this.advance_if_needed(cx);
            });
        }));
    }
}

/// what the reader task id reads from the stores
#[derive(Clone, PartialEq)]
struct RiseChatViewportStoresScope {
    account: Option<String>,
    epoch: Option<String>,
    engine_active: bool,
    contacts: Arc<HashSet<String>>,
}

impl RiseChatViewportStoresScope {
    fn differs_from_stores(&self, cx: &mut App) -> bool {
        let account = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let engine_active = RiseConversationPresentation::shared(cx).read(cx).is_active();
        let services = ChatServicesStore::shared(cx);
        self.account != account
            || self.epoch != SaiAccountCacheScope::active_epoch()
            || self.engine_active != engine_active
            || *self.contacts != services.read(cx).contact_ids
    }

    fn current(cx: &mut App) -> Self {
        let account = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let engine_active = RiseConversationPresentation::shared(cx).read(cx).is_active();
        let services = ChatServicesStore::shared(cx);
        let contacts = Arc::new(services.read(cx).contact_ids.clone());
        Self { account, epoch: SaiAccountCacheScope::active_epoch(), engine_active, contacts }
    }
}

#[derive(Clone, PartialEq)]
struct RiseChatViewportReaderScope {
    stores: RiseChatViewportStoresScope,
    kind: RiseChatListKind,
    folder: Option<ChatFolder>,
}

struct RiseChatViewportReaderState {
    viewport: Entity<RiseChatViewport>,
    stores: RiseChatViewportStoresScope,
    observed_scope: Option<RiseChatViewportReaderScope>,
    delivered_active: Option<bool>,
    _subscriptions: Vec<Subscription>,
}

impl RiseChatViewportReaderState {
    fn stores_changed(&mut self, view: EntityId, cx: &mut Context<Self>) {
        if !self.stores.differs_from_stores(cx) {
            return;
        }
        self.stores = RiseChatViewportStoresScope::current(cx);
        App::notify(cx, view);
    }
}

fn reader_child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

/// each mounted folder owns one bounded window and one cancellable observation
/// unmounted folders keep only their bookmark in the viewport store
#[derive(IntoElement)]
pub struct RiseChatViewportReader {
    id: ElementId,
    kind: RiseChatListKind,
    folder: Option<ChatFolder>,
    active: bool,
    restore_chat_ids: Vec<String>,
    content: Box<dyn FnOnce(Entity<RiseChatViewport>, &mut Window, &mut App) -> AnyElement>,
}

impl RiseChatViewportReader {
    pub fn new(
        id: impl Into<ElementId>,
        kind: RiseChatListKind,
        content: impl FnOnce(Entity<RiseChatViewport>, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            folder: None,
            active: true,
            restore_chat_ids: Vec::new(),
            content: Box::new(content),
        }
    }

    pub fn folder(mut self, folder: Option<ChatFolder>) -> Self {
        self.folder = folder;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn restore_chat_ids(mut self, restore_chat_ids: Vec<String>) -> Self {
        self.restore_chat_ids = restore_chat_ids;
        self
    }
}

impl RenderOnce for RiseChatViewportReader {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.current_view();
        let state = window.use_keyed_state::<RiseChatViewportReaderState>(self.id.clone(), cx, move |_, cx| {
            let viewport = cx.new(|_| RiseChatViewport::new());
            let auth = AuthServicesStore::shared(cx);
            let presentation = RiseConversationPresentation::shared(cx);
            let services = ChatServicesStore::shared(cx);
            let subscriptions = vec![
                cx.observe(&auth, move |state: &mut RiseChatViewportReaderState, _, cx| state.stores_changed(view, cx)),
                cx.observe(&presentation, move |state: &mut RiseChatViewportReaderState, _, cx| {
                    state.stores_changed(view, cx)
                }),
                cx.observe(&services, move |state: &mut RiseChatViewportReaderState, _, cx| {
                    state.stores_changed(view, cx)
                }),
                cx.observe(&viewport, move |_: &mut RiseChatViewportReaderState, _, cx| App::notify(cx, view)),
            ];
            RiseChatViewportReaderState {
                viewport,
                stores: RiseChatViewportStoresScope::current(cx),
                observed_scope: None,
                delivered_active: None,
                _subscriptions: subscriptions,
            }
        });
        let (viewport, stores) = {
            let state = state.read(cx);
            (state.viewport.clone(), state.stores.clone())
        };
        let identity = format!(
            "{}|{}",
            stores.account.as_deref().unwrap_or_default(),
            stores.epoch.as_deref().unwrap_or_default()
        );
        let contacts = stores.contacts.clone();
        let scope = RiseChatViewportReaderScope { stores, kind: self.kind, folder: self.folder.clone() };
        let active = self.active;
        let (scope_changed, active_changed) = state.update(cx, |state, _| {
            let scope_changed = state.observed_scope.as_ref() != Some(&scope);
            let active_changed = state.delivered_active != Some(active);
            state.observed_scope = Some(scope);
            state.delivered_active = Some(active);
            (scope_changed, active_changed)
        });
        if active_changed {
            let viewport = viewport.clone();
            window.defer(cx, move |_, cx| viewport.update(cx, |viewport, cx| viewport.set_active(active, cx)));
        }
        if scope_changed {
            let viewport = viewport.clone();
            let kind = self.kind;
            let folder = self.folder;
            let anchor_ids = self.restore_chat_ids;
            window.defer(cx, move |_, cx| {
                viewport.update(cx, |viewport, cx| viewport.observe(kind, folder, contacts, anchor_ids, cx));
            });
        }
        let content = (self.content)(viewport, window, cx);
        div()
            .id(ElementId::NamedChild(Arc::new(reader_child_id(&self.id, "content")), SharedString::from(identity)))
            .size_full()
            .child(content)
    }
}

fn x253_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X253", "{}", stage());
}
