use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineSearchDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::search::engine::rise_search_domain::RiseSearchDomain;

pub fn install_rise_search_domain_hooks() {
    RiseAppEngineSearchDomainHooks::install(RiseAppEngineSearchDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let search = RiseSearchDomain::new(context.engine.clone());
                if let Err(error) = search.prepare(&context).await {
                    search.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(search))
            }
            .boxed()
        }),
    });
}

fn lifetime(search: Arc<RiseSearchDomain>) -> RiseAppEngineDomainLifetime {
    let activate = search.clone();
    let did_activate_ownership = search.clone();
    let socket_connected = search.clone();
    let shutdown = search;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let search = activate.clone();
            async move { search.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let search = did_activate_ownership.clone();
            async move { search.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let search = socket_connected.clone();
            async move { search.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let search = shutdown.clone();
            async move { search.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
