use std::sync::Arc;

use futures::FutureExt;

use crate::core::rise_engine_bridge::rise_app_engine_domain_hooks::RiseAppEngineMusicDomainHooks;
use crate::core::rise_engine_bridge::rise_app_engine_session::{
    RiseAppEngineDomainLifetime, RiseAppEngineDomainLifetimeArgs,
};
use crate::modules::music::engine::rise_music_domain::RiseMusicDomain;

pub(crate) fn install_rise_music_domain_hooks() {
    RiseAppEngineMusicDomainHooks::install(RiseAppEngineMusicDomainHooks {
        prepare: Box::new(|context| {
            async move {
                let music = RiseMusicDomain::new(context.engine.clone());
                if let Err(error) = music.prepare(&context).await {
                    music.shutdown().await;
                    return Err(error);
                }
                Ok(lifetime(music))
            }
            .boxed()
        }),
    });
}

fn lifetime(music: Arc<RiseMusicDomain>) -> RiseAppEngineDomainLifetime {
    let activate = music.clone();
    let did_activate_ownership = music.clone();
    let socket_connected = music.clone();
    let shutdown = music;
    RiseAppEngineDomainLifetime::new(RiseAppEngineDomainLifetimeArgs {
        activate: Arc::new(move || {
            let music = activate.clone();
            async move { music.activate().await }.boxed()
        }),
        did_activate_ownership: Arc::new(move || {
            let music = did_activate_ownership.clone();
            async move { music.did_activate_ownership().await }.boxed()
        }),
        socket_connected: Arc::new(move || {
            let music = socket_connected.clone();
            async move { music.socket_connected().await }.boxed()
        }),
        shutdown: Arc::new(move || {
            let music = shutdown.clone();
            async move { music.shutdown().await }.boxed()
        }),
        ..RiseAppEngineDomainLifetimeArgs::default()
    })
}
