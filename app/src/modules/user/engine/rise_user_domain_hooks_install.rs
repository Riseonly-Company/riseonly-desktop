use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineUserDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::user::engine::rise_user_domain::RiseUserDomain;

pub(crate) fn install_rise_user_domain_hooks() {
    RiseAppEngineUserDomainHooks::install(RiseAppEngineUserDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let user = RiseUserDomain::new(context.engine.clone());
                if let Err(error) = user.prepare(&context).await {
                    user.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(user))
            }
            .boxed()
        }),
    });
}

fn lifetime(user: Arc<RiseUserDomain>) -> RiseAppEngineDomainLifetime {
    let activate = user.clone();
    let did_activate_ownership = user.clone();
    let socket_connected = user.clone();
    let shutdown = user;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let user = activate.clone();
            async move { user.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let user = did_activate_ownership.clone();
            async move { user.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let user = socket_connected.clone();
            async move { user.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let user = shutdown.clone();
            async move { user.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
