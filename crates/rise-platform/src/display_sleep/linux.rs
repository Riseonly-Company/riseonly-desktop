use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::channel::oneshot;
use zbus::zvariant::{OwnedObjectPath, Value};

use super::display_sleep::*;
use super::freedesktop_inhibit::*;
use crate::system_activity::freedesktop_activity::{missing_bus_detail, session_bus_socket};

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

pub(super) struct NativeDisplaySleep;

impl DisplaySleepInhibitor for NativeDisplaySleep {
    fn inhibit(&self, reason: &str) -> Result<SystemInhibition, DisplaySleepUnavailable> {
        let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let socket = session_bus_socket(address.as_deref(), runtime_dir.as_deref());
        if let Some(detail) = missing_bus_detail("session", &socket) {
            return Err(DisplaySleepUnavailable::NoService(detail));
        }
        let (release, released) = oneshot::channel::<()>();
        let reason = inhibit_reason(reason).to_owned();
        std::thread::Builder::new()
            .name("rise-display-sleep".to_owned())
            .spawn(move || zbus::block_on(hold(reason, released)))
            .map_err(|error| DisplaySleepUnavailable::NoService(format!("no thread for the hold: {error}")))?;
        Ok(SystemInhibition::new(release))
    }
}

/// what one route holds until the guard drops
enum Held {
    Portal(OwnedObjectPath),
    ScreenSaver(u32),
}

/// holds until released fires or its sender drops, then gives the hold back
async fn hold(reason: String, released: oneshot::Receiver<()>) {
    let connection = match zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(target: "riseonly", %error, "display may sleep during the call: no session bus");
            return;
        }
    };
    let held = match acquire_first(&connection, &reason).await {
        Ok(held) => held,
        Err(failures) => {
            let failure = DisplaySleepUnavailable::NoService(no_service_detail(&failures));
            tracing::warn!(target: "riseonly", %failure, "display may sleep during the call");
            return;
        }
    };
    let _ = released.await;
    if let Err(error) = release(&connection, held).await {
        tracing::debug!(target: "riseonly", %error, "display sleep release failed, the connection closes it");
    }
}

async fn acquire_first(
    connection: &zbus::Connection,
    reason: &str,
) -> Result<Held, Vec<(InhibitRoute, String)>> {
    let mut failures = Vec::new();
    for route in INHIBIT_ROUTES {
        let outcome = match route {
            InhibitRoute::Portal => inhibit_portal(connection, reason).await.map(Held::Portal),
            InhibitRoute::ScreenSaver => {
                inhibit_screen_saver(connection, reason).await.map(Held::ScreenSaver)
            }
        };
        match outcome {
            Ok(held) => return Ok(held),
            Err(error) => failures.push((route, error.to_string())),
        }
    }
    Err(failures)
}

async fn inhibit_portal(
    connection: &zbus::Connection,
    reason: &str,
) -> zbus::Result<OwnedObjectPath> {
    let portal =
        zbus::Proxy::new(connection, PORTAL_BUS_NAME, PORTAL_OBJECT_PATH, INHIBIT_INTERFACE)
            .await?;
    let token = inhibit_token(NEXT_TOKEN.fetch_add(1, Ordering::Relaxed));
    let options: HashMap<&str, Value<'_>> = HashMap::from([
        (HANDLE_TOKEN_OPTION, Value::from(token.as_str())),
        (REASON_OPTION, Value::from(reason)),
    ]);
    portal
        .call(INHIBIT_METHOD, &(NO_PARENT_WINDOW, IDLE_FLAG, options))
        .await
}

async fn inhibit_screen_saver(connection: &zbus::Connection, reason: &str) -> zbus::Result<u32> {
    let screen_saver = zbus::Proxy::new(
        connection,
        SCREEN_SAVER_BUS_NAME,
        SCREEN_SAVER_OBJECT_PATH,
        SCREEN_SAVER_INTERFACE,
    )
    .await?;
    screen_saver
        .call(
            SCREEN_SAVER_INHIBIT_METHOD,
            &(SCREEN_SAVER_APPLICATION_NAME, reason),
        )
        .await
}

async fn release(connection: &zbus::Connection, held: Held) -> zbus::Result<()> {
    match held {
        Held::Portal(handle) => {
            zbus::Proxy::new(connection, PORTAL_BUS_NAME, handle, REQUEST_INTERFACE)
                .await?
                .call::<_, _, ()>(CLOSE_METHOD, &())
                .await
        }
        Held::ScreenSaver(cookie) => {
            zbus::Proxy::new(
                connection,
                SCREEN_SAVER_BUS_NAME,
                SCREEN_SAVER_OBJECT_PATH,
                SCREEN_SAVER_INTERFACE,
            )
            .await?
            .call::<_, _, ()>(SCREEN_SAVER_UNINHIBIT_METHOD, &(cookie,))
            .await
        }
    }
}
