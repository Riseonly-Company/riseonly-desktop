use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::channel::oneshot;
use futures::future::{BoxFuture, FutureExt, LocalBoxFuture, Shared};
use gpui::AsyncApp;
use rise_canon::rise_key::events;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_url::{Url, UrlComponents, UrlQueryItem};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::modules::share_extension::entry::share_extension_configuration::ShareExtensionConfiguration;
use crate::modules::share_extension::entry::share_extension_dependencies::{
    ShareContainingAppOpening, ShareLargeSendHandling,
};
use crate::modules::share_extension::entry::share_extension_models::{ShareExtensionError, ShareSendRequest};
use crate::modules::share_extension::shared::rise_share_container::{RiseShareContainer, RiseShareContainerError};
use crate::modules::share_extension::shared::rise_share_models::{
    RiseShareSendAttachment, RiseShareSendDestination, RiseShareSendSession,
};

const SHARE_SEND_HOST: &str = "share-send";

/// detached move of the temp folder into the send sessions, tracks cancelled and finished
#[derive(Clone)]
struct ShareSendMoveTask {
    cancelled: Arc<AtomicBool>,
    finished: Shared<BoxFuture<'static, ()>>,
}

impl ShareSendMoveTask {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct AppGroupShareSendHandoffState {
    durable_session_by_idempotency_key: HashMap<Uuid, String>,
    move_tasks_by_idempotency_key: HashMap<Uuid, ShareSendMoveTask>,
    app_owned_session_ids: HashSet<String>,
}

/// share send handoff, state is a RefCell touched only between awaits on the foreground executor
pub struct AppGroupShareSendHandoff {
    configuration: ShareExtensionConfiguration,
    account_id: String,
    app_opener: Weak<dyn ShareContainingAppOpening>,
    state: RefCell<AppGroupShareSendHandoffState>,
}

struct ShareSendMoveCancellation<'a> {
    state: &'a RefCell<AppGroupShareSendHandoffState>,
    idempotency_key: Uuid,
    move_task: ShareSendMoveTask,
    armed: bool,
}

impl Drop for ShareSendMoveCancellation<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.move_task.cancel();
        self.state
            .borrow_mut()
            .move_tasks_by_idempotency_key
            .remove(&self.idempotency_key);
    }
}

impl AppGroupShareSendHandoff {
    /// builds the send handoff
    pub fn new(
        configuration: ShareExtensionConfiguration,
        account_id: String,
        app_opener: &Rc<dyn ShareContainingAppOpening>,
    ) -> Self {
        Self {
            configuration,
            account_id,
            app_opener: Rc::downgrade(app_opener),
            state: RefCell::new(AppGroupShareSendHandoffState::default()),
        }
    }

    async fn handoff_request(&self, request: ShareSendRequest, cx: AsyncApp) -> anyhow::Result<()> {
        let idempotency_key = request.idempotency_key;
        let existing = self
            .state
            .borrow()
            .durable_session_by_idempotency_key
            .get(&idempotency_key)
            .cloned();
        let session_id = match existing {
            Some(existing) => existing,
            None => self.move_session(&request).await?,
        };

        let mut components = UrlComponents::new();
        components.set_scheme(Some(&self.configuration.story_url_scheme));
        components.set_host(Some(SHARE_SEND_HOST));
        components.set_query_items(Some(vec![UrlQueryItem::new(
            events::SESSION_ID,
            Some(session_id.clone()),
        )]));
        let (Some(url), Some(app_opener)) = (components.url(), self.app_opener.upgrade()) else {
            return Err(ShareExtensionError::AppOpenFailed.into());
        };
        if !app_opener.open_containing_app(url, cx).await {
            return Err(ShareExtensionError::AppOpenFailed.into());
        }
        self.state.borrow_mut().app_owned_session_ids.insert(session_id);
        Ok(())
    }

    async fn move_session(&self, request: &ShareSendRequest) -> anyhow::Result<String> {
        let idempotency_key = request.idempotency_key;
        let session = Self::make_session(request, &self.account_id)?;
        let source_directory = request.payload.session_directory_url.clone();
        self.state
            .borrow_mut()
            .durable_session_by_idempotency_key
            .insert(idempotency_key, session.id.clone());

        let cancelled = Arc::new(AtomicBool::new(false));
        let (value_sender, value_receiver) = oneshot::channel();
        let (finished_sender, finished_receiver) = oneshot::channel::<()>();
        let is_cancelled = cancelled.clone();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let moved = RiseShareContainer::create_send_session(session, &source_directory, &|| {
                is_cancelled.load(Ordering::SeqCst)
            })
            .map(|(moved, _)| moved);
            let _ = value_sender.send(moved);
            let _ = finished_sender.send(());
        }));
        let move_task = ShareSendMoveTask {
            cancelled,
            finished: finished_receiver.map(|_| ()).boxed().shared(),
        };
        self.state
            .borrow_mut()
            .move_tasks_by_idempotency_key
            .insert(idempotency_key, move_task.clone());

        let mut cancellation = ShareSendMoveCancellation {
            state: &self.state,
            idempotency_key,
            move_task: move_task.clone(),
            armed: true,
        };
        let outcome = value_receiver.await;
        cancellation.armed = false;
        drop(cancellation);

        let moved = match outcome {
            Ok(Ok(moved)) => moved,
            Ok(Err(error)) => {
                move_task.cancel();
                let mut state = self.state.borrow_mut();
                state.move_tasks_by_idempotency_key.remove(&idempotency_key);
                if !matches!(error, RiseShareContainerError::Cancelled(_)) {
                    state.durable_session_by_idempotency_key.remove(&idempotency_key);
                }
                return Err(error.into());
            }
            Err(_) => {
                move_task.cancel();
                self.state
                    .borrow_mut()
                    .move_tasks_by_idempotency_key
                    .remove(&idempotency_key);
                return Err(CancellationError.into());
            }
        };
        self.state
            .borrow_mut()
            .move_tasks_by_idempotency_key
            .remove(&idempotency_key);
        Ok(moved.id)
    }

    async fn cancel_pending(&self, idempotency_key: Uuid) {
        let (session_id, move_task) = {
            let state = self.state.borrow();
            let Some(session_id) = state.durable_session_by_idempotency_key.get(&idempotency_key).cloned()
            else {
                return;
            };
            if state.app_owned_session_ids.contains(&session_id) {
                return;
            }
            (session_id, state.move_tasks_by_idempotency_key.get(&idempotency_key).cloned())
        };
        if let Some(move_task) = move_task {
            move_task.cancel();
            move_task.finished.await;
            self.state
                .borrow_mut()
                .move_tasks_by_idempotency_key
                .remove(&idempotency_key);
        }
        let expected_account_id = self.account_id.clone();
        let _ = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                RiseShareContainer::discard_cancelled_send_session(&session_id, &expected_account_id);
            })
            .await;
        self.state
            .borrow_mut()
            .durable_session_by_idempotency_key
            .remove(&idempotency_key);
    }

    fn make_session(
        request: &ShareSendRequest,
        account_id: &str,
    ) -> Result<RiseShareSendSession, RiseShareContainerError> {
        let session_directory = Url::from_file_path(&request.payload.session_directory_url.join(""))
            .map(|url| url.standardized());
        let attachments = request
            .payload
            .attachments
            .iter()
            .map(|attachment| {
                let mut relative_path = None;
                if let Some(local_file_url) = &attachment.local_file_url {
                    let local_url = Url::from_file_path(local_file_url).map(|url| url.standardized());
                    let (Some(local_url), Some(session_directory)) = (local_url, session_directory.as_ref())
                    else {
                        return Err(RiseShareContainerError::UnsafeRelativePath);
                    };
                    if local_url.deleting_last_path_component() != *session_directory {
                        return Err(RiseShareContainerError::UnsafeRelativePath);
                    }
                    relative_path = Some(local_url.last_path_component());
                }
                Ok(RiseShareSendAttachment {
                    id: attachment.id.uuid_string().to_lowercase(),
                    kind: attachment.kind.raw_value().to_owned(),
                    content_type_identifier: attachment.content_type_identifier.clone(),
                    mime_type: attachment.mime_type.clone(),
                    original_file_name: attachment.original_file_name.clone(),
                    relative_path,
                    text: attachment.text.clone(),
                    byte_count: attachment.byte_count,
                    source_index: attachment.source_index,
                    uploaded_media: None,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let destinations = request
            .selections
            .iter()
            .map(|selection| RiseShareSendDestination {
                chat_id: selection.destination.id.clone(),
                topic_id: selection.wire_topic_id().map(str::to_owned),
                title: selection.destination.title.clone(),
            })
            .collect();
        let now = (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64;
        Ok(RiseShareSendSession::new(
            Uuid::new_v4().uuid_string().to_lowercase(),
            account_id.to_owned(),
            now,
            request.idempotency_key.uuid_string().to_lowercase(),
            destinations,
            request.comment.clone(),
            request.payload.textual_content(),
            attachments,
        ))
    }
}

impl ShareLargeSendHandling for AppGroupShareSendHandoff {
    fn handoff(&self, request: ShareSendRequest, cx: AsyncApp) -> LocalBoxFuture<'_, anyhow::Result<()>> {
        self.handoff_request(request, cx).boxed_local()
    }

    fn cancel_pending_handoff(&self, idempotency_key: Uuid) -> LocalBoxFuture<'_, ()> {
        self.cancel_pending(idempotency_key).boxed_local()
    }
}
