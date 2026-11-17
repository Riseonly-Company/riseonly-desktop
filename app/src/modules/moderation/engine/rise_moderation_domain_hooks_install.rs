use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineModerationDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::moderation::engine::rise_moderation_domain::RiseModerationDomain;

pub fn install_rise_moderation_domain_hooks() {
    RiseAppEngineModerationDomainHooks::install(RiseAppEngineModerationDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let moderation = RiseModerationDomain::new(context.engine.clone());
                if let Err(error) = moderation.prepare(&context).await {
                    moderation.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(moderation))
            }
            .boxed()
        }),
    });
}

fn lifetime(moderation: Arc<RiseModerationDomain>) -> RiseAppEngineDomainLifetime {
    let activate = moderation.clone();
    let did_activate_ownership = moderation.clone();
    let socket_connected = moderation.clone();
    let shutdown = moderation;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let moderation = activate.clone();
            async move { moderation.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let moderation = did_activate_ownership.clone();
            async move { moderation.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let moderation = socket_connected.clone();
            async move { moderation.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let moderation = shutdown.clone();
            async move { moderation.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
