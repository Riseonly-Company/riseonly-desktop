use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineCommentDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::comment::engine::rise_comment_domain::RiseCommentDomain;

pub fn install_rise_comment_domain_hooks() {
    RiseAppEngineCommentDomainHooks::install(RiseAppEngineCommentDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let comment = RiseCommentDomain::new(context.engine.clone());
                if let Err(error) = comment.prepare(&context).await {
                    comment.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(comment))
            }
            .boxed()
        }),
    });
}

fn lifetime(comment: Arc<RiseCommentDomain>) -> RiseAppEngineDomainLifetime {
    let activate = comment.clone();
    let did_activate_ownership = comment.clone();
    let socket_connected = comment.clone();
    let shutdown = comment;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let comment = activate.clone();
            async move { comment.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let comment = did_activate_ownership.clone();
            async move { comment.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let comment = socket_connected.clone();
            async move { comment.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let comment = shutdown.clone();
            async move { comment.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
