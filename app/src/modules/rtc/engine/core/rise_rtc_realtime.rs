use std::sync::Arc;

use rise_engine::rise_sync::RiseSyncPushHandler;

use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::modules::rtc::engine::rise_rtc_repository::RiseRTCRepository;

impl RiseRTCRepository {
    pub async fn register(
        self: &Arc<Self>,
        context: &RiseAppEngineBootstrapContext,
    ) -> anyhow::Result<()> {
        let repository = Arc::downgrade(self);
        context
            .register_push_handler(RiseSyncPushHandler::new(Self::PUSH_EVENT_TYPE, move |push| {
                let repository = repository.clone();
                async move {
                    let Some(repository) = repository.upgrade() else {
                        return Ok(None);
                    };
                    repository.make_call_push_batch(push).await
                }
            }))
            .await
    }
}
