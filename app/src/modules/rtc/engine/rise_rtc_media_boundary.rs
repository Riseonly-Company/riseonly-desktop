use std::sync::{Arc, LazyLock};

use futures::FutureExt as _;
use futures::channel::mpsc::{self, UnboundedReceiver};
use futures::future::BoxFuture;
use rise_core::rise_isolated::RiseIsolated;

use crate::modules::rtc::engine::rise_rtc_engine_models::{
    RiseRTCMediaDriving, RiseRTCMediaEvent, RiseRTCMediaIntent, RiseRTCStateError,
};

pub type RiseRTCMediaRuntimeFactory = Arc<dyn Fn() -> Arc<dyn RiseRTCMediaDriving> + Send + Sync>;

static SHARED: LazyLock<RiseRTCMediaRuntime> = LazyLock::new(RiseRTCMediaRuntime::new);

/// injection point, the livekit controller installs a factory at startup
/// so the engine core and tests never link livekit
pub struct RiseRTCMediaRuntime {
    factory: RiseIsolated<RiseRTCMediaRuntimeFactory>,
}

impl RiseRTCMediaRuntime {
    pub fn shared() -> &'static RiseRTCMediaRuntime {
        &SHARED
    }

    fn new() -> Self {
        Self { factory: RiseIsolated::new(Self::null_factory()) }
    }

    fn null_factory() -> RiseRTCMediaRuntimeFactory {
        Arc::new(|| Arc::new(RiseRTCNullMediaDriver) as Arc<dyn RiseRTCMediaDriving>)
    }

    pub fn install(&self, factory: RiseRTCMediaRuntimeFactory) {
        self.factory.with(|current| *current = factory);
    }

    pub fn reset(&self) {
        self.factory.with(|current| *current = Self::null_factory());
    }

    pub fn make_driver(&self) -> Arc<dyn RiseRTCMediaDriving> {
        let factory = self.factory.with(|current| current.clone());
        factory()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RiseRTCNullMediaDriver;

impl RiseRTCMediaDriving for RiseRTCNullMediaDriver {
    fn perform(&self, intent: RiseRTCMediaIntent) -> BoxFuture<'_, anyhow::Result<()>> {
        let result = match intent {
            RiseRTCMediaIntent::Connect(_) => Err(RiseRTCStateError::Remote("rtc_media_driver_unavailable".to_owned()).into()),
            RiseRTCMediaIntent::Disconnect { .. }
            | RiseRTCMediaIntent::SetMicrophone { .. }
            | RiseRTCMediaIntent::SetRemotePlayoutMuted { .. }
            | RiseRTCMediaIntent::SetListenerVolumes { .. }
            | RiseRTCMediaIntent::SetCamera { .. }
            | RiseRTCMediaIntent::SetScreenShare { .. }
            | RiseRTCMediaIntent::SwitchCamera { .. }
            | RiseRTCMediaIntent::SelectAudioInput { .. }
            | RiseRTCMediaIntent::SelectAudioOutput { .. }
            | RiseRTCMediaIntent::SetEncryptionKey { .. } => Ok(()),
            RiseRTCMediaIntent::SendData { .. } => {
                Err(RiseRTCStateError::Remote("rtc_livekit_data_publish_disabled".to_owned()).into())
            }
        };
        async move { result }.boxed()
    }

    fn events(&self) -> BoxFuture<'_, UnboundedReceiver<RiseRTCMediaEvent>> {
        let (_sender, receiver) = mpsc::unbounded();
        async move { receiver }.boxed()
    }

    fn is_null_media_driver(&self) -> bool {
        true
    }
}
