use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::autostart::*;
use super::background_portal::*;
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::system_activity::freedesktop_activity::{missing_bus_detail, session_bus_socket};

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn directory() -> Result<PathBuf, AutostartError> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);

    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(unavailable)?;

    Ok(autostart_directory(config_home.as_deref(), &home))
}

fn unavailable() -> AutostartError {
    AutostartError::Unavailable(AutostartMechanism::for_host(HostOs::current()))
}

fn sandbox() -> Sandbox {
    let snap_name = std::env::var_os(SNAP_NAME_VARIABLE);
    Sandbox::detect(Path::new(FLATPAK_INFO_PATH).exists(), snap_name.as_deref())
}

/// some detail when theres no session bus for the portal
fn missing_session_bus() -> Option<String> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    missing_bus_detail("session", &session_bus_socket(address.as_deref(), runtime_dir.as_deref()))
}

pub(super) fn launched_at_login() -> bool {
    launched_at_login_by_argument()
}

pub(super) fn availability(_config: &AutostartConfig) -> PlatformSupport {
    if directory().is_err() {
        return PlatformSupport::Unsupported;
    }
    if sandbox().needs_portal() && missing_session_bus().is_some() {
        return PlatformSupport::Unsupported;
    }
    PlatformSupport::Performed
}

pub(super) fn state(config: &AutostartConfig) -> Result<AutostartState, AutostartError> {
    read_desktop_entry_state(&directory()?, config)
}

pub(super) fn set_enabled(
    config: &AutostartConfig,
    enabled: bool,
) -> Result<PlatformSupport, AutostartError> {
    let directory = directory()?;
    if !sandbox().needs_portal() {
        return apply_desktop_entry(&directory, config, enabled);
    }

    let previous = read_desktop_entry_state(&directory, config)?;
    if transition(previous, enabled) == AutostartTransition::AlreadySettled {
        return Ok(PlatformSupport::Performed);
    }
    if let Some(detail) = missing_session_bus() {
        tracing::warn!(target: "riseonly", %detail, "launch at login: no session bus for the background portal");
        return Err(unavailable());
    }

    apply_desktop_entry(&directory, config, enabled)?;
    let record = directory.clone();
    let request_config = config.clone();
    let started = std::thread::Builder::new().name("rise-autostart-portal".into()).spawn(move || {
        let answer = zbus::block_on(request_background(&request_config, enabled));
        let outcome = match answer {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "launch at login: the background portal did not answer");
                BackgroundOutcome::Refused
            }
        };
        if let Some(reason) = outcome.refusal_reason() {
            tracing::info!(target: "riseonly", reason, enabled, "launch at login: request not applied");
            roll_back(&directory, &request_config, previous);
        }
    });

    if let Err(error) = started {
        roll_back(&record, config, previous);
        return Err(error.into());
    }
    Ok(PlatformSupport::Performed)
}

/// puts the sandbox record back to what it was before the request
fn roll_back(directory: &Path, config: &AutostartConfig, previous: AutostartState) {
    if let Err(error) = apply_desktop_entry(directory, config, previous.is_enabled()) {
        tracing::warn!(target: "riseonly", %error, "launch at login: the sandbox record was not rolled back");
    }
}

async fn request_background(
    config: &AutostartConfig,
    enabled: bool,
) -> zbus::Result<BackgroundOutcome> {
    let connection = zbus::Connection::session().await?;
    let background =
        zbus::Proxy::new(&connection, PORTAL_BUS_NAME, PORTAL_OBJECT_PATH, BACKGROUND_INTERFACE)
            .await?;
    let unique_name =
        connection.unique_name().map(|name| name.as_str().to_owned()).unwrap_or_default();
    let token = request_token(NEXT_TOKEN.fetch_add(1, Ordering::Relaxed));
    let expected = request_path(&unique_name, &token);
    let mut responses =
        zbus::Proxy::new(&connection, PORTAL_BUS_NAME, expected.clone(), REQUEST_INTERFACE)
            .await?
            .receive_signal(RESPONSE_SIGNAL)
            .await?;

    let options: HashMap<&str, Value<'_>> = HashMap::from([
        (HANDLE_TOKEN_OPTION, Value::from(token.as_str())),
        (AUTOSTART_OPTION, Value::from(enabled)),
        (COMMANDLINE_OPTION, Value::from(background_commandline(config))),
        (DBUS_ACTIVATABLE_OPTION, Value::from(false)),
    ]);
    let handle: OwnedObjectPath =
        background.call(REQUEST_BACKGROUND_METHOD, &("", options)).await?;
    if handle.as_str() != expected {
        responses = zbus::Proxy::new(&connection, PORTAL_BUS_NAME, handle, REQUEST_INTERFACE)
            .await?
            .receive_signal(RESPONSE_SIGNAL)
            .await?;
    }

    let message = responses.next().await.ok_or_else(|| {
        zbus::Error::Failure("the background portal request closed without a response".to_owned())
    })?;
    let (code, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize()?;
    let autostart = results.get(AUTOSTART_RESULT).and_then(|value| bool::try_from(value).ok());
    Ok(background_outcome(code, autostart, enabled))
}
