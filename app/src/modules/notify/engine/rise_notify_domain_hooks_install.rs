use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineNotifyDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::notify::engine::rise_notify_domain::RiseNotifyDomain;

pub fn install_rise_notify_domain_hooks() {
    RiseAppEngineNotifyDomainHooks::install(RiseAppEngineNotifyDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let notify = RiseNotifyDomain::new(context.engine.clone());
                if let Err(error) = notify.prepare(&context).await {
                    notify.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(notify))
            }
            .boxed()
        }),
    });
}

fn lifetime(notify: Arc<RiseNotifyDomain>) -> RiseAppEngineDomainLifetime {
    let activate = notify.clone();
    let did_activate_ownership = notify.clone();
    let socket_connected = notify.clone();
    let application_did_become_active = notify.clone();
    let shutdown = notify;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let notify = activate.clone();
            async move { notify.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let notify = did_activate_ownership.clone();
            async move { notify.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let notify = socket_connected.clone();
            async move { notify.socket_connected().await }.boxed()
        }),
        application_did_become_active: Arc::new(move || {
            let notify = application_did_become_active.clone();
            async move { notify.refresh_unread_badge().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let notify = shutdown.clone();
            async move { notify.shutdown().await }.boxed()
        }),
    })
}
