use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineSparksDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::sparks::engine::rise_sparks_domain::RiseSparksDomain;

pub fn install_rise_sparks_domain_hooks() {
    RiseAppEngineSparksDomainHooks::install(RiseAppEngineSparksDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let sparks = RiseSparksDomain::new(context.engine.clone());
                if let Err(error) = sparks.prepare(&context).await {
                    sparks.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(sparks))
            }
            .boxed()
        }),
    });
}

fn lifetime(sparks: Arc<RiseSparksDomain>) -> RiseAppEngineDomainLifetime {
    let activate = sparks.clone();
    let did_activate_ownership = sparks.clone();
    let socket_connected = sparks.clone();
    let shutdown = sparks;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let sparks = activate.clone();
            async move { sparks.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let sparks = did_activate_ownership.clone();
            async move { sparks.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let sparks = socket_connected.clone();
            async move { sparks.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let sparks = shutdown.clone();
            async move { sparks.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
