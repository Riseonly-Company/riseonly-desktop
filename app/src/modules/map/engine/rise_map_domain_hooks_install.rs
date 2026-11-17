use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineMapDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::map::engine::rise_map_domain::RiseMapDomain;

pub fn install_rise_map_domain_hooks() {
    RiseAppEngineMapDomainHooks::install(RiseAppEngineMapDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let map = RiseMapDomain::new(context.engine.clone());
                if let Err(error) = map.prepare(&context).await {
                    map.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(map))
            }
            .boxed()
        }),
    });
}

fn lifetime(map: Arc<RiseMapDomain>) -> RiseAppEngineDomainLifetime {
    let activate = map.clone();
    let did_activate_ownership = map.clone();
    let socket_connected = map.clone();
    let application_did_become_active = map.clone();
    let shutdown = map;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let map = activate.clone();
            async move { map.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let map = did_activate_ownership.clone();
            async move { map.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let map = socket_connected.clone();
            async move { map.socket_connected().await }.boxed()
        }),
        application_did_become_active: Arc::new(move || {
            let map = application_did_become_active.clone();
            async move { map.application_did_become_active().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let map = shutdown.clone();
            async move { map.shutdown().await }.boxed()
        }),
    })
}
