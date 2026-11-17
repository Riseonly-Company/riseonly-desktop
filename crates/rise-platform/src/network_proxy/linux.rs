use std::path::PathBuf;

use futures::future::ready;
use futures::stream::select;
use futures::StreamExt as _;
use zbus::message::Type as MessageType;
use zbus::{MatchRule, MessageStream};

use super::network_proxy::*;
use super::proxy_resolver_portal::*;
use crate::system_activity::freedesktop_activity::{missing_bus_detail, session_bus_socket};
use crate::system_activity::linux::watch_on_bus_thread;
use crate::system_activity::ActivityWatch;

pub(super) struct NativeProxySource;

impl SystemProxySource for NativeProxySource {
    fn watch_settings(&self, sink: ProxySettingsSink) -> Result<ActivityWatch, ProxyUnavailable> {
        let session = session_bus_socket(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(),
            std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).as_deref(),
        );
        if let Some(detail) = missing_bus_detail("session", &session) {
            return Err(ProxyUnavailable::NoService(detail));
        }
        watch_on_bus_thread("rise-proxy-settings", async move {
            if let Err(error) = follow_settings(sink).await {
                tracing::warn!(target: "riseonly", %error, "system proxy changes are not heard");
            }
        })
        .map_err(|error| ProxyUnavailable::SystemRefused(format!("no thread for the settings watch: {error}")))
    }

    fn proxies_for(&self, target: &ProxyTarget) -> Result<Vec<SystemProxyEntry>, ProxyUnavailable> {
        let portal = portal_lookup(&target.lookup_url());
        let environment = ManualProxies::from_environment(|name| std::env::var(name).ok());
        Ok(linux_entries(portal.as_deref(), &environment, target))
    }
}

fn portal_lookup(uri: &str) -> Option<Vec<String>> {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        tracing::debug!(target: "riseonly", "network proxy: off the engine runtime, the ProxyResolver portal is not asked");
        return None;
    };
    let _context = runtime.enter();
    match futures::executor::block_on(tokio::time::timeout(AUTO_CONFIG_TIMEOUT, lookup(uri))) {
        Ok(Ok(uris)) => Some(uris),
        Ok(Err(error)) => {
            tracing::debug!(target: "riseonly", %error, "network proxy: the ProxyResolver portal did not answer");
            None
        }
        Err(_) => {
            tracing::warn!(target: "riseonly", timeout = ?AUTO_CONFIG_TIMEOUT, "network proxy: the ProxyResolver portal timed out");
            None
        }
    }
}

async fn lookup(uri: &str) -> zbus::Result<Vec<String>> {
    let connection = zbus::Connection::session().await?;
    let portal = zbus::Proxy::new(
        &connection,
        PORTAL_BUS_NAME,
        PORTAL_OBJECT_PATH,
        PROXY_RESOLVER_INTERFACE,
    )
    .await?;
    portal.call(LOOKUP_METHOD, &(uri,)).await
}

async fn follow_settings(sink: ProxySettingsSink) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let dconf_rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface(DCONF_WRITER_INTERFACE)?
        .member(DCONF_NOTIFY_SIGNAL)?
        .path_namespace(DCONF_WRITER_PATH_NAMESPACE)?
        .build();
    let kio_rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface(KIO_SCHEDULER_INTERFACE)?
        .member(KIO_REPARSE_SIGNAL)?
        .path(KIO_SCHEDULER_PATH)?
        .build();
    let dconf = MessageStream::for_match_rule(dconf_rule, &connection, None)
        .await?
        .map(|message| message.is_ok_and(|message| dconf_message_touches_proxy(&message)));
    let kio = MessageStream::for_match_rule(kio_rule, &connection, None)
        .await?
        .map(|message| message.is_ok());
    let mut changes = std::pin::pin!(select(dconf, kio).filter(|touches_proxy| ready(*touches_proxy)));
    while changes.next().await.is_some() {
        while let Ok(Some(_)) = tokio::time::timeout(SETTINGS_QUIET, changes.next()).await {}
        sink();
    }
    Err(zbus::Error::Failure(
        "the session bus closed the proxy settings stream".to_owned(),
    ))
}

fn dconf_message_touches_proxy(message: &zbus::Message) -> bool {
    let body = message.body();
    match body.deserialize::<(String, Vec<String>, String)>() {
        Ok((prefix, changes, _tag)) => dconf_commit_touches_proxy(&prefix, &changes),
        Err(error) => {
            tracing::debug!(target: "riseonly", %error, "network proxy: malformed dconf Notify");
            false
        }
    }
}
