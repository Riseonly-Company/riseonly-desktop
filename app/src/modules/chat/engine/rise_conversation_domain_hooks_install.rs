use std::any::Any;
use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineConversationDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomain;

pub(crate) fn install_rise_conversation_domain_hooks() {
    RiseAppEngineConversationDomainHooks::install(RiseAppEngineConversationDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let conversation = RiseConversationDomain::new(context.engine.clone());
                if let Err(error) = conversation.prepare(&context).await {
                    conversation.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(conversation))
            }
            .boxed()
        }),
    });
}

fn lifetime(conversation: Arc<RiseConversationDomain>) -> RiseAppEngineDomainLifetime {
    let activate = conversation.clone();
    let did_activate_ownership = conversation.clone();
    let socket_connected = conversation.clone();
    let application_did_become_active = conversation.clone();
    let shutdown = conversation.clone();
    let domain: Arc<dyn Any + Send + Sync> = conversation;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let conversation = activate.clone();
            async move { conversation.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let conversation = did_activate_ownership.clone();
            async move { conversation.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let conversation = socket_connected.clone();
            async move { conversation.socket_connected().await }.boxed()
        }),
        application_did_become_active: Arc::new(move || {
            let conversation = application_did_become_active.clone();
            async move { conversation.application_did_become_active().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let conversation = shutdown.clone();
            async move { conversation.shutdown().await }.boxed()
        }),
    })
    .with_domain(domain)
}
