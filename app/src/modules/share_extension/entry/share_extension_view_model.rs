use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext as _, AsyncApp, Context, Task, WeakEntity};
use rise_canon::RiseTimingCanon;
use rise_core::foundation::foundation_compare::contains_case_diacritic_insensitive;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_platform::haptics::{NotificationFeedbackGenerator, NotificationKind};

use crate::modules::share_extension::entry::share_extension_configuration::ShareExtensionConfiguration;
use crate::modules::share_extension::entry::share_extension_dependencies::{
    ShareExtensionDependencies, ShareSendCommitStarted, ShareSendProgressHandler,
};
use crate::modules::share_extension::entry::share_extension_models::{
    ShareAccountSnapshot, ShareDestination, ShareDestinationKind, ShareExtensionError, SharePartialDeliveryError,
    SharePayload, ShareSelection, ShareSendProgress, ShareSendRequest, ShareTopic,
};
use crate::modules::share_extension::entry::share_extension_strings::{ShareErrorPresenter, ShareStrings};
use crate::modules::share_extension::entry::share_item_loader::{ShareExtensionItem, ShareItemLoader};
use crate::modules::share_extension::entry::share_temporary_file_store::ShareTemporaryFileStore;
use crate::modules::share_extension::shared::rise_share_models::{RiseShareTopicGroup, RiseShareTopicGrouping};

const SEARCH_DEBOUNCE_FALLBACK_MS: u64 = 280;
const SEARCH_DEBOUNCE_NAME: &str = "share_targets_search";
const MINIMUM_VISIBLE_SEND_PROGRESS: f64 = 0.04;
const ACKNOWLEDGED_PAUSE: Duration = Duration::from_millis(180);
const SUCCEEDED_PAUSE: Duration = Duration::from_millis(800);
const PAGE_PREFETCH_DISTANCE: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareExtensionViewModelActivity {
    Loading,
    Ready,
    Sending,
    OpeningStory,
    OpeningApp,
    Succeeded,
}

/// view model of the share layer, every mutation notifies
pub struct ShareExtensionViewModel {
    activity: ShareExtensionViewModelActivity,
    payload: Option<SharePayload>,
    account: Option<ShareAccountSnapshot>,
    destinations: Vec<ShareDestination>,
    displayed_destinations: Vec<ShareDestination>,
    selections_by_destination_id: HashMap<String, ShareSelection>,
    topic_destination: Option<ShareDestination>,
    topic_groups: Vec<RiseShareTopicGroup<ShareTopic>>,
    is_loading_topics: bool,
    is_searching: bool,
    is_live_validated: bool,
    is_retry_locked: bool,
    has_partial_delivery: bool,
    did_receive_backend_acknowledgement: bool,
    is_commit_started: bool,
    send_progress: f64,
    error_message: Option<String>,
    error_title: Option<String>,
    nonfatal_message: Option<String>,
    pub search_text: String,
    pub comment: String,

    pub configuration: ShareExtensionConfiguration,

    dependencies: Rc<ShareExtensionDependencies>,
    item_loader: Rc<ShareItemLoader>,
    temporary_file_store: Arc<ShareTemporaryFileStore>,
    input_items: Vec<ShareExtensionItem>,
    suggested_destination_id: Option<String>,
    on_finish: Rc<dyn Fn(&mut App)>,
    on_cancel: Rc<dyn Fn(&mut App)>,
    idempotency_key: Uuid,
    retained_retry_request: Option<ShareSendRequest>,
    bootstrap_task: Option<Task<()>>,
    search_task: Option<Task<()>>,
    more_destinations_task: Option<Task<()>>,
    recent_next_offset: Option<i64>,
    search_next_offset: Option<i64>,
    topics_task: Option<Task<()>>,
    send_task: Option<Task<()>>,
    did_terminate: bool,
}

impl ShareExtensionViewModel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        configuration: ShareExtensionConfiguration,
        dependencies: ShareExtensionDependencies,
        item_loader: ShareItemLoader,
        temporary_file_store: Arc<ShareTemporaryFileStore>,
        input_items: Vec<ShareExtensionItem>,
        suggested_destination_id: Option<String>,
        on_finish: Rc<dyn Fn(&mut App)>,
        on_cancel: Rc<dyn Fn(&mut App)>,
    ) -> Self {
        Self {
            activity: ShareExtensionViewModelActivity::Loading,
            payload: None,
            account: None,
            destinations: Vec::new(),
            displayed_destinations: Vec::new(),
            selections_by_destination_id: HashMap::new(),
            topic_destination: None,
            topic_groups: Vec::new(),
            is_loading_topics: false,
            is_searching: false,
            is_live_validated: false,
            is_retry_locked: false,
            has_partial_delivery: false,
            did_receive_backend_acknowledgement: false,
            is_commit_started: false,
            send_progress: 0.0,
            error_message: None,
            error_title: None,
            nonfatal_message: None,
            search_text: String::new(),
            comment: String::new(),
            configuration,
            dependencies: Rc::new(dependencies),
            item_loader: Rc::new(item_loader),
            temporary_file_store,
            input_items,
            suggested_destination_id,
            on_finish,
            on_cancel,
            idempotency_key: Uuid::new_v4(),
            retained_retry_request: None,
            bootstrap_task: None,
            search_task: None,
            more_destinations_task: None,
            recent_next_offset: None,
            search_next_offset: None,
            topics_task: None,
            send_task: None,
            did_terminate: false,
        }
    }

    pub fn activity(&self) -> ShareExtensionViewModelActivity {
        self.activity
    }

    pub fn payload(&self) -> Option<&SharePayload> {
        self.payload.as_ref()
    }

    pub fn account(&self) -> Option<&ShareAccountSnapshot> {
        self.account.as_ref()
    }

    pub fn destinations(&self) -> &[ShareDestination] {
        &self.destinations
    }

    pub fn displayed_destinations(&self) -> &[ShareDestination] {
        &self.displayed_destinations
    }

    pub fn selections_by_destination_id(&self) -> &HashMap<String, ShareSelection> {
        &self.selections_by_destination_id
    }

    pub fn topic_destination(&self) -> Option<&ShareDestination> {
        self.topic_destination.as_ref()
    }

    pub fn topic_groups(&self) -> &[RiseShareTopicGroup<ShareTopic>] {
        &self.topic_groups
    }

    pub fn is_loading_topics(&self) -> bool {
        self.is_loading_topics
    }

    pub fn is_searching(&self) -> bool {
        self.is_searching
    }

    pub fn is_live_validated(&self) -> bool {
        self.is_live_validated
    }

    pub fn is_retry_locked(&self) -> bool {
        self.is_retry_locked
    }

    pub fn has_partial_delivery(&self) -> bool {
        self.has_partial_delivery
    }

    pub fn did_receive_backend_acknowledgement(&self) -> bool {
        self.did_receive_backend_acknowledgement
    }

    pub fn is_commit_started(&self) -> bool {
        self.is_commit_started
    }

    pub fn send_progress(&self) -> f64 {
        self.send_progress
    }

    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    pub fn error_title(&self) -> Option<&str> {
        self.error_title.as_deref()
    }

    pub fn nonfatal_message(&self) -> Option<&str> {
        self.nonfatal_message.as_deref()
    }

    /// a new error message drops the title of the previous one
    fn set_error_message(&mut self, message: Option<String>) {
        self.error_message = message;
        self.error_title = None;
    }

    pub fn selected_count(&self) -> i64 {
        self.selections_by_destination_id.len() as i64
    }

    pub fn can_send(&self) -> bool {
        self.selected_count() > 0
            && self.activity == ShareExtensionViewModelActivity::Ready
            && self.is_live_validated
    }

    pub fn can_share_to_story(&self) -> bool {
        self.payload.as_ref().is_some_and(SharePayload::can_share_to_story)
            && self.activity == ShareExtensionViewModelActivity::Ready
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            self.activity,
            ShareExtensionViewModelActivity::Sending
                | ShareExtensionViewModelActivity::OpeningStory
                | ShareExtensionViewModelActivity::OpeningApp
                | ShareExtensionViewModelActivity::Succeeded
        )
    }

    pub fn can_cancel(&self) -> bool {
        !self.is_commit_started && !self.did_receive_backend_acknowledgement && !self.has_partial_delivery
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.bootstrap_task.is_some() {
            return;
        }
        self.bootstrap_task = Some(cx.spawn(async move |this, cx| {
            Self::bootstrap(this, cx).await;
        }));
    }

    pub fn set_search_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.is_retry_locked {
            return;
        }
        self.is_searching = active;
        if !active {
            self.search_task = None;
            self.cancel_more_destinations();
            self.search_text = String::new();
            self.displayed_destinations = self.destinations.clone();
        }
        cx.notify();
    }

    pub fn set_search_text(&mut self, value: String, cx: &mut Context<Self>) {
        if self.search_text == value {
            return;
        }
        self.search_text = value;
        cx.notify();
    }

    fn trimmed_search_text(&self) -> String {
        trimming_whitespaces_and_newlines(&self.search_text).to_owned()
    }

    pub fn search_did_change(&mut self, cx: &mut Context<Self>) {
        self.search_task = None;
        self.cancel_more_destinations();
        self.search_next_offset = None;
        let query = self.trimmed_search_text();
        let payload = match &self.payload {
            Some(payload) if !query.is_empty() => payload.clone(),
            _ => {
                self.displayed_destinations = self.destinations.clone();
                cx.notify();
                return;
            }
        };
        cx.notify();
        let remote = self.dependencies.remote.clone();
        let snapshot = self.dependencies.snapshot.clone();
        let limit = self.configuration.search_result_limit;
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let debounce_ms = RiseTimingCanon::DEBOUNCES
                .iter()
                .find(|spec| spec.name == SEARCH_DEBOUNCE_NAME)
                .and_then(|spec| u64::try_from(spec.client_debounce_ms).ok())
                .unwrap_or(SEARCH_DEBOUNCE_FALLBACK_MS);
            cx.background_executor().timer(Duration::from_millis(debounce_ms)).await;
            let mut found: Vec<ShareDestination> = Vec::new();
            let mut offset = 0;
            let outcome: anyhow::Result<Option<Option<i64>>> = async {
                loop {
                    let page = remote
                        .destinations(Some(query.clone()), payload.clone(), limit, offset, cx.clone())
                        .await?;
                    let still_current = this
                        .read_with(cx, |model, _| model.is_searching && model.trimmed_search_text() == query)
                        .unwrap_or(false);
                    if !still_current {
                        return Ok(None);
                    }
                    found = Self::appending_new(page.destinations, &found);
                    let next = page.next_offset;
                    offset = next.unwrap_or(offset);
                    if !(found.is_empty() && next.is_some()) {
                        return Ok(Some(next));
                    }
                }
            }
            .await;
            match outcome {
                Ok(None) => {}
                Ok(Some(next)) => {
                    let _ = this.update(cx, |model, cx| {
                        model.search_next_offset = next;
                        let presented = Self::preserving_local_presentation(found, &model.destinations);
                        model.displayed_destinations = model.including_saved_match(presented, &query);
                        cx.notify();
                    });
                }
                Err(error) => {
                    let local = snapshot.search_destinations(query.clone(), limit).await.unwrap_or_default();
                    let _ = this.update(cx, |model, cx| {
                        if !(model.is_searching && model.trimmed_search_text() == query) {
                            return;
                        }
                        model.displayed_destinations = local;
                        model.is_live_validated = false;
                        model.nonfatal_message = Some(ShareErrorPresenter::message(&error));
                        cx.notify();
                    });
                }
            }
        }));
    }

    pub fn tap_destination(&mut self, destination: ShareDestination, cx: &mut Context<Self>) {
        if self.activity != ShareExtensionViewModelActivity::Ready
            || !self.is_live_validated
            || self.is_retry_locked
            || !destination.is_writable
        {
            return;
        }
        self.set_error_message(None);
        if destination.has_topics {
            self.open_topics(destination, cx);
            cx.notify();
            return;
        }
        if self.selections_by_destination_id.contains_key(&destination.id) {
            self.selections_by_destination_id.remove(&destination.id);
        } else if self.can_add_destination(&destination.id) {
            self.selections_by_destination_id.insert(
                destination.id.clone(),
                ShareSelection {
                    destination,
                    topic: None,
                },
            );
        }
        cx.notify();
    }

    pub fn select_topic(&mut self, topic: ShareTopic, cx: &mut Context<Self>) {
        let Some(destination) = self.topic_destination.clone() else {
            return;
        };
        if !self.is_live_validated || self.is_retry_locked || topic.destination_id != destination.id || !topic.is_writable
        {
            return;
        }
        let current_topic_selection_id = self
            .selections_by_destination_id
            .get(&destination.id)
            .map(|selection| selection.topic.as_ref().map(ShareTopic::selection_id));
        if current_topic_selection_id
            .as_ref()
            .and_then(Option::as_ref)
            .is_some_and(|selection_id| *selection_id == topic.selection_id())
        {
            self.selections_by_destination_id.remove(&destination.id);
            cx.notify();
            return;
        }
        if current_topic_selection_id.is_none() && !self.can_add_destination(&destination.id) {
            cx.notify();
            return;
        }
        self.selections_by_destination_id.insert(
            destination.id.clone(),
            ShareSelection {
                destination,
                topic: Some(topic),
            },
        );
        cx.notify();
    }

    pub fn close_topics(&mut self, cx: &mut Context<Self>) {
        self.topics_task = None;
        self.topic_destination = None;
        self.topic_groups = Vec::new();
        self.is_loading_topics = false;
        cx.notify();
    }

    pub fn set_comment(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.is_retry_locked {
            return;
        }
        let maximum = usize::try_from(self.configuration.maximum_comment_length).unwrap_or(0);
        let limited: String = value.chars().take(maximum).collect();
        if limited == self.comment {
            return;
        }
        self.comment = limited;
        cx.notify();
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.set_error_message(None);
        cx.notify();
        if self.payload.is_none() && self.activity == ShareExtensionViewModelActivity::Ready {
            self.cancel(cx);
        }
    }

    pub fn retry_loading(&mut self, cx: &mut Context<Self>) {
        if self.is_busy() {
            return;
        }
        self.set_error_message(None);
        self.bootstrap_task = None;
        self.activity = ShareExtensionViewModelActivity::Loading;
        cx.notify();
        self.start(cx);
    }

    pub fn send(&mut self, cx: &mut Context<Self>) {
        if !self.can_send() {
            return;
        }
        let Some(payload) = self.payload.clone() else {
            return;
        };
        let trimmed_comment = trimming_whitespaces_and_newlines(&self.comment).to_owned();
        if payload.file_attachments().is_empty() {
            let textual_content = payload.textual_content();
            let assembled = if trimmed_comment.is_empty() {
                textual_content
            } else if trimming_whitespaces_and_newlines(&textual_content).is_empty() {
                trimmed_comment
            } else {
                format!("{trimmed_comment}\n{textual_content}")
            };
            if assembled.chars().count() as i64 > self.configuration.maximum_content_length {
                self.set_error_message(Some(
                    ShareExtensionError::ContentTooLong(self.configuration.maximum_content_length)
                        .localized_description(),
                ));
                cx.notify();
                return;
            }
        }
        self.send_task = None;
        self.activity = ShareExtensionViewModelActivity::Sending;
        self.send_progress = self.send_progress.max(MINIMUM_VISIBLE_SEND_PROGRESS);
        self.set_error_message(None);
        let request = match &self.retained_retry_request {
            Some(retained) => retained.clone(),
            None => {
                let mut selections: Vec<ShareSelection> =
                    self.selections_by_destination_id.values().cloned().collect();
                selections.sort_by_key(ShareSelection::id);
                ShareSendRequest::new(self.idempotency_key, payload, selections, self.comment.clone())
            }
        };
        self.retained_retry_request = Some(request.clone());
        self.is_retry_locked = true;
        cx.notify();

        let weak_model = cx.weak_entity();
        let on_commit_started: ShareSendCommitStarted = Rc::new(move |cx: &mut App| {
            let _ = weak_model.update(cx, |model, cx| model.mark_commit_started(cx));
        });
        let weak_model = cx.weak_entity();
        let progress: ShareSendProgressHandler = Rc::new(move |progress: ShareSendProgress, cx: &mut App| {
            let _ = weak_model.update(cx, |model, cx| {
                if model.activity != ShareExtensionViewModelActivity::Sending {
                    return;
                }
                model.send_progress = model.send_progress.max(progress.fraction_completed);
                cx.notify();
            });
        });

        let dependencies = self.dependencies.clone();
        let foreground_byte_cap = self.configuration.foreground_upload_byte_cap;
        self.send_task = Some(cx.spawn(async move |this, cx| {
            let outcome: anyhow::Result<()> = async {
                if request.payload.requires_containing_app_upload(foreground_byte_cap) {
                    this.update(cx, |model, cx| {
                        model.activity = ShareExtensionViewModelActivity::OpeningApp;
                        cx.notify();
                    })?;
                    dependencies
                        .large_send_handoff
                        .handoff(request.clone(), cx.clone())
                        .await?;
                    this.update(cx, |model, cx| model.finish_without_cleanup(cx))?;
                    return Ok(());
                }
                dependencies
                    .sender
                    .send(request.clone(), on_commit_started, progress, cx.clone())
                    .await?;
                this.update(cx, |model, cx| {
                    model.did_receive_backend_acknowledgement = true;
                    model.send_progress = 1.0;
                    cx.notify();
                })?;
                cx.background_executor().timer(ACKNOWLEDGED_PAUSE).await;
                this.update(cx, |model, cx| {
                    model.activity = ShareExtensionViewModelActivity::Succeeded;
                    cx.notify();
                })?;
                NotificationFeedbackGenerator::new().notification_occurred(NotificationKind::Success);
                cx.background_executor().timer(SUCCEEDED_PAUSE).await;
                let finish = this.update(cx, |model, cx| model.finish_successfully(cx))?;
                finish.await;
                Ok(())
            }
            .await;
            let Err(error) = outcome else {
                return;
            };
            if Self::is_cancellation(&error) {
                return;
            }
            let _ = this.update(cx, |model, cx| {
                model.activity = ShareExtensionViewModelActivity::Ready;
                if let Some(partial) = error.downcast_ref::<SharePartialDeliveryError>() {
                    if partial.delivered_count > 0 {
                        model.did_receive_backend_acknowledgement = true;
                        model.has_partial_delivery = true;
                    } else {
                        model.unlock_failed_operation_without_delivery(cx);
                    }
                }
                model.set_error_message(Some(ShareErrorPresenter::message(&error)));
                cx.notify();
            });
        }));
    }

    pub fn share_to_story(&mut self, cx: &mut Context<Self>) {
        if !self.can_share_to_story() {
            return;
        }
        let Some(payload) = self.payload.clone() else {
            return;
        };
        self.activity = ShareExtensionViewModelActivity::OpeningStory;
        self.set_error_message(None);
        cx.notify();
        let story_handoff = self.dependencies.story_handoff.clone();
        self.send_task = Some(cx.spawn(async move |this, cx| {
            let outcome = story_handoff.handoff(payload, cx.clone()).await;
            if let Err(error) = &outcome
                && Self::is_cancellation(error)
            {
                return;
            }
            let _ = this.update(cx, |model, cx| match outcome {
                Ok(()) => model.finish_without_cleanup(cx),
                Err(error) => {
                    model.activity = ShareExtensionViewModelActivity::Ready;
                    model.set_error_message(Some(ShareErrorPresenter::message(&error)));
                    cx.notify();
                }
            });
        }));
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.did_terminate || !self.can_cancel() {
            return;
        }
        self.did_terminate = true;
        self.bootstrap_task = None;
        self.search_task = None;
        self.more_destinations_task = None;
        self.topics_task = None;
        self.send_task = None;
        let keys: HashSet<Uuid> = [
            Some(self.idempotency_key),
            self.retained_retry_request.as_ref().map(|request| request.idempotency_key),
        ]
        .into_iter()
        .flatten()
        .collect();
        let dependencies = self.dependencies.clone();
        let temporary_file_store = self.temporary_file_store.clone();
        let on_cancel = self.on_cancel.clone();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            for key in keys {
                dependencies.large_send_handoff.cancel_pending_handoff(key).await;
                dependencies.sender.discard_cached_uploads(key).await;
            }
            cx.background_spawn(async move { temporary_file_store.remove_session_files() }).await;
            cx.update(|cx| on_cancel(cx));
        })
        .detach();
    }

    async fn bootstrap(this: WeakEntity<Self>, cx: &mut AsyncApp) {
        let Ok((loader, items, dependencies, recent_limit, suggested_destination_id)) = this.read_with(cx, |model, _| {
            (
                model.item_loader.clone(),
                model.input_items.clone(),
                model.dependencies.clone(),
                model.configuration.recent_destination_limit,
                model.suggested_destination_id.clone(),
            )
        }) else {
            return;
        };
        let outcome: anyhow::Result<()> = async {
            let executor = cx.background_executor().clone();
            let payload_value = cx.spawn(async move |_| loader.load(executor, items).await);
            let account_snapshot = dependencies.snapshot.clone();
            let account_value = cx.background_spawn(async move { account_snapshot.account().await });
            let recent_snapshot = dependencies.snapshot.clone();
            let local_destinations =
                cx.background_spawn(async move { recent_snapshot.recent_destinations(recent_limit).await });
            let account = account_value.await?;
            this.update(cx, |model, cx| {
                model.account = Some(account);
                cx.notify();
            })?;
            let local = local_destinations.await?;
            this.update(cx, |model, cx| {
                model.destinations = local.clone();
                model.displayed_destinations = local.clone();
                cx.notify();
            })?;
            let loaded_payload = payload_value.await?;
            this.update(cx, |model, cx| {
                model.payload = Some(loaded_payload.clone());
                model.activity = ShareExtensionViewModelActivity::Ready;
                model.is_live_validated = false;
                cx.notify();
            })?;

            let mut fresh: Vec<ShareDestination> = Vec::new();
            match dependencies
                .remote
                .destinations(None, loaded_payload, recent_limit, 0, cx.clone())
                .await
            {
                Ok(page) => {
                    tracing::warn!(
                        target: "riseonly::X1089",
                        "share first page got={} next={:?} local={}",
                        page.destinations.len(),
                        page.next_offset,
                        local.len()
                    );
                    fresh = this.update(cx, |model, cx| {
                        model.recent_next_offset = page.next_offset;
                        let presented = Self::preserving_local_presentation(page.destinations, &local);
                        model.destinations = presented.clone();
                        model.displayed_destinations = presented.clone();
                        model.reconcile_selections(&presented);
                        model.is_live_validated = true;
                        cx.notify();
                        presented
                    })?;
                }
                Err(error) => {
                    if local.is_empty() {
                        return Err(error);
                    }
                    this.update(cx, |model, cx| {
                        model.nonfatal_message = Some(ShareErrorPresenter::message(&error));
                        cx.notify();
                    })?;
                }
            }

            let suggested = suggested_destination_id.and_then(|suggested_id| {
                fresh
                    .into_iter()
                    .find(|destination| destination.id == suggested_id && destination.is_writable)
            });
            if let Some(suggested) = suggested {
                this.update(cx, |model, cx| {
                    if suggested.has_topics {
                        model.open_topics(suggested, cx);
                    } else {
                        model.selections_by_destination_id.insert(
                            suggested.id.clone(),
                            ShareSelection {
                                destination: suggested,
                                topic: None,
                            },
                        );
                    }
                    cx.notify();
                })?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = outcome {
            if Self::is_cancellation(&error) {
                return;
            }
            let _ = this.update(cx, |model, cx| {
                model.activity = ShareExtensionViewModelActivity::Ready;
                model.set_error_message(Some(ShareErrorPresenter::message(&error)));
                cx.notify();
            });
        }
    }

    fn open_topics(&mut self, destination: ShareDestination, cx: &mut Context<Self>) {
        let Some(payload) = self.payload.clone() else {
            return;
        };
        let destination_id = destination.id.clone();
        self.topic_destination = Some(destination);
        self.topic_groups = Vec::new();
        self.is_loading_topics = true;
        let remote = self.dependencies.remote.clone();
        self.topics_task = Some(cx.spawn(async move |this, cx| {
            let outcome = remote.topics(destination_id.clone(), payload, cx.clone()).await;
            if let Err(error) = &outcome
                && Self::is_cancellation(error)
            {
                return;
            }
            let _ = this.update(cx, |model, cx| {
                if model.topic_destination.as_ref().map(|destination| destination.id.as_str())
                    != Some(destination_id.as_str())
                {
                    return;
                }
                match outcome {
                    Ok(structure) => {
                        model.topic_groups = RiseShareTopicGrouping::groups(
                            &structure.sections,
                            structure.topics,
                            |topic: &ShareTopic| topic.id.clone(),
                            |topic: &ShareTopic| topic.section_id.clone().unwrap_or_default(),
                            |topic: &ShareTopic| topic.sort_index,
                            |topic: &ShareTopic| topic.is_general,
                        );
                        model.is_loading_topics = false;
                    }
                    Err(error) => {
                        model.is_loading_topics = false;
                        model.set_error_message(Some(ShareErrorPresenter::message(&error)));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn can_add_destination(&mut self, id: &str) -> bool {
        if self.selections_by_destination_id.contains_key(id) {
            return true;
        }
        if self.selected_count() >= self.configuration.maximum_destination_count {
            self.set_error_message(None);
            self.error_title = Some(ShareStrings::too_many_destinations_title(
                self.configuration.maximum_destination_count,
            ));
            return false;
        }
        true
    }

    fn finish_successfully(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if self.did_terminate {
            return Task::ready(());
        }
        self.did_terminate = true;
        let sender = self.dependencies.sender.clone();
        let idempotency_key = self.idempotency_key;
        let temporary_file_store = self.temporary_file_store.clone();
        let on_finish = self.on_finish.clone();
        cx.spawn(async move |_, cx| {
            sender.discard_cached_uploads(idempotency_key).await;
            cx.background_spawn(async move { temporary_file_store.remove_session_files() }).await;
            cx.update(|cx| on_finish(cx));
        })
    }

    fn finish_without_cleanup(&mut self, cx: &mut Context<Self>) {
        if self.did_terminate {
            return;
        }
        self.did_terminate = true;
        let on_finish = self.on_finish.clone();
        cx.defer(move |cx| on_finish(cx));
    }

    pub fn retry_live_validation(&mut self, cx: &mut Context<Self>) {
        if self.activity != ShareExtensionViewModelActivity::Ready || self.is_searching || self.is_retry_locked {
            return;
        }
        let Some(payload) = self.payload.clone() else {
            return;
        };
        self.nonfatal_message = None;
        cx.notify();
        let remote = self.dependencies.remote.clone();
        let limit = self.configuration.recent_destination_limit;
        self.search_task = Some(cx.spawn(async move |this, cx| {
            let outcome = remote.destinations(None, payload, limit, 0, cx.clone()).await;
            let _ = this.update(cx, |model, cx| {
                match outcome {
                    Ok(page) => {
                        model.cancel_more_destinations();
                        model.recent_next_offset = page.next_offset;
                        let presented = Self::preserving_local_presentation(page.destinations, &model.destinations);
                        model.destinations = presented.clone();
                        model.displayed_destinations = presented.clone();
                        model.reconcile_selections(&presented);
                        model.is_live_validated = true;
                    }
                    Err(error) => {
                        model.nonfatal_message = Some(ShareErrorPresenter::message(&error));
                        model.is_live_validated = false;
                    }
                }
                cx.notify();
            });
        }));
    }

    fn reconcile_selections(&mut self, fresh: &[ShareDestination]) {
        let mut writable: HashMap<&str, &ShareDestination> = HashMap::new();
        for destination in fresh.iter().filter(|destination| destination.is_writable) {
            writable.entry(destination.id.as_str()).or_insert(destination);
        }
        let mut reconciled: HashMap<String, ShareSelection> = HashMap::new();
        for (id, selection) in &self.selections_by_destination_id {
            let Some(destination) = writable.get(id.as_str()) else {
                continue;
            };
            if selection.topic.is_none() && !destination.has_topics {
                reconciled.insert(
                    id.clone(),
                    ShareSelection {
                        destination: (*destination).clone(),
                        topic: None,
                    },
                );
            }
        }
        self.selections_by_destination_id = reconciled;
    }

    pub fn destination_did_appear(&mut self, destination: &ShareDestination, cx: &mut Context<Self>) {
        if self.more_destinations_task.is_some() || self.activity != ShareExtensionViewModelActivity::Ready {
            return;
        }
        let Some(payload) = self.payload.clone() else {
            return;
        };
        let Some(index) = self
            .displayed_destinations
            .iter()
            .rposition(|shown| shown.id == destination.id)
        else {
            return;
        };
        if index + PAGE_PREFETCH_DISTANCE < self.displayed_destinations.len() {
            return;
        }
        let query = self.trimmed_search_text();
        let searching = self.is_searching && !query.is_empty();
        let next_offset = if searching { self.search_next_offset } else { self.recent_next_offset };
        let Some(offset) = next_offset else {
            return;
        };
        let remote = self.dependencies.remote.clone();
        let limit = if searching {
            self.configuration.search_result_limit
        } else {
            self.configuration.recent_destination_limit
        };
        self.more_destinations_task = Some(cx.spawn(async move |this, cx| {
            let outcome = remote
                .destinations(searching.then(|| query.clone()), payload, limit, offset, cx.clone())
                .await;
            let _ = this.update(cx, |model, cx| {
                model.more_destinations_task = None;
                let page = match outcome {
                    Ok(page) => page,
                    Err(error) => {
                        tracing::warn!(target: "riseonly::X1089", "share page failed offset={} searching={} error={:?}", offset, searching, error);
                        return;
                    }
                };
                tracing::warn!(
                    target: "riseonly::X1089",
                    "share page offset={} searching={} got={} next={:?} shown={}",
                    offset,
                    searching,
                    page.destinations.len(),
                    page.next_offset,
                    model.displayed_destinations.len()
                );
                if searching {
                    if !model.is_searching
                        || model.trimmed_search_text() != query
                        || model.search_next_offset != Some(offset)
                    {
                        return;
                    }
                    model.search_next_offset = page.next_offset;
                    model.displayed_destinations =
                        Self::appending_new(page.destinations, &model.displayed_destinations);
                } else {
                    if model.recent_next_offset != Some(offset) {
                        return;
                    }
                    model.recent_next_offset = page.next_offset;
                    model.destinations = Self::appending_new(page.destinations, &model.destinations);
                    if !model.is_searching {
                        model.displayed_destinations = model.destinations.clone();
                    }
                }
                cx.notify();
            });
        }));
    }

    fn cancel_more_destinations(&mut self) {
        self.more_destinations_task = None;
    }

    fn appending_new(page: Vec<ShareDestination>, shown: &[ShareDestination]) -> Vec<ShareDestination> {
        let mut known: HashSet<String> = shown.iter().map(|destination| destination.id.clone()).collect();
        let mut result = shown.to_vec();
        result.extend(page.into_iter().filter(|destination| known.insert(destination.id.clone())));
        result
    }

    fn including_saved_match(&self, results: Vec<ShareDestination>, query: &str) -> Vec<ShareDestination> {
        if results.iter().any(|destination| destination.kind == ShareDestinationKind::Saved) {
            return results;
        }
        let Some(saved) = self
            .destinations
            .iter()
            .find(|destination| destination.kind == ShareDestinationKind::Saved && destination.is_writable)
        else {
            return results;
        };
        if !contains_case_diacritic_insensitive(&saved.title, query) {
            return results;
        }
        let mut combined = Vec::with_capacity(results.len() + 1);
        combined.push(saved.clone());
        combined.extend(results);
        combined
    }

    fn preserving_local_presentation(
        remote: Vec<ShareDestination>,
        fallback: &[ShareDestination],
    ) -> Vec<ShareDestination> {
        let mut cached_by_id: HashMap<&str, &ShareDestination> = HashMap::new();
        for destination in fallback {
            cached_by_id.entry(destination.id.as_str()).or_insert(destination);
        }
        remote
            .into_iter()
            .map(|destination| {
                let Some(cached) = cached_by_id.get(destination.id.as_str()) else {
                    return destination;
                };
                ShareDestination {
                    avatar_url: destination.avatar_url.or_else(|| cached.avatar_url.clone()),
                    avatar_color_key: destination.avatar_color_key.or_else(|| cached.avatar_color_key.clone()),
                    ..destination
                }
            })
            .collect()
    }

    pub fn finish_partial_delivery(&mut self, cx: &mut Context<Self>) {
        if !self.has_partial_delivery || self.did_terminate {
            return;
        }
        self.did_terminate = true;
        let sender = self.dependencies.sender.clone();
        let idempotency_key = self.idempotency_key;
        let temporary_file_store = self.temporary_file_store.clone();
        let on_finish = self.on_finish.clone();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            sender.discard_cached_uploads(idempotency_key).await;
            cx.background_spawn(async move { temporary_file_store.remove_session_files() }).await;
            cx.update(|cx| on_finish(cx));
        })
        .detach();
    }

    fn unlock_failed_operation_without_delivery(&mut self, cx: &mut Context<Self>) {
        let Some(retained_retry_request) = self.retained_retry_request.take() else {
            return;
        };
        self.is_retry_locked = false;
        self.is_commit_started = false;
        let failed_key = retained_retry_request.idempotency_key;
        self.idempotency_key = Uuid::new_v4();
        let sender = self.dependencies.sender.clone();
        cx.spawn(async move |_, _| sender.discard_cached_uploads(failed_key).await)
            .detach();
    }

    /// true for a cancelled operation, it ends silently with no alert
    fn is_cancellation(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| cause.is::<CancellationError>())
    }

    fn mark_commit_started(&mut self, cx: &mut Context<Self>) {
        self.is_commit_started = true;
        cx.notify();
    }
}
