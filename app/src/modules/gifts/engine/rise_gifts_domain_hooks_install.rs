use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineGiftsDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::gifts::engine::rise_gifts_domain::RiseGiftsDomain;

pub(crate) fn install_rise_gifts_domain_hooks() {
    RiseAppEngineGiftsDomainHooks::install(RiseAppEngineGiftsDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let gifts = RiseGiftsDomain::new(context.engine.clone());
                if let Err(error) = gifts.prepare(&context).await {
                    gifts.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(gifts))
            }
            .boxed()
        }),
    });
}

fn lifetime(gifts: Arc<RiseGiftsDomain>) -> RiseAppEngineDomainLifetime {
    let activate = gifts.clone();
    let did_activate_ownership = gifts.clone();
    let socket_connected = gifts.clone();
    let shutdown = gifts;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let gifts = activate.clone();
            async move { gifts.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let gifts = did_activate_ownership.clone();
            async move { gifts.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let gifts = socket_connected.clone();
            async move { gifts.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let gifts = shutdown.clone();
            async move { gifts.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
