use std::path::{Path, PathBuf};

use futures::stream::select;
use futures::StreamExt as _;

use crate::system_activity::freedesktop_activity::*;
use crate::system_activity::*;

pub(crate) struct NativeSystemActivity;

impl SystemActivitySource for NativeSystemActivity {
    fn watch_sleep(&self, sink: EventSink) -> Result<ActivityWatch, SystemActivityUnavailable> {
        let what = SystemActivityKind::SleepAndWake;
        let system = system_bus_socket(env_value("DBUS_SYSTEM_BUS_ADDRESS").as_deref());
        if let Some(detail) = missing_bus_detail("system", &system) {
            return Err(SystemActivityUnavailable::SystemRefused { what, detail });
        }
        spawn_on_engine(what, async move {
            if let Err(error) = watch_prepare_for_sleep(&sink).await {
                tracing::warn!(%error, "system activity: logind PrepareForSleep is not observed");
            }
        })
    }

    fn watch_network_path(
        &self,
        sink: EventSink,
    ) -> Result<ActivityWatch, SystemActivityUnavailable> {
        let what = SystemActivityKind::NetworkPath;
        let sandboxed = Path::new(FLATPAK_INFO_PATH).exists();
        let system = system_bus_socket(env_value("DBUS_SYSTEM_BUS_ADDRESS").as_deref());
        let session = session_bus_socket(
            env_value("DBUS_SESSION_BUS_ADDRESS").as_deref(),
            env_value("XDG_RUNTIME_DIR").map(PathBuf::from).as_deref(),
        );
        let routes: Vec<NetworkPathRoute> = network_path_routes(sandboxed)
            .iter()
            .copied()
            .filter(|route| match route {
                NetworkPathRoute::NetworkManager => missing_bus_detail("system", &system).is_none(),
                NetworkPathRoute::Portal => missing_bus_detail("session", &session).is_none(),
            })
            .collect();
        if routes.is_empty() {
            let detail = missing_bus_detail("session", &session)
                .unwrap_or_else(|| "no bus answers the network path".to_owned());
            return Err(SystemActivityUnavailable::SystemRefused { what, detail });
        }
        spawn_on_engine(what, async move {
            for route in routes {
                let outcome = match route {
                    NetworkPathRoute::NetworkManager => watch_network_manager(&sink).await,
                    NetworkPathRoute::Portal => watch_network_monitor_portal(&sink).await,
                };
                match outcome {
                    Ok(()) => return,
                    Err(error) => {
                        tracing::info!(%error, ?route, "system activity: network path route failed");
                    }
                }
            }
            tracing::warn!("system activity: no service reports the network path");
        })
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// runs the service until the watch drops
fn spawn_on_engine(
    what: SystemActivityKind,
    service: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<ActivityWatch, SystemActivityUnavailable> {
    watch_on_bus_thread("rise-system-activity", service).map_err(|error| {
        SystemActivityUnavailable::SystemRefused {
            what,
            detail: format!("no thread for the bus service: {error}"),
        }
    })
}

/// runs a bus service on its own thread until the watch drops, whoever asks
pub(crate) fn watch_on_bus_thread(
    name: &str,
    service: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<ActivityWatch> {
    let (stop, stopped) = futures::channel::oneshot::channel::<()>();
    std::thread::Builder::new().name(name.to_owned()).spawn(move || {
        zbus::block_on(async move {
            let service = std::pin::pin!(service);
            let _ = futures::future::select(service, stopped).await;
        });
    })?;
    Ok(ActivityWatch::new(stop))
}

async fn watch_prepare_for_sleep(sink: &EventSink) -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let manager = zbus::Proxy::new(
        &connection,
        LOGIN1_SERVICE,
        LOGIN1_PATH,
        LOGIN1_MANAGER_INTERFACE,
    )
    .await?;
    let mut signals = std::pin::pin!(manager.receive_signal(PREPARE_FOR_SLEEP_SIGNAL).await?);
    while let Some(message) = signals.next().await {
        let body = message.body();
        match body.deserialize::<bool>() {
            Ok(start) => sink(prepare_for_sleep_event(start)),
            Err(error) => tracing::warn!(%error, "system activity: malformed PrepareForSleep"),
        }
    }
    Err(zbus::Error::Failure(
        "the system bus closed the PrepareForSleep stream".to_owned(),
    ))
}

async fn watch_network_manager(sink: &EventSink) -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let manager = zbus::Proxy::new(
        &connection,
        NETWORK_MANAGER_SERVICE,
        NETWORK_MANAGER_PATH,
        NETWORK_MANAGER_INTERFACE,
    )
    .await?;
    let read = || async {
        let connectivity = manager
            .get_property::<u32>(NETWORK_MANAGER_CONNECTIVITY)
            .await?;
        let state = manager.get_property::<u32>(NETWORK_MANAGER_STATE).await?;
        Ok::<_, zbus::Error>(network_manager_satisfied(connectivity, state))
    };
    let mut reporter = NetworkPathReporter::default();
    if let Some(event) = reporter.report(read().await?) {
        sink(event);
    }
    let connectivity = manager
        .receive_property_changed::<u32>(NETWORK_MANAGER_CONNECTIVITY)
        .await;
    let state = manager
        .receive_property_changed::<u32>(NETWORK_MANAGER_STATE)
        .await;
    let mut changes = std::pin::pin!(select(connectivity, state));
    while changes.next().await.is_some() {
        match read().await {
            Ok(satisfied) => {
                if let Some(event) = reporter.report(satisfied) {
                    sink(event);
                }
            }
            Err(error) => tracing::warn!(%error, "system activity: NetworkManager read failed"),
        }
    }
    Ok(())
}

async fn watch_network_monitor_portal(sink: &EventSink) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let monitor = zbus::Proxy::new(
        &connection,
        PORTAL_SERVICE,
        PORTAL_PATH,
        NETWORK_MONITOR_INTERFACE,
    )
    .await?;
    let mut reporter = NetworkPathReporter::default();
    let mut changes = std::pin::pin!(
        monitor
            .receive_signal(NETWORK_MONITOR_CHANGED_SIGNAL)
            .await?
    );
    let available = monitor
        .call::<_, _, bool>(NETWORK_MONITOR_GET_AVAILABLE, &())
        .await?;
    if let Some(event) = reporter.report(available) {
        sink(event);
    }
    while changes.next().await.is_some() {
        match monitor
            .call::<_, _, bool>(NETWORK_MONITOR_GET_AVAILABLE, &())
            .await
        {
            Ok(available) => {
                if let Some(event) = reporter.report(available) {
                    sink(event);
                }
            }
            Err(error) => tracing::warn!(%error, "system activity: portal GetAvailable failed"),
        }
    }
    Ok(())
}
