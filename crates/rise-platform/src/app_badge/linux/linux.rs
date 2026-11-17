use std::collections::HashMap;
use std::path::PathBuf;

use futures::StreamExt;
use futures::channel::mpsc;
use parking_lot::Mutex;
use zbus::zvariant::Value;

use crate::app_badge::app_badge_hooks::app_badge_hooks;
use crate::app_badge::launcher_entry::*;
use crate::gpui_shim::PlatformSupport;
use crate::system_activity::freedesktop_activity::{missing_bus_detail, session_bus_socket};

static UPDATES: Mutex<Option<mpsc::UnboundedSender<u32>>> = Mutex::new(None);

pub(crate) type UpdateBody = (String, HashMap<&'static str, Value<'static>>);

pub(crate) fn update_body(app_uri: &str, count: u32) -> UpdateBody {
    let (number, visible) = launcher_count(count);
    let properties = HashMap::from([
        (LAUNCHER_COUNT_PROPERTY, Value::from(number)),
        (LAUNCHER_COUNT_VISIBLE_PROPERTY, Value::from(visible)),
    ]);
    (app_uri.to_owned(), properties)
}

pub(crate) fn apply_count(count: u32) -> PlatformSupport {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let socket = session_bus_socket(address.as_deref(), runtime_dir.as_deref());
    if let Some(detail) = missing_bus_detail("session", &socket) {
        tracing::debug!(target: "riseonly", %detail, "app badge: the number stays in the tray");
        return PlatformSupport::Unsupported;
    }
    let Some(hooks) = app_badge_hooks() else {
        tracing::debug!(target: "riseonly", "app badge: no hooks installed, the number stays in the tray");
        return PlatformSupport::Unsupported;
    };
    let Some(bundle_identifier) = (hooks.bundle_identifier)() else {
        tracing::debug!(target: "riseonly", "app badge: no identity for this environment, the number stays in the tray");
        return PlatformSupport::Unsupported;
    };

    let mut updates = UPDATES.lock();
    if let Some(sender) = updates.as_ref()
        && sender.unbounded_send(count).is_ok()
    {
        return PlatformSupport::Performed;
    }
    let (sender, receiver) = mpsc::unbounded();
    let _ = sender.unbounded_send(count);
    (hooks.spawn)(Box::pin(announce(launcher_app_uri(bundle_identifier), receiver)));
    *updates = Some(sender);
    PlatformSupport::Performed
}

/// sends each number, only the newest of a burst
async fn announce(app_uri: String, mut updates: mpsc::UnboundedReceiver<u32>) {
    let connection = match zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(target: "riseonly", %error, "app badge: no session bus, the number stays in the tray");
            return;
        }
    };
    while let Some(mut count) = updates.next().await {
        while let Ok(newer) = updates.try_recv() {
            count = newer;
        }
        let sent = connection
            .emit_signal(
                None::<&str>,
                LAUNCHER_ENTRY_PATH,
                LAUNCHER_ENTRY_INTERFACE,
                LAUNCHER_ENTRY_SIGNAL,
                &update_body(&app_uri, count),
            )
            .await;
        if let Err(error) = sent {
            tracing::debug!(target: "riseonly", %error, "app badge: the launcher signal was not sent");
        }
    }
}
