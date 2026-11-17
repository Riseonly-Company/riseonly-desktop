use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::{App, AsyncApp};
use rise_engine::{RiseWireFrameSender, RiseWireSocketSession};

use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::rise_engine_bridge::rise_engine_trace_bridge::RiseEngineTraceBridge;

#[derive(Clone)]
pub struct RiseWireSaiSocketAdapter {
    main: RiseEngineMainHop,
}

impl RiseWireSaiSocketAdapter {
    pub fn new(cx: &mut App) -> Self {
        Self { main: RiseEngineMainHop::shared(cx) }
    }
}

impl RiseWireFrameSender for RiseWireSaiSocketAdapter {
    fn send(
        &self,
        frame: Vec<u8>,
        session: RiseWireSocketSession,
    ) -> BoxFuture<'static, anyhow::Result<()>> {
        let main = self.main.clone();
        async move {
            let text = String::from_utf8(frame).map_err(|_| {
                anyhow::Error::new(SaiWsSendError::EncodingFailed(
                    "RiseWire UTF-8 frame".to_owned(),
                ))
            })?;
            let request_summary = if RiseEngineTraceBridge::DEBUG_TOOLS {
                RiseEngineTraceBridge::request_summary(text.as_bytes())
            } else {
                String::new()
            };
            main.run(move |cx: AsyncApp| {
                async move {
                    let manager = cx.update(SaiWebSocketManager::shared);
                    SaiWebSocketManager::rise_wire_send_prepared_text(
                        manager,
                        text,
                        session,
                        request_summary,
                        cx,
                    )
                    .await
                }
                .boxed_local()
            })
            .await
        }
        .boxed()
    }
}
