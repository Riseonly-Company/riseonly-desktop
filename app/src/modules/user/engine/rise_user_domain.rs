use std::sync::Arc;

use rise_engine::RiseWire;
use tokio::sync::watch;

use super::rise_user_presentation::UserSnapshot;
use super::rise_user_repository::{
    FOLLOW_DEBOUNCE, LiveUserTransport, UserCommand, UserRepository, UserTransport,
};

pub struct RiseUserDomain {
    repository: UserRepository,
}

impl RiseUserDomain {
    pub fn open(runtime: &tokio::runtime::Handle, wire: Arc<RiseWire>) -> Self {
        Self {
            repository: UserRepository::spawn(
                runtime,
                Arc::new(LiveUserTransport::new(wire)) as Arc<dyn UserTransport>,
                FOLLOW_DEBOUNCE,
            ),
        }
    }

    pub fn snapshot(&self) -> Arc<UserSnapshot> {
        self.repository.snapshot()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<UserSnapshot>> {
        self.repository.subscribe()
    }

    pub fn dispatch(&self, command: UserCommand) {
        self.repository.dispatch(command);
    }
}
