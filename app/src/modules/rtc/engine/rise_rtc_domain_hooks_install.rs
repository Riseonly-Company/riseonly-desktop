use std::any::Any;
use std::sync::{Arc, Weak};

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineRtcDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::chat::engine::messages::rise_call_history_message::RiseCallHistoryRecord;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomain;
use crate::modules::rtc::engine::rise_rtc_domain::{RiseRTCCallHistorySink, RiseRTCDomain};

pub(crate) fn install_rise_rtc_domain_hooks() {
    RiseAppEngineRtcDomainHooks::install(RiseAppEngineRtcDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let rtc = RiseRTCDomain::new(context.engine.clone());
                if let Err(error) = rtc.prepare(&context).await {
                    rtc.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(rtc))
            }
            .boxed()
        }),
        set_call_history_sink: Box::new(|rtc, conversation| {
            async move {
                let Ok(rtc) = rtc.downcast::<RiseRTCDomain>() else {
                    return;
                };
                rtc.set_call_history_sink(call_history_sink(conversation)).await;
            }
            .boxed()
        }),
    });
}

/// conversation is held weakly so a sink that outlives it
/// drops the record and doesnt keep the chat domain alive
fn call_history_sink(conversation: Weak<dyn Any + Send + Sync>) -> RiseRTCCallHistorySink {
    Arc::new(move |record: RiseCallHistoryRecord| {
        let conversation = conversation.clone();
        async move {
            let Some(conversation) = conversation.upgrade() else {
                return;
            };
            let Ok(conversation) = conversation.downcast::<RiseConversationDomain>() else {
                return;
            };
            conversation.ingest_call_history(record).await;
        }
        .boxed()
    })
}

fn lifetime(rtc: Arc<RiseRTCDomain>) -> RiseAppEngineDomainLifetime {
    let activate = rtc.clone();
    let did_activate_ownership = rtc.clone();
    let socket_connected = rtc.clone();
    let application_did_become_active = rtc.clone();
    let shutdown = rtc.clone();
    let domain: Arc<dyn Any + Send + Sync> = rtc;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let rtc = activate.clone();
            async move { rtc.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let rtc = did_activate_ownership.clone();
            async move { rtc.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let rtc = socket_connected.clone();
            async move { rtc.socket_connected().await }.boxed()
        }),
        application_did_become_active: Arc::new(move || {
            let rtc = application_did_become_active.clone();
            async move { rtc.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let rtc = shutdown.clone();
            async move { rtc.shutdown().await }.boxed()
        }),
    })
    .with_domain(domain)
}
