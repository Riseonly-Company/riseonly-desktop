use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt as _;
use parking_lot::Mutex;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::camera_portal::*;
use super::capture_permission::*;

static CAMERA_ANSWER: Mutex<Option<bool>> = Mutex::new(None);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

pub(super) struct NativeCaptureAccess;

fn sandboxed() -> bool {
    Path::new(FLATPAK_INFO_PATH).exists()
}

fn current(kind: CaptureKind) -> CaptureAccess {
    linux_access(sandboxed(), kind, *CAMERA_ANSWER.lock())
}

impl CaptureAccessSource for NativeCaptureAccess {
    fn status(&self, kind: CaptureKind) -> Result<CaptureAccess, CaptureUnavailable> {
        Ok(current(kind))
    }

    fn request(&self, kind: CaptureKind, done: AccessAnswer) -> Result<(), CaptureUnavailable> {
        match current(kind) {
            CaptureAccess::Granted => done(true),
            CaptureAccess::Denied | CaptureAccess::Restricted => done(false),
            CaptureAccess::NotDetermined => ask_camera_portal(done),
        }
        Ok(())
    }
}

fn ask_camera_portal(done: AccessAnswer) {
    let unanswered = Arc::new(Mutex::new(Some(done)));
    let answer = Arc::clone(&unanswered);
    let spawned = std::thread::Builder::new().name("rise-camera-portal".to_owned()).spawn(move || {
        let granted = crate::dbus_block_on::block_on(camera_portal_answer());
        let done = answer.lock().take();
        if let Some(done) = done {
            done(granted);
        }
    });
    if let Err(error) = spawned {
        tracing::warn!(%error, "capture access: no thread for the camera portal");
        let done = unanswered.lock().take();
        if let Some(done) = done {
            done(false);
        }
    }
}

async fn camera_portal_answer() -> bool {
    match access_camera().await {
        Ok(Some(response)) => {
            let granted = response.is_granted();
            *CAMERA_ANSWER.lock() = Some(granted);
            granted
        }
        Ok(None) => {
            tracing::debug!("capture access: the camera portal sees no camera");
            false
        }
        Err(error) => {
            tracing::warn!(%error, "capture access: the camera portal did not answer");
            false
        }
    }
}

async fn access_camera() -> zbus::Result<Option<PortalResponse>> {
    let connection = zbus::Connection::session().await?;
    let camera =
        zbus::Proxy::new(&connection, PORTAL_BUS_NAME, PORTAL_OBJECT_PATH, CAMERA_INTERFACE)
            .await?;
    if let Ok(false) = camera.get_property::<bool>(IS_CAMERA_PRESENT_PROPERTY).await {
        return Ok(None);
    }
    let unique_name =
        connection.unique_name().map(|name| name.as_str().to_owned()).unwrap_or_default();
    let token = request_token(NEXT_TOKEN.fetch_add(1, Ordering::Relaxed));
    let expected = request_path(&unique_name, &token);
    let mut responses = zbus::Proxy::new(
        &connection,
        PORTAL_BUS_NAME,
        expected.clone(),
        REQUEST_INTERFACE,
    )
    .await?
    .receive_signal(RESPONSE_SIGNAL)
    .await?;
    let options: HashMap<&str, Value<'_>> =
        HashMap::from([(HANDLE_TOKEN_OPTION, Value::from(token.as_str()))]);
    let handle: OwnedObjectPath = camera.call(ACCESS_CAMERA_METHOD, &(options,)).await?;
    if handle.as_str() != expected {
        responses = zbus::Proxy::new(&connection, PORTAL_BUS_NAME, handle, REQUEST_INTERFACE)
            .await?
            .receive_signal(RESPONSE_SIGNAL)
            .await?;
    }
    let message = responses.next().await.ok_or_else(|| {
        zbus::Error::Failure("the camera portal request closed without a response".to_owned())
    })?;
    let (code, _results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize()?;
    Ok(Some(PortalResponse::from_code(code)))
}
