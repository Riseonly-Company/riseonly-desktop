use std::sync::Arc;

use futures::future::BoxFuture;
use rise_engine::{MethodDescriptor, RiseWire, WireError};
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use super::core::rise_session_rpc as rpc;
use super::rise_session_engine_models::{SessionEntry, normalize};

/// How many rows a page asks for. The reference's twenty.
const PAGE_SIZE: u32 = 20;

pub trait SessionTransport: Send + Sync {
    fn call(
        &self,
        descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>>;
}

pub struct LiveSessionTransport {
    wire: Arc<RiseWire>,
}

impl LiveSessionTransport {
    pub fn new(wire: Arc<RiseWire>) -> Self {
        Self { wire }
    }
}

impl SessionTransport for LiveSessionTransport {
    fn call(
        &self,
        descriptor: &'static MethodDescriptor,
        body: Value,
    ) -> BoxFuture<'static, Result<Value, WireError>> {
        let wire = Arc::clone(&self.wire);
        Box::pin(async move {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_millis() as u64)
                .unwrap_or_default();
            wire.call(descriptor, body, now).await
        })
    }
}

/// Who the requests are made as.
///
/// Supplied per call rather than held, because the account can change under a
/// long-lived repository and a request stamped with the previous user's id is
/// one the gateway will refuse — or, worse, answer.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SessionIdentity {
    pub user_id: String,
    pub session_id: String,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SessionSnapshot {
    pub revision: u64,
    pub rows: Vec<SessionEntry>,
    pub is_loading: bool,
    pub is_mutating: bool,
    pub has_more: bool,
    pub cursor: Option<String>,
    pub error_key: Option<&'static str>,
    /// Set once, when the server says this client ended its own session. The
    /// shell reads it and signs out; the repository does not, because signing
    /// out is auth's job.
    pub should_sign_out: bool,
    pub did_load: bool,
}

pub enum SessionCommand {
    Load {
        identity: SessionIdentity,
        force: bool,
    },
    LoadMore {
        identity: SessionIdentity,
    },
    Terminate {
        identity: SessionIdentity,
        session_id: String,
    },
    TerminateAllOthers {
        identity: SessionIdentity,
    },
    Reset,
    Shutdown,
}

pub struct SessionRepository {
    commands: mpsc::Sender<SessionCommand>,
    snapshot: watch::Receiver<Arc<SessionSnapshot>>,
}

impl SessionRepository {
    pub fn spawn(runtime: &tokio::runtime::Handle, transport: Arc<dyn SessionTransport>) -> Self {
        let (commands, inbox) = mpsc::channel(32);
        let (publisher, snapshot) = watch::channel(Arc::new(SessionSnapshot::default()));

        runtime.spawn(
            SessionState {
                transport,
                publisher,
                revision: 0,
                rows: Vec::new(),
                previous: None,
                is_loading: false,
                is_mutating: false,
                has_more: false,
                cursor: None,
                error_key: None,
                should_sign_out: false,
                did_load: false,
            }
            .run(inbox),
        );

        Self { commands, snapshot }
    }

    pub fn snapshot(&self) -> Arc<SessionSnapshot> {
        Arc::clone(&self.snapshot.borrow())
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<SessionSnapshot>> {
        self.snapshot.clone()
    }

    pub fn dispatch(&self, command: SessionCommand) {
        let _ = self.commands.try_send(command);
    }
}

struct SessionState {
    transport: Arc<dyn SessionTransport>,
    publisher: watch::Sender<Arc<SessionSnapshot>>,
    revision: u64,
    rows: Vec<SessionEntry>,
    /// What to restore if an optimistic removal is refused.
    previous: Option<Vec<SessionEntry>>,
    is_loading: bool,
    is_mutating: bool,
    has_more: bool,
    cursor: Option<String>,
    error_key: Option<&'static str>,
    should_sign_out: bool,
    did_load: bool,
}

impl SessionState {
    async fn run(mut self, mut inbox: mpsc::Receiver<SessionCommand>) {
        while let Some(command) = inbox.recv().await {
            match command {
                SessionCommand::Load { identity, force } => self.load(identity, force).await,
                SessionCommand::LoadMore { identity } => self.load_more(identity).await,
                SessionCommand::Terminate {
                    identity,
                    session_id,
                } => self.terminate(identity, Some(session_id), false).await,
                SessionCommand::TerminateAllOthers { identity } => {
                    self.terminate(identity, None, true).await;
                }
                SessionCommand::Reset => {
                    self.rows.clear();
                    self.previous = None;
                    self.cursor = None;
                    self.has_more = false;
                    self.did_load = false;
                    self.should_sign_out = false;
                    self.error_key = None;
                    self.publish();
                }
                SessionCommand::Shutdown => break,
            }
        }
    }

    async fn load(&mut self, identity: SessionIdentity, force: bool) {
        if self.is_loading || (self.did_load && !force) {
            return;
        }

        self.is_loading = true;
        self.error_key = None;
        self.publish();

        let request = rpc::GetUserSessionsRequest {
            user_id: identity.user_id.clone(),
            current_session_id: identity.session_id.clone(),
            relative_id: None,
            limit: PAGE_SIZE,
        };

        match self.fetch(request, &identity).await {
            Ok(page) => {
                self.rows = normalize(page.rows);
                self.cursor = page.cursor;
                self.has_more = page.has_more;
                self.did_load = true;
            }
            // A failed refresh never clears a page that is already on screen:
            // the cache stays visible and the error is shown beside it.
            Err(key) => self.error_key = Some(key),
        }

        self.is_loading = false;
        self.publish();
    }

    async fn load_more(&mut self, identity: SessionIdentity) {
        // One in-flight edge request, and never one past the end. Without both,
        // a list that reaches its bottom asks for the same page forever.
        if self.is_loading || !self.has_more {
            return;
        }
        let Some(cursor) = self.cursor.clone() else {
            return;
        };

        self.is_loading = true;
        self.publish();

        let request = rpc::GetUserSessionsRequest {
            user_id: identity.user_id.clone(),
            current_session_id: identity.session_id.clone(),
            relative_id: Some(cursor),
            limit: PAGE_SIZE,
        };

        match self.fetch(request, &identity).await {
            Ok(page) => {
                let known: std::collections::BTreeSet<String> =
                    self.rows.iter().map(|row| row.id.clone()).collect();
                self.rows
                    .extend(page.rows.into_iter().filter(|row| !known.contains(&row.id)));
                self.rows = normalize(std::mem::take(&mut self.rows));
                self.cursor = page.cursor;
                self.has_more = page.has_more;
            }
            // An error on a later page must not block the edge forever: the
            // cursor is kept, so scrolling again retries.
            Err(key) => self.error_key = Some(key),
        }

        self.is_loading = false;
        self.publish();
    }

    async fn fetch(
        &self,
        request: rpc::GetUserSessionsRequest,
        _identity: &SessionIdentity,
    ) -> Result<Page, &'static str> {
        let current = request.current_session_id.clone();
        let body = serde_json::to_value(request).unwrap_or(Value::Null);

        let payload = self
            .transport
            .call(&rpc::GET_USER_SESSIONS, body)
            .await
            .map_err(|_| "settings_sessions_title")?;

        let response: rpc::GetUserSessionsResponse =
            serde_json::from_value(payload).unwrap_or_default();
        if response.error.is_some() {
            return Err("settings_sessions_title");
        }

        Ok(Page {
            rows: response
                .sessions
                .iter()
                .map(|dto| SessionEntry::from_dto(dto, &current))
                .collect(),
            cursor: response.relative_id,
            has_more: response.is_have_more.unwrap_or(false),
        })
    }

    async fn terminate(
        &mut self,
        identity: SessionIdentity,
        session_id: Option<String>,
        is_all: bool,
    ) {
        if self.is_mutating {
            return;
        }

        // Optimistic, with the previous list kept for the rollback. The row goes
        // as soon as it is pressed; if the server refuses, it comes back.
        self.previous = Some(self.rows.clone());
        self.rows = match (&session_id, is_all) {
            (_, true) => self
                .rows
                .iter()
                .filter(|row| row.is_current)
                .cloned()
                .collect(),
            (Some(id), false) => self
                .rows
                .iter()
                .filter(|row| &row.id != id)
                .cloned()
                .collect(),
            (None, false) => self.rows.clone(),
        };
        self.is_mutating = true;
        self.error_key = None;
        self.publish();

        let body = serde_json::to_value(rpc::DeleteUserSessionsRequest {
            user_id: identity.user_id.clone(),
            session_id_to_terminate: session_id.clone().unwrap_or_default(),
            is_all,
            current_session_id: identity.session_id.clone(),
        })
        .unwrap_or(Value::Null);

        match self.transport.call(&rpc::DELETE_USER_SESSIONS, body).await {
            Ok(payload) => {
                let response: rpc::DeleteUserSessionsResponse =
                    serde_json::from_value(payload).unwrap_or_default();

                if response.error.is_some() {
                    self.rollback();
                } else {
                    if response.should_logout == Some(true) {
                        self.should_sign_out = true;
                    }
                    if let Some(remaining) = response.remaining_sessions {
                        self.rows = normalize(
                            remaining
                                .iter()
                                .map(|dto| SessionEntry::from_dto(dto, &identity.session_id))
                                .collect(),
                        );
                    }
                    self.previous = None;
                }
            }
            Err(_) => self.rollback(),
        }

        self.is_mutating = false;
        self.publish();
    }

    fn rollback(&mut self) {
        if let Some(previous) = self.previous.take() {
            self.rows = previous;
        }
        self.error_key = Some("settings_sessions_title");
    }

    fn publish(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.publisher.send_replace(Arc::new(SessionSnapshot {
            revision: self.revision,
            rows: self.rows.clone(),
            is_loading: self.is_loading,
            is_mutating: self.is_mutating,
            has_more: self.has_more,
            cursor: self.cursor.clone(),
            error_key: self.error_key,
            should_sign_out: self.should_sign_out,
            did_load: self.did_load,
        }));
    }
}

struct Page {
    rows: Vec<SessionEntry>,
    cursor: Option<String>,
    has_more: bool,
}

#[cfg(test)]
#[path = "rise_session_repository_tests.rs"]
mod tests;
