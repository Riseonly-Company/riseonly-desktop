use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use futures::channel::oneshot;
use futures::future::{select, Either};
use futures::StreamExt as _;
use zbus::zvariant::{OwnedValue, Value};

use super::accessibility::{
    AccessibilitySettings, AccessibilityUnavailable, AccessibilityWatch, ReduceMotionSink,
    ReduceTransparencySink,
};
use super::freedesktop_motion::*;
use crate::system_activity::freedesktop_activity::{missing_bus_detail, session_bus_socket};

const NO_TRANSPARENCY_KEY: &str =
    "the appearance portal and the Linux desktops have no reduce transparency key";

static KNOWN_MOTION: AtomicU8 = AtomicU8::new(0);
static READING: AtomicBool = AtomicBool::new(false);

pub(super) struct NativeAccessibility;

impl AccessibilitySettings for NativeAccessibility {
    fn reduce_transparency(&self) -> Result<bool, AccessibilityUnavailable> {
        Err(AccessibilityUnavailable::Unsupported(NO_TRANSPARENCY_KEY))
    }

    fn watch_reduce_transparency(
        &self,
        sink: ReduceTransparencySink,
    ) -> Result<AccessibilityWatch, AccessibilityUnavailable> {
        drop(sink);
        Err(AccessibilityUnavailable::Unsupported(NO_TRANSPARENCY_KEY))
    }

    fn reduce_motion(&self) -> Result<bool, AccessibilityUnavailable> {
        if let Some(reduce) = known_motion() {
            return Ok(reduce);
        }
        session_bus()?;
        start_one_off_read();
        Err(AccessibilityUnavailable::SourceRefused(NOT_READ_YET.to_owned()))
    }

    fn watch_reduce_motion(
        &self,
        sink: ReduceMotionSink,
    ) -> Result<AccessibilityWatch, AccessibilityUnavailable> {
        session_bus()?;
        let (stop, stopped) = oneshot::channel::<()>();
        std::thread::Builder::new()
            .name("rise-reduce-motion".into())
            .spawn(move || {
                let ended = crate::dbus_block_on::block_on(async move {
                    let watch = std::pin::pin!(watch_motion(&sink));
                    match select(watch, stopped).await {
                        Either::Left((ended, _)) => ended,
                        Either::Right(_) => Ok(()),
                    }
                });
                if let Err(error) = ended {
                    tracing::warn!(target: "riseonly", %error, "reduce motion is no longer observed; the last value stays");
                }
            })
            .map_err(|error| {
                AccessibilityUnavailable::SourceRefused(format!(
                    "no thread for the reduce motion watch: {error}"
                ))
            })?;
        Ok(AccessibilityWatch::new(StopOnDrop(Some(stop))))
    }
}

fn known_motion() -> Option<bool> {
    KnownMotion::from_code(KNOWN_MOTION.load(Ordering::Acquire)).value()
}

fn remember_motion(reduce: bool) {
    KNOWN_MOTION.store(KnownMotion::Known(reduce).code(), Ordering::Release);
}

/// refuses when theres no session bus to ask
fn session_bus() -> Result<(), AccessibilityUnavailable> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let socket = session_bus_socket(address.as_deref(), runtime_dir.as_deref());
    match missing_bus_detail("session", &socket) {
        Some(detail) => Err(AccessibilityUnavailable::SourceRefused(detail)),
        None => Ok(()),
    }
}

fn start_one_off_read() {
    if READING.swap(true, Ordering::AcqRel) {
        return;
    }
    let started = std::thread::Builder::new()
        .name("rise-reduce-motion-read".into())
        .spawn(|| {
            let read = crate::dbus_block_on::block_on(async {
                tokio::time::timeout(READ_TIMEOUT, read_motion_settings()).await
            });
            match read {
                Ok(Ok(settings)) => match settings.resolved() {
                    Some(reduce) => remember_motion(reduce),
                    None => {
                        tracing::debug!(target: "riseonly", reason = NO_MOTION_SETTING, "reduce motion unknown");
                    }
                },
                Ok(Err(error)) => {
                    tracing::debug!(target: "riseonly", %error, "reduce motion: the settings portal refused");
                }
                Err(_) => {
                    tracing::debug!(target: "riseonly", timeout = ?READ_TIMEOUT, "reduce motion: the settings portal did not answer");
                }
            }
            READING.store(false, Ordering::Release);
        });
    if let Err(error) = started {
        READING.store(false, Ordering::Release);
        tracing::warn!(target: "riseonly", %error, "no thread for the reduce motion read");
    }
}

struct StopOnDrop(Option<oneshot::Sender<()>>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            let _ = stop.send(());
        }
    }
}

async fn settings_portal(connection: &zbus::Connection) -> zbus::Result<zbus::Proxy<'static>> {
    zbus::Proxy::new(
        connection,
        PORTAL_BUS_NAME,
        PORTAL_OBJECT_PATH,
        SETTINGS_INTERFACE,
    )
    .await
}

async fn read_motion_settings() -> zbus::Result<MotionSettings> {
    let connection = zbus::Connection::session().await?;
    let portal = settings_portal(&connection).await?;
    Ok(read_all(&portal).await)
}

/// every motion key the portal answers, a key it doesnt have stays unknown
async fn read_all(portal: &zbus::Proxy<'_>) -> MotionSettings {
    let mut settings = MotionSettings::default();
    for source in MotionSource::ALL {
        match read_one(portal, source).await {
            Ok(value) => settings.record(source, &value),
            Err(error) => {
                tracing::debug!(target: "riseonly", namespace = source.namespace(), key = source.key(), %error, "reduce motion: the settings portal has no such key");
            }
        }
    }
    settings
}

async fn read_one(portal: &zbus::Proxy<'_>, source: MotionSource) -> zbus::Result<SettingValue> {
    let arguments = (source.namespace(), source.key());
    match portal
        .call::<_, _, OwnedValue>(READ_ONE_METHOD, &arguments)
        .await
    {
        Ok(value) => Ok(setting_value(&value)),
        Err(read_one_error) => {
            portal
                .call::<_, _, OwnedValue>(READ_METHOD, &arguments)
                .await
                .map(|value| setting_value(&value))
                .map_err(|_| read_one_error)
        }
    }
}

fn setting_value(value: &Value<'_>) -> SettingValue {
    match value {
        Value::Bool(flag) => SettingValue::Bool(*flag),
        Value::U8(number) => SettingValue::Unsigned(u64::from(*number)),
        Value::U16(number) => SettingValue::Unsigned(u64::from(*number)),
        Value::U32(number) => SettingValue::Unsigned(u64::from(*number)),
        Value::U64(number) => SettingValue::Unsigned(*number),
        Value::I16(number) => SettingValue::Signed(i64::from(*number)),
        Value::I32(number) => SettingValue::Signed(i64::from(*number)),
        Value::I64(number) => SettingValue::Signed(*number),
        Value::F64(number) => SettingValue::Double(*number),
        Value::Str(text) => SettingValue::Text(text.as_str().to_owned()),
        Value::Value(inner) => setting_value(inner),
        _ => SettingValue::Other,
    }
}

async fn watch_motion(sink: &ReduceMotionSink) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let portal = settings_portal(&connection).await?;
    let mut changes = std::pin::pin!(portal.receive_signal(SETTING_CHANGED_SIGNAL).await?);
    let mut settings = read_all(&portal).await;
    if let Some(reduce) = settings.resolved() {
        deliver(sink, reduce);
    }
    while let Some(message) = changes.next().await {
        let body = message.body();
        match body.deserialize::<(String, String, OwnedValue)>() {
            Ok((namespace, key, value)) => {
                if let Some(reduce) =
                    settings.apply_change(&namespace, &key, &setting_value(&value))
                {
                    deliver(sink, reduce);
                }
            }
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "reduce motion: malformed SettingChanged");
            }
        }
    }
    Err(zbus::Error::Failure(
        "the session bus closed the SettingChanged stream".to_owned(),
    ))
}

fn deliver(sink: &ReduceMotionSink, reduce: bool) {
    remember_motion(reduce);
    let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sink(reduce)));
    if delivered.is_err() {
        tracing::error!(target: "riseonly", reduce, "reduce motion handler panicked");
    }
}
