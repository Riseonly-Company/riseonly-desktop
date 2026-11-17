use std::collections::{BTreeSet, HashMap, HashSet};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Context, Entity, Subscription, Task};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_energy::rise_energy_saving::{RiseEnergySavingDidChange, RiseEnergySavingStore};
use rise_platform::power::POWER_STATE_DID_CHANGE;
use rise_platform::process_info::THERMAL_STATE_DID_CHANGE;
use serde_json::Value;

use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_disk_cache::SaiWsDiskCache;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::media::rise_media_foreground_gate::{RISE_MEDIA_FOREGROUND_DID_CHANGE, RiseMediaForegroundGate};
use crate::core::ui::clever_image_disk_cache::CleverImagePrefetchPolicy;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

/// entry of the history warmup input signature
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSignatureEntry {
    pub index: i64,
    pub chat_id: String,
    pub topics_enabled: bool,
}

/// input signature of the history warmup policy
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSignature {
    pub entries: Vec<InputSignatureEntry>,
}

pub enum ChatListHistoryWarmupPolicy {}

impl ChatListHistoryWarmupPolicy {
    pub fn maximum_visible_candidates() -> i64 {
        RiseLimits::client_value("chat_list_history_warmup_visible_candidates")
    }

    pub fn maximum_scan_distance() -> i64 {
        RiseLimits::client_value("chat_list_history_warmup_scan_entries")
    }

    pub fn maximum_forum_candidates() -> i64 {
        RiseLimits::client_value("chat_list_history_warmup_forum_candidates")
    }

    /// limit none means the default maximum_scan_distance
    pub fn candidate_indices(item_count: i64, center: i64, previous_center: Option<i64>, limit: Option<i64>) -> Vec<i64> {
        let limit = limit.unwrap_or_else(Self::maximum_scan_distance);
        if item_count <= 0 || center < 0 || center >= item_count || limit <= 0 {
            return Vec::new();
        }

        let limit = limit as usize;
        let mut result: Vec<i64> = Vec::with_capacity(limit.min(item_count as usize));
        let mut seen: HashSet<i64> = HashSet::with_capacity(limit.min(item_count as usize));
        let mut append = |index: i64| {
            if result.len() < limit && index >= 0 && index < item_count && seen.insert(index) {
                result.push(index);
            }
        };

        append(center);
        let direction = match previous_center {
            Some(previous_center) if previous_center != center => {
                Some(if center > previous_center { 1 } else { -1 })
            }
            _ => None,
        };

        for distance in 1..=item_count {
            if let Some(direction) = direction {
                append(center + distance * direction);
                append(center - distance * direction);
            } else {
                append(center + distance);
                append(center - distance);
            }
        }
        result
    }

    /// limit none means the default maximum_forum_candidates
    pub fn forum_candidate_indices(
        item_count: i64,
        center: i64,
        previous_center: Option<i64>,
        limit: Option<i64>,
        is_forum_at_index: impl Fn(i64) -> bool,
    ) -> Vec<i64> {
        let limit = limit.unwrap_or_else(Self::maximum_forum_candidates);
        if limit <= 0 {
            return Vec::new();
        }
        let nearby = Self::candidate_indices(item_count, center, previous_center, Some(Self::maximum_scan_distance()));
        let limit = limit as usize;
        let mut result: Vec<i64> = Vec::with_capacity(limit.min(nearby.len()));
        for index in nearby {
            if !is_forum_at_index(index) {
                continue;
            }
            result.push(index);
            if result.len() == limit {
                break;
            }
        }
        result
    }

    pub fn bounded_input_signature(
        item_count: i64,
        center: i64,
        chat_id_at_index: impl Fn(i64) -> String,
        topics_enabled_at_index: impl Fn(i64) -> bool,
    ) -> InputSignature {
        if item_count <= 0 || center < 0 || center >= item_count {
            return InputSignature { entries: Vec::new() };
        }

        let maximum_scan_distance = Self::maximum_scan_distance();
        let mut indices: BTreeSet<i64> = (0..item_count.min(maximum_scan_distance)).collect();
        let lower = (center - maximum_scan_distance).max(0);
        let upper = item_count.min(center + maximum_scan_distance + 1);
        indices.extend(lower..upper);
        InputSignature {
            entries: indices
                .into_iter()
                .map(|index| InputSignatureEntry {
                    index,
                    chat_id: chat_id_at_index(index),
                    topics_enabled: topics_enabled_at_index(index),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatListForumPresentationReplacementPolicyAction {
    TransferToNewPresentation,
    ReleaseDisjointPayloads,
    Discard,
}

pub enum ChatListForumPresentationReplacementPolicy {}

impl ChatListForumPresentationReplacementPolicy {
    #[allow(clippy::too_many_arguments)]
    pub fn action(
        old_chat_id: &str,
        new_chat_id: &str,
        old_account_user_id: Option<&str>,
        old_account_epoch: Option<&str>,
        new_account_user_id: Option<&str>,
        new_account_epoch: Option<&str>,
        old_lifecycle_generation: u64,
        current_lifecycle_generation: u64,
    ) -> ChatListForumPresentationReplacementPolicyAction {
        if old_lifecycle_generation != current_lifecycle_generation
            || old_account_user_id != new_account_user_id
            || old_account_epoch != new_account_epoch
        {
            return ChatListForumPresentationReplacementPolicyAction::Discard;
        }
        if old_chat_id == new_chat_id {
            ChatListForumPresentationReplacementPolicyAction::TransferToNewPresentation
        } else {
            ChatListForumPresentationReplacementPolicyAction::ReleaseDisjointPayloads
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PayloadKind {
    ForumStructure,
    Topics,
    EffectivePermissions,
}

impl PayloadKind {
    const ALL_CASES: [Self; 3] = [Self::ForumStructure, Self::Topics, Self::EffectivePermissions];

    fn cache_key(self, chat_id: &str) -> String {
        match self {
            Self::ForumStructure => ChatServicesStore::forum_structure_cache_key(chat_id),
            Self::Topics => ChatServicesStore::topics_cache_key(chat_id),
            Self::EffectivePermissions => ChatServicesStore::effective_perms_cache_key(chat_id, None),
        }
    }
}

#[derive(PartialEq, Eq)]
struct CandidateSignature {
    chat_ids: Vec<String>,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

#[derive(Clone)]
struct Request {
    chat_id: String,
    payload_kind: PayloadKind,
    cache_key: String,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    generation: u64,
    lifecycle_generation: u64,
}

struct DeferredPayload {
    chat_id: String,
    payload_kind: PayloadKind,
    cache_key: String,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    presentation_generation: u64,
    lifecycle_generation: u64,
    value: Value,
}

struct Presentation {
    chat_id: String,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
    generation: u64,
    lifecycle_generation: u64,
    is_settled: bool,
    deferred: HashMap<PayloadKind, DeferredPayload>,
}

/// single lane speculative disk queue for nearby forum structures
/// never does network work and never reads disk on the main thread
struct ChatListForumDiskWarmupQueue {
    candidate_generation: u64,
    presentation_generation: u64,
    lifecycle_generation: u64,
    signature: Option<CandidateSignature>,
    desired_cache_keys: HashSet<String>,
    pending: Vec<Request>,
    in_flight: Option<Request>,
    presentation: Option<Presentation>,
    _environment_observers: Vec<RiseSubscription>,
    _energy_saving_observer: Subscription,
    _environment_signals: Task<()>,
}

impl ChatListForumDiskWarmupQueue {
    fn new(cx: &mut Context<Self>) -> Self {
        let _ = RiseMediaForegroundGate::shared(cx);
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        let center = RiseAppEvents::default_center();
        let environment_observers = [
            RISE_MEDIA_FOREGROUND_DID_CHANGE,
            RiseNotificationName::new(POWER_STATE_DID_CHANGE),
            RiseNotificationName::new(THERMAL_STATE_DID_CHANGE),
        ]
        .into_iter()
        .map(|name| {
            let sender = sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(());
            })
        })
        .collect();
        let energy_store = RiseEnergySavingStore::shared(cx);
        let energy_saving_observer = cx.subscribe(
            &energy_store,
            |this: &mut Self, _, _: &RiseEnergySavingDidChange, cx: &mut Context<Self>| {
                this.pump(cx);
            },
        );
        let environment_signals = cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                let updated = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| this.pump(cx));
                if updated.is_err() {
                    return;
                }
            }
        });
        Self {
            candidate_generation: 0,
            presentation_generation: 0,
            lifecycle_generation: 0,
            signature: None,
            desired_cache_keys: HashSet::new(),
            pending: Vec::new(),
            in_flight: None,
            presentation: None,
            _environment_observers: environment_observers,
            _energy_saving_observer: energy_saving_observer,
            _environment_signals: environment_signals,
        }
    }

    fn replace_candidates(&mut self, raw_chat_ids: &[String], cx: &mut Context<Self>) {
        let chat_ids = Self::normalized_chat_ids(raw_chat_ids);
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let next_signature = CandidateSignature {
            chat_ids: chat_ids.clone(),
            account_user_id: account_user_id.clone(),
            account_epoch: account_epoch.clone(),
        };

        if self.signature.as_ref() != Some(&next_signature) {
            self.candidate_generation = self.candidate_generation.wrapping_add(1);
            self.signature = Some(next_signature);
            self.pending.clear();
        }
        let requests = self.make_requests(
            &chat_ids,
            account_user_id.as_deref(),
            account_epoch.as_deref(),
            self.candidate_generation,
        );
        self.desired_cache_keys = requests.iter().map(|request| request.cache_key.clone()).collect();
        for request in requests {
            self.enqueue_candidate_if_needed(request, cx);
        }
        self.enqueue_presentation_requests_at_front(cx);
        self.pump(cx);
    }

    fn will_present_forum(&mut self, raw_chat_id: &str, cx: &mut Context<Self>) {
        let chat_id = Self::normalized_chat_id(raw_chat_id);
        if chat_id.is_empty() {
            return;
        }
        let chat_id = chat_id.to_owned();

        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let previous = self.presentation.take();
        let replaced_presentation_generation = self.presentation_generation;
        self.presentation_generation = self.presentation_generation.wrapping_add(1);
        let mut next = Presentation {
            chat_id: chat_id.clone(),
            account_user_id: account_user_id.clone(),
            account_epoch: account_epoch.clone(),
            generation: self.presentation_generation,
            lifecycle_generation: self.lifecycle_generation,
            is_settled: false,
            deferred: HashMap::new(),
        };

        let mut displaced_payloads: Vec<DeferredPayload> = Vec::new();
        let previous = previous.filter(|previous| previous.generation == replaced_presentation_generation);
        if let Some(mut previous) = previous {
            let action = ChatListForumPresentationReplacementPolicy::action(
                &previous.chat_id,
                &chat_id,
                previous.account_user_id.as_deref(),
                previous.account_epoch.as_deref(),
                account_user_id.as_deref(),
                account_epoch.as_deref(),
                previous.lifecycle_generation,
                self.lifecycle_generation,
            );
            match action {
                ChatListForumPresentationReplacementPolicyAction::TransferToNewPresentation => {
                    for payload_kind in PayloadKind::ALL_CASES {
                        let Some(payload) = previous.deferred.remove(&payload_kind) else {
                            continue;
                        };
                        if !self.deferred_payload_is_current(
                            &payload,
                            &previous,
                            account_user_id.as_deref(),
                            account_epoch.as_deref(),
                            cx,
                        ) {
                            continue;
                        }
                        next.deferred.insert(
                            payload_kind,
                            DeferredPayload {
                                chat_id: chat_id.clone(),
                                payload_kind,
                                cache_key: payload.cache_key,
                                account_user_id: account_user_id.clone(),
                                account_epoch: account_epoch.clone(),
                                presentation_generation: self.presentation_generation,
                                lifecycle_generation: self.lifecycle_generation,
                                value: payload.value,
                            },
                        );
                    }
                }
                ChatListForumPresentationReplacementPolicyAction::ReleaseDisjointPayloads => {
                    let opening_cache_keys: HashSet<String> =
                        PayloadKind::ALL_CASES.iter().map(|kind| kind.cache_key(&chat_id)).collect();
                    for payload_kind in PayloadKind::ALL_CASES {
                        let Some(payload) = previous.deferred.remove(&payload_kind) else {
                            continue;
                        };
                        if opening_cache_keys.contains(&payload.cache_key)
                            || !self.deferred_payload_is_current(
                                &payload,
                                &previous,
                                account_user_id.as_deref(),
                                account_epoch.as_deref(),
                                cx,
                            )
                        {
                            continue;
                        }
                        displaced_payloads.push(payload);
                    }
                }
                ChatListForumPresentationReplacementPolicyAction::Discard => {}
            }
        }

        let next_generation = next.generation;
        self.presentation = Some(next);
        for payload in &displaced_payloads {
            self.hydrate_displaced_payload_if_safe(payload, next_generation, cx);
        }
        self.enqueue_presentation_requests_at_front(cx);
        self.pump(cx);
    }

    fn did_settle_forum_presentation(&mut self, raw_chat_id: &str, cx: &mut Context<Self>) {
        let chat_id = Self::normalized_chat_id(raw_chat_id);
        let presentation_generation = self.presentation_generation;
        let lifecycle_generation = self.lifecycle_generation;
        let active = match self.presentation.as_mut() {
            Some(active)
                if active.chat_id == chat_id
                    && active.generation == presentation_generation
                    && active.lifecycle_generation == lifecycle_generation =>
            {
                active
            }
            _ => return,
        };
        if !SaiAccountCacheScope::is_current_account_epoch(
            active.account_user_id.as_deref(),
            active.account_epoch.as_deref(),
        ) {
            self.presentation_generation = self.presentation_generation.wrapping_add(1);
            self.presentation = None;
            return;
        }

        active.is_settled = true;
        let deferred: Vec<DeferredPayload> =
            PayloadKind::ALL_CASES.iter().filter_map(|kind| active.deferred.remove(kind)).collect();
        active.deferred.clear();
        for payload in &deferred {
            self.release_deferred_payload(payload, cx);
        }
        self.enqueue_presentation_requests_at_front(cx);
        self.pump(cx);
    }

    fn cancel_all(&mut self) {
        self.candidate_generation = self.candidate_generation.wrapping_add(1);
        self.presentation_generation = self.presentation_generation.wrapping_add(1);
        self.lifecycle_generation = self.lifecycle_generation.wrapping_add(1);
        self.signature = None;
        self.desired_cache_keys.clear();
        self.pending.clear();
        self.presentation = None;
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        if self.in_flight.is_some() || !RiseMediaForegroundGate::shared(cx).read(cx).is_foreground() {
            return;
        }
        while !self.pending.is_empty() {
            let allows_speculation = CleverImagePrefetchPolicy::allows_prefetch_now(cx);
            let Some(index) = self
                .pending
                .iter()
                .position(|request| allows_speculation || self.presentation_matches(request))
            else {
                return;
            };
            let request = self.pending.remove(index);
            let request_lifecycle_is_current = request.lifecycle_generation == self.lifecycle_generation;
            let candidate_is_current = request_lifecycle_is_current
                && request.generation == self.candidate_generation
                && self.desired_cache_keys.contains(&request.cache_key);
            let presentation_is_current = self.presentation_matches(&request);
            let is_admissible = (candidate_is_current || presentation_is_current)
                && SaiAccountCacheScope::is_current_account_epoch(
                    request.account_user_id.as_deref(),
                    request.account_epoch.as_deref(),
                )
                && request.payload_kind.cache_key(&request.chat_id) == request.cache_key
                && !self.has_deferred_payload(&request)
                && SaiWsResponseCache::memory_only(&request.cache_key, cx).is_none();
            if !is_admissible {
                continue;
            }

            self.in_flight = Some(request.clone());
            let this = cx.weak_entity();
            SaiWsDiskCache::load_asynchronously(
                request.cache_key.clone(),
                move |payload, cx| {
                    let _ = this.update(cx, |this, cx| this.finish(request, payload, cx));
                },
                cx,
            );
            return;
        }
    }

    fn finish(&mut self, request: Request, payload: Option<Value>, cx: &mut Context<Self>) {
        let is_active = self
            .in_flight
            .as_ref()
            .is_some_and(|active| active.cache_key == request.cache_key && active.generation == request.generation);
        if !is_active {
            return;
        }
        self.in_flight = None;
        self.apply_finished_load(request, payload, cx);
        self.pump(cx);
    }

    fn apply_finished_load(&mut self, request: Request, payload: Option<Value>, cx: &mut Context<Self>) {
        let Some(payload) = payload else {
            return;
        };
        if !SaiAccountCacheScope::is_current_account_epoch(
            request.account_user_id.as_deref(),
            request.account_epoch.as_deref(),
        ) || request.payload_kind.cache_key(&request.chat_id) != request.cache_key
            || SaiWsResponseCache::memory_only(&request.cache_key, cx).is_some()
        {
            return;
        }

        if self.presentation_matches(&request) {
            let Some(active) = self.presentation.as_mut() else {
                return;
            };
            if active.is_settled {
                SaiWsResponseCache::hydrate_memory(&request.cache_key, &payload, true, cx);
            } else {
                let deferred = DeferredPayload {
                    chat_id: request.chat_id,
                    payload_kind: request.payload_kind,
                    cache_key: request.cache_key,
                    account_user_id: request.account_user_id,
                    account_epoch: request.account_epoch,
                    presentation_generation: active.generation,
                    lifecycle_generation: active.lifecycle_generation,
                    value: payload,
                };
                active.deferred.insert(deferred.payload_kind, deferred);
            }
            return;
        }

        if request.lifecycle_generation != self.lifecycle_generation
            || request.generation != self.candidate_generation
            || !self.desired_cache_keys.contains(&request.cache_key)
        {
            return;
        }
        if !CleverImagePrefetchPolicy::allows_prefetch_now(cx) {
            self.enqueue_candidate_if_needed(request, cx);
            return;
        }
        SaiWsResponseCache::hydrate_memory(&request.cache_key, &payload, true, cx);
    }

    fn release_deferred_payload(&self, payload: &DeferredPayload, cx: &mut App) {
        let Some(active) = self.presentation.as_ref() else {
            return;
        };
        if !active.is_settled
            || active.chat_id != payload.chat_id
            || active.generation != payload.presentation_generation
            || active.generation != self.presentation_generation
            || active.lifecycle_generation != self.lifecycle_generation
            || payload.lifecycle_generation != active.lifecycle_generation
            || active.account_user_id != payload.account_user_id
            || active.account_epoch != payload.account_epoch
            || !SaiAccountCacheScope::is_current_account_epoch(
                payload.account_user_id.as_deref(),
                payload.account_epoch.as_deref(),
            )
            || payload.payload_kind.cache_key(&payload.chat_id) != payload.cache_key
            || SaiWsResponseCache::memory_only(&payload.cache_key, cx).is_some()
        {
            return;
        }

        SaiWsResponseCache::hydrate_memory(&payload.cache_key, &payload.value, true, cx);
    }

    fn deferred_payload_is_current(
        &self,
        payload: &DeferredPayload,
        owner: &Presentation,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        cx: &mut App,
    ) -> bool {
        owner.chat_id == payload.chat_id
            && owner.generation == payload.presentation_generation
            && owner.lifecycle_generation == self.lifecycle_generation
            && payload.lifecycle_generation == owner.lifecycle_generation
            && owner.account_user_id == payload.account_user_id
            && owner.account_epoch == payload.account_epoch
            && payload.account_user_id.as_deref() == account_user_id
            && payload.account_epoch.as_deref() == account_epoch
            && SaiAccountCacheScope::is_current_account_epoch(
                payload.account_user_id.as_deref(),
                payload.account_epoch.as_deref(),
            )
            && payload.payload_kind.cache_key(&payload.chat_id) == payload.cache_key
            && SaiWsResponseCache::memory_only(&payload.cache_key, cx).is_none()
    }

    fn hydrate_displaced_payload_if_safe(
        &self,
        payload: &DeferredPayload,
        while_presenting_generation: u64,
        cx: &mut App,
    ) {
        let Some(active) = self.presentation.as_ref() else {
            return;
        };
        if active.generation != while_presenting_generation
            || active.generation != self.presentation_generation
            || active.lifecycle_generation != self.lifecycle_generation
            || active.account_user_id != payload.account_user_id
            || active.account_epoch != payload.account_epoch
            || PayloadKind::ALL_CASES.iter().any(|kind| kind.cache_key(&active.chat_id) == payload.cache_key)
            || payload.lifecycle_generation != self.lifecycle_generation
            || !SaiAccountCacheScope::is_current_account_epoch(
                payload.account_user_id.as_deref(),
                payload.account_epoch.as_deref(),
            )
            || payload.payload_kind.cache_key(&payload.chat_id) != payload.cache_key
            || SaiWsResponseCache::memory_only(&payload.cache_key, cx).is_some()
        {
            return;
        }

        SaiWsResponseCache::hydrate_memory(&payload.cache_key, &payload.value, true, cx);
    }

    fn enqueue_candidate_if_needed(&mut self, request: Request, cx: &mut App) {
        if SaiWsResponseCache::memory_only(&request.cache_key, cx).is_some()
            || self.has_deferred_payload(&request)
            || self
                .pending
                .iter()
                .any(|pending| pending.cache_key == request.cache_key && pending.generation == request.generation)
        {
            return;
        }
        let in_flight = self.in_flight.as_ref().filter(|in_flight| in_flight.cache_key == request.cache_key);
        if let Some(in_flight) = in_flight {
            let current_candidate_load = in_flight.lifecycle_generation == self.lifecycle_generation
                && in_flight.generation == request.generation;
            if current_candidate_load || self.presentation_matches(in_flight) {
                return;
            }
        }
        self.pending.push(request);
    }

    fn enqueue_presentation_requests_at_front(&mut self, cx: &mut App) {
        let Some(active) = self.presentation.as_ref() else {
            return;
        };
        if active.generation != self.presentation_generation
            || !SaiAccountCacheScope::is_current_account_epoch(
                active.account_user_id.as_deref(),
                active.account_epoch.as_deref(),
            )
        {
            return;
        }

        let requests = self.make_requests(
            std::slice::from_ref(&active.chat_id),
            active.account_user_id.as_deref(),
            active.account_epoch.as_deref(),
            self.candidate_generation,
        );
        let mut priority_requests: Vec<Request> = Vec::with_capacity(requests.len());
        for request in requests {
            let is_loading = self.in_flight.as_ref().is_some_and(|in_flight| {
                in_flight.cache_key == request.cache_key && in_flight.lifecycle_generation == self.lifecycle_generation
            });
            if SaiWsResponseCache::memory_only(&request.cache_key, cx).is_some()
                || self.has_deferred_payload(&request)
                || is_loading
            {
                continue;
            }
            self.pending.retain(|pending| pending.cache_key != request.cache_key);
            priority_requests.push(request);
        }
        if !priority_requests.is_empty() {
            priority_requests.append(&mut self.pending);
            self.pending = priority_requests;
        }
    }

    fn presentation_matches(&self, request: &Request) -> bool {
        self.presentation.as_ref().is_some_and(|active| self.presentation_matches_active(request, active))
    }

    fn presentation_matches_active(&self, request: &Request, active: &Presentation) -> bool {
        active.generation == self.presentation_generation
            && active.lifecycle_generation == self.lifecycle_generation
            && request.lifecycle_generation == self.lifecycle_generation
            && active.chat_id == request.chat_id
            && active.account_user_id == request.account_user_id
            && active.account_epoch == request.account_epoch
            && request.payload_kind.cache_key(&request.chat_id) == request.cache_key
    }

    fn has_deferred_payload(&self, request: &Request) -> bool {
        self.presentation.as_ref().is_some_and(|active| {
            self.presentation_matches_active(request, active) && active.deferred.contains_key(&request.payload_kind)
        })
    }

    fn make_requests(
        &self,
        chat_ids: &[String],
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
        generation: u64,
    ) -> Vec<Request> {
        let lifecycle_generation = self.lifecycle_generation;
        chat_ids
            .iter()
            .flat_map(|chat_id| {
                PayloadKind::ALL_CASES.into_iter().map(move |payload_kind| Request {
                    chat_id: chat_id.clone(),
                    payload_kind,
                    cache_key: payload_kind.cache_key(chat_id),
                    account_user_id: account_user_id.map(str::to_owned),
                    account_epoch: account_epoch.map(str::to_owned),
                    generation,
                    lifecycle_generation,
                })
            })
            .collect()
    }

    fn normalized_chat_ids(raw_chat_ids: &[String]) -> Vec<String> {
        let maximum_forum_candidates = ChatListHistoryWarmupPolicy::maximum_forum_candidates();
        let mut result: Vec<String> = Vec::with_capacity(usize::try_from(maximum_forum_candidates).unwrap_or(0));
        let mut seen: HashSet<&str> = HashSet::new();
        for raw_chat_id in raw_chat_ids {
            let chat_id = Self::normalized_chat_id(raw_chat_id);
            if chat_id.is_empty() || !seen.insert(chat_id) {
                continue;
            }
            result.push(chat_id.to_owned());
            if result.len() as i64 == maximum_forum_candidates {
                break;
            }
        }
        result
    }

    fn normalized_chat_id(raw_chat_id: &str) -> &str {
        trimming_whitespaces_and_newlines(raw_chat_id)
    }
}

struct PageState {
    center: Option<i64>,
    center_chat_id: Option<String>,
    input_signature: Option<InputSignature>,
    visible_chat_ids: Vec<String>,
    recent_chat_ids: Vec<String>,
    forum_chat_ids: Vec<String>,
}

/// feeds a stable direction aware batch to the bounded disk history warmer
/// separate page state so retained folder tabs cant steal priority from the visible page
pub struct ChatListHistoryWarmupController {
    pages: HashMap<String, PageState>,
    active_page_id: Option<String>,
    forum_disk_warmup: Option<Entity<ChatListForumDiskWarmupQueue>>,
}

impl Default for ChatListHistoryWarmupController {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatListHistoryWarmupController {
    pub fn new() -> Self {
        Self {
            pages: HashMap::new(),
            active_page_id: None,
            forum_disk_warmup: None,
        }
    }

    fn forum_disk_warmup(&mut self, cx: &mut App) -> Entity<ChatListForumDiskWarmupQueue> {
        self.forum_disk_warmup.get_or_insert_with(|| cx.new(ChatListForumDiskWarmupQueue::new)).clone()
    }

    pub fn activate(&mut self, page_id: &str, cx: &mut App) {
        if self.active_page_id.as_deref() == Some(page_id) {
            return;
        }
        self.active_page_id = Some(page_id.to_owned());
        self.publish(page_id, cx);
    }

    pub fn update(&mut self, page_id: &str, visible_index: i64, chats: &[ChatInfo], cx: &mut App) {
        let Some(center_chat) = usize::try_from(visible_index).ok().and_then(|index| chats.get(index)) else {
            return;
        };
        let center_chat_id = center_chat.id.as_str();
        let item_count = chats.len() as i64;
        let input_signature = ChatListHistoryWarmupPolicy::bounded_input_signature(
            item_count,
            visible_index,
            |index| chats[index as usize].id.clone(),
            |index| chats[index as usize].topics_enabled,
        );
        let state = self.pages.get(page_id);
        let is_unchanged = state.is_some_and(|state| {
            state.center.is_some_and(|center| (center - visible_index).abs() < 2)
                && state.center_chat_id.as_deref() == Some(center_chat_id)
                && state.input_signature.as_ref() == Some(&input_signature)
        });
        if is_unchanged {
            return;
        }

        let previous_center = state.and_then(|state| state.center);
        let indices = ChatListHistoryWarmupPolicy::candidate_indices(item_count, visible_index, previous_center, None);
        let maximum_visible_candidates = ChatListHistoryWarmupPolicy::maximum_visible_candidates();
        let visible_chat_ids =
            Self::eligible_chat_ids(indices.iter().map(|&index| &chats[index as usize]), maximum_visible_candidates);
        let recent_scan_count = usize::try_from(ChatListHistoryWarmupPolicy::maximum_scan_distance()).unwrap_or(0);
        let recent_chat_ids = Self::eligible_chat_ids(chats.iter().take(recent_scan_count), maximum_visible_candidates);
        let forum_indices = ChatListHistoryWarmupPolicy::forum_candidate_indices(
            item_count,
            visible_index,
            previous_center,
            None,
            |index| chats[index as usize].topics_enabled,
        );
        let forum_chat_ids = Self::eligible_forum_chat_ids(forum_indices.iter().map(|&index| &chats[index as usize]));
        self.pages.insert(
            page_id.to_owned(),
            PageState {
                center: Some(visible_index),
                center_chat_id: Some(center_chat_id.to_owned()),
                input_signature: Some(input_signature),
                visible_chat_ids,
                recent_chat_ids,
                forum_chat_ids,
            },
        );

        if self.active_page_id.as_deref() != Some(page_id) {
            return;
        }
        self.publish(page_id, cx);
    }

    pub fn will_present_forum(&mut self, chat_id: &str, cx: &mut App) {
        let forum_disk_warmup = self.forum_disk_warmup(cx);
        forum_disk_warmup.update(cx, |queue, cx| queue.will_present_forum(chat_id, cx));
    }

    /// releases at most the structure and permissions pair after the opening animation fully settled
    pub fn did_settle_forum_presentation(&mut self, chat_id: &str, cx: &mut App) {
        let forum_disk_warmup = self.forum_disk_warmup(cx);
        forum_disk_warmup.update(cx, |queue, cx| queue.did_settle_forum_presentation(chat_id, cx));
    }

    pub fn cancel_all(&mut self, cx: &mut App) {
        self.active_page_id = None;
        self.pages.clear();
        let forum_disk_warmup = self.forum_disk_warmup(cx);
        forum_disk_warmup.update(cx, |queue, _| queue.cancel_all());
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.cancel_message_history_warmup(cx));
    }

    fn publish(&mut self, page_id: &str, cx: &mut App) {
        let forum_disk_warmup = self.forum_disk_warmup(cx);
        let Some(state) = self.pages.get(page_id) else {
            forum_disk_warmup.update(cx, |queue, cx| queue.replace_candidates(&[], cx));
            return;
        };
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.prioritize_message_history_warmup(&state.visible_chat_ids, &state.recent_chat_ids, cx);
        });
        forum_disk_warmup.update(cx, |queue, cx| queue.replace_candidates(&state.forum_chat_ids, cx));
    }

    fn eligible_chat_ids<'a>(chats: impl IntoIterator<Item = &'a ChatInfo>, limit: i64) -> Vec<String> {
        let mut ids: Vec<String> = Vec::with_capacity(usize::try_from(limit).unwrap_or(0));
        let mut seen: HashSet<&str> = HashSet::new();
        for chat in chats {
            let id = trimming_whitespaces_and_newlines(&chat.id);
            if id.is_empty()
                || chat.topics_enabled
                || UserServicesStore::is_contact_join_virtual_chat_id(id)
                || !seen.insert(id)
            {
                continue;
            }
            ids.push(id.to_owned());
            if ids.len() as i64 == limit {
                break;
            }
        }
        ids
    }

    fn eligible_forum_chat_ids<'a>(chats: impl IntoIterator<Item = &'a ChatInfo>) -> Vec<String> {
        let maximum_forum_candidates = ChatListHistoryWarmupPolicy::maximum_forum_candidates();
        let mut ids: Vec<String> = Vec::with_capacity(usize::try_from(maximum_forum_candidates).unwrap_or(0));
        let mut seen: HashSet<&str> = HashSet::new();
        for chat in chats {
            let id = trimming_whitespaces_and_newlines(&chat.id);
            if id.is_empty()
                || !chat.topics_enabled
                || UserServicesStore::is_contact_join_virtual_chat_id(id)
                || !seen.insert(id)
            {
                continue;
            }
            ids.push(id.to_owned());
            if ids.len() as i64 == maximum_forum_candidates {
                break;
            }
        }
        ids
    }
}

#[cfg(test)]
#[path = "chat_list_history_warmup_controller_tests.rs"]
mod tests;
