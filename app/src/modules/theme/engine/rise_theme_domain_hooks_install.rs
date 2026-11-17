use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineThemeDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::theme::engine::rise_theme_domain::RiseThemeDomain;

pub(crate) fn install_rise_theme_domain_hooks() {
    RiseAppEngineThemeDomainHooks::install(RiseAppEngineThemeDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let theme = RiseThemeDomain::new(context.engine.clone());
                if let Err(error) = theme.prepare(&context).await {
                    theme.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(theme))
            }
            .boxed()
        }),
    });
}

fn lifetime(theme: Arc<RiseThemeDomain>) -> RiseAppEngineDomainLifetime {
    let activate = theme.clone();
    let did_activate_ownership = theme.clone();
    let socket_connected = theme.clone();
    let shutdown = theme;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let theme = activate.clone();
            async move { theme.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let theme = did_activate_ownership.clone();
            async move { theme.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let theme = socket_connected.clone();
            async move { theme.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let theme = shutdown.clone();
            async move { theme.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
