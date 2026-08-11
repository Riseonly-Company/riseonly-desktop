use std::sync::Arc;

use rise_engine::RiseWire;
use tokio::sync::watch;

use super::rise_post_repository::{
    LivePostTransport, PostCommand, PostRepository, PostSnapshot, PostTransport,
};

pub struct RisePostDomain {
    repository: PostRepository,
}

impl RisePostDomain {
    pub fn open(runtime: &tokio::runtime::Handle, wire: Arc<RiseWire>) -> Self {
        Self {
            repository: PostRepository::spawn(
                runtime,
                Arc::new(LivePostTransport::new(wire)) as Arc<dyn PostTransport>,
            ),
        }
    }

    pub fn snapshot(&self) -> Arc<PostSnapshot> {
        self.repository.snapshot()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<PostSnapshot>> {
        self.repository.subscribe()
    }

    pub fn dispatch(&self, command: PostCommand) {
        self.repository.dispatch(command);
    }
}
