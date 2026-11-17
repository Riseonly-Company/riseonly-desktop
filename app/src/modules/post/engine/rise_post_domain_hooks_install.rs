use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEnginePostDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::post::engine::rise_post_domain::RisePostDomain;

pub fn install_rise_post_domain_hooks() {
    RiseAppEnginePostDomainHooks::install(RiseAppEnginePostDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let post = RisePostDomain::new(context.engine.clone());
                if let Err(error) = post.prepare(&context).await {
                    post.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(post))
            }
            .boxed()
        }),
    });
}

fn lifetime(post: Arc<RisePostDomain>) -> RiseAppEngineDomainLifetime {
    let activate = post.clone();
    let did_activate_ownership = post.clone();
    let socket_connected = post.clone();
    let shutdown = post;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let post = activate.clone();
            async move { post.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let post = did_activate_ownership.clone();
            async move { post.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let post = socket_connected.clone();
            async move { post.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let post = shutdown.clone();
            async move { post.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
