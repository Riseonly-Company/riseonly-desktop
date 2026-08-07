use std::sync::Arc;

use rise_engine::RiseWire;
use tokio::sync::watch;

use super::rise_session_repository::{
    LiveSessionTransport, SessionCommand, SessionRepository, SessionSnapshot, SessionTransport,
};

/// The composition root for session-service.
///
/// Account-scoped in principle — the list belongs to one account — but built on
/// the process-wide socket, exactly as every other domain will be: the socket
/// outlives an account and the DATA does not, which is why the domain is dropped
/// on a switch and the transport is not.
pub struct RiseSessionDomain {
    repository: SessionRepository,
}

impl RiseSessionDomain {
    pub fn open(runtime: &tokio::runtime::Handle, wire: Arc<RiseWire>) -> Self {
        Self {
            repository: SessionRepository::spawn(
                runtime,
                Arc::new(LiveSessionTransport::new(wire)) as Arc<dyn SessionTransport>,
            ),
        }
    }

    pub fn snapshot(&self) -> Arc<SessionSnapshot> {
        self.repository.snapshot()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<SessionSnapshot>> {
        self.repository.subscribe()
    }

    pub fn dispatch(&self, command: SessionCommand) {
        self.repository.dispatch(command);
    }
}
