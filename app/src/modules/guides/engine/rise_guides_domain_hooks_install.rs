use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineGuidesDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::guides::engine::rise_guides_domain::RiseGuidesDomain;

pub fn install_rise_guides_domain_hooks() {
    RiseAppEngineGuidesDomainHooks::install(RiseAppEngineGuidesDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let guides = RiseGuidesDomain::new(context.engine.clone());
                if let Err(error) = guides.prepare(&context).await {
                    guides.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(guides))
            }
            .boxed()
        }),
    });
}

fn lifetime(guides: Arc<RiseGuidesDomain>) -> RiseAppEngineDomainLifetime {
    let activate = guides.clone();
    let did_activate_ownership = guides.clone();
    let socket_connected = guides.clone();
    let shutdown = guides;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let guides = activate.clone();
            async move { guides.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let guides = did_activate_ownership.clone();
            async move { guides.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let guides = socket_connected.clone();
            async move { guides.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let guides = shutdown.clone();
            async move { guides.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
