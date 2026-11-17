use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use futures::channel::{mpsc, oneshot};
use futures::future::{BoxFuture, FutureExt as _};
use futures::stream::FuturesUnordered;
use futures::StreamExt as _;
use parking_lot::Mutex;
use rise_core::foundation::foundation_date::RiseClock;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};
use zbus::DBusError as _;

use super::location::*;
use super::xdg_location::*;
use crate::host_os::HostOs;

pub(super) struct NativeLocation;

enum Command {
    Authorize,
    StartUpdates(UpdateRequest),
    StopUpdates,
    RequestFix,
}

/// queue between the main thread and the driver, one per process
struct Route {
    commands: mpsc::UnboundedSender<Command>,
    pending_driver: Mutex<Option<mpsc::UnboundedReceiver<Command>>>,
}

fn route() -> &'static Route {
    static ROUTE: OnceLock<Route> = OnceLock::new();
    ROUTE.get_or_init(|| {
        let (commands, driver) = mpsc::unbounded();
        Route { commands, pending_driver: Mutex::new(Some(driver)) }
    })
}

static SINK: OnceLock<LocationSink> = OnceLock::new();
static LEVEL: OnceLock<Mutex<LocationAuthorization>> = OnceLock::new();
static LAST_FIX: Mutex<Option<LocationFix>> = Mutex::new(None);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn level() -> &'static Mutex<LocationAuthorization> {
    LEVEL.get_or_init(|| Mutex::new(initial_level(Path::new(FLATPAK_INFO_PATH).exists())))
}

fn emit(event: LocationEvent) {
    if let Some(sink) = SINK.get() {
        sink(event);
    }
}

/// stores the level and sends the authorization event if it changed or always_report is set
fn settle(next: LocationAuthorization, always_report: bool) {
    let changed = {
        let mut current = level().lock();
        let changed = *current != next;
        *current = next;
        changed
    };
    if changed || always_report {
        emit(LocationEvent::Authorization(permission_of(next)));
    }
}

impl LocationSource for NativeLocation {
    fn take_driver(&self) -> Option<LocationDriver> {
        let commands = route().pending_driver.lock().take()?;
        Some(Box::pin(drive(commands)))
    }

    fn permission(&self) -> Result<LocationPermission, LocationUnavailable> {
        Ok(permission_of(*level().lock()))
    }

    fn last_fix(&self) -> Result<Option<LocationFix>, LocationUnavailable> {
        Ok(*LAST_FIX.lock())
    }

    fn service_available(&self, service: LocationService) -> Result<(), LocationUnavailable> {
        Err(LocationUnavailable::NoSuchService(HostOs::Linux, service))
    }

    fn install(&self, sink: LocationSink) -> Result<(), LocationUnavailable> {
        let _ = SINK.set(sink);
        Ok(())
    }

    fn perform(&self, command: LocationCommand) -> Result<(), LocationUnavailable> {
        let command = match command {
            LocationCommand::RequestAuthorization(_) => Command::Authorize,
            LocationCommand::StartUpdates(request) => Command::StartUpdates(request),
            LocationCommand::StopUpdates => Command::StopUpdates,
            LocationCommand::RequestFix => Command::RequestFix,
            other => {
                let service = other.service().unwrap_or(LocationService::SignificantChanges);
                return Err(LocationUnavailable::NoSuchService(HostOs::Linux, service));
            }
        };
        if route().commands.unbounded_send(command).is_err() {
            tracing::warn!("location: the portal driver is not running");
        }
        Ok(())
    }
}

/// runs for the whole process, each command is its own future so a waiting dialog never blocks a stop
/// standard updates run one session at a time, nothing is spawned
async fn drive(mut commands: mpsc::UnboundedReceiver<Command>) {
    let mut tasks: FuturesUnordered<BoxFuture<'static, ()>> = FuturesUnordered::new();
    let mut updates: Option<oneshot::Sender<()>> = None;
    loop {
        tokio::select! {
            command = commands.next() => {
                let Some(command) = command else { break };
                match command {
                    Command::Authorize => tasks.push(authorize().boxed()),
                    Command::RequestFix => tasks.push(request_fix().boxed()),
                    Command::StartUpdates(request) => {
                        drop(updates.take());
                        let (stop, stopped) = oneshot::channel();
                        updates = Some(stop);
                        tasks.push(run_updates(request, stopped).boxed());
                    }
                    Command::StopUpdates => drop(updates.take()),
                }
            }
            Some(()) = tasks.next(), if !tasks.is_empty() => {}
        }
    }
    while tasks.next().await.is_some() {}
}

/// one portal session after start answered
/// updates are subscribed before start so an early fix is not lost
struct OpenSession {
    connection: zbus::Connection,
    handle: OwnedObjectPath,
    updates: zbus::proxy::SignalStream<'static>,
    response: StartResponse,
}

impl OpenSession {
    /// next fix of this session, none when the bus stream ended
    async fn next_fix(&mut self) -> Option<LocationFix> {
        while let Some(message) = self.updates.next().await {
            let Ok((session, values)) = message.body().deserialize::<(OwnedObjectPath, HashMap<String, OwnedValue>)>()
            else {
                continue;
            };
            if session != self.handle {
                continue;
            }
            let now = RiseClock::now().time_interval_since_1970();
            let fix = portal_location(&values).to_fix(now);
            *LAST_FIX.lock() = Some(fix);
            return Some(fix);
        }
        None
    }

    async fn close(self) {
        let closed = async {
            let session =
                zbus::Proxy::new(&self.connection, PORTAL_BUS_NAME, self.handle.clone(), SESSION_INTERFACE).await?;
            session.call::<_, _, ()>(CLOSE_METHOD, &()).await
        };
        if let Err(error) = closed.await {
            tracing::debug!(%error, "location: the portal session did not close");
        }
    }
}

fn read_f64(values: &HashMap<String, OwnedValue>, key: &str) -> Option<f64> {
    values.get(key).and_then(|value| f64::try_from(value).ok())
}

fn read_timestamp(values: &HashMap<String, OwnedValue>) -> Option<(u64, u64)> {
    let structure = <&Structure<'_>>::try_from(values.get(TIMESTAMP_KEY)?).ok()?;
    match structure.fields() {
        [Value::U64(seconds), Value::U64(microseconds)] => Some((*seconds, *microseconds)),
        _ => None,
    }
}

fn portal_location(values: &HashMap<String, OwnedValue>) -> PortalLocation {
    PortalLocation {
        latitude: read_f64(values, LATITUDE_KEY),
        longitude: read_f64(values, LONGITUDE_KEY),
        accuracy: read_f64(values, ACCURACY_KEY),
        altitude: read_f64(values, ALTITUDE_KEY),
        speed: read_f64(values, SPEED_KEY),
        heading: read_f64(values, HEADING_KEY),
        timestamp: read_timestamp(values),
    }
}

async fn open_session(options: SessionOptions) -> zbus::Result<OpenSession> {
    let connection = zbus::Connection::session().await?;
    let location = zbus::Proxy::new(&connection, PORTAL_BUS_NAME, PORTAL_OBJECT_PATH, LOCATION_INTERFACE).await?;
    let serial = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    let session_options: HashMap<&str, Value<'_>> = HashMap::from([
        (SESSION_HANDLE_TOKEN_OPTION, Value::from(session_token(serial))),
        (DISTANCE_THRESHOLD_OPTION, Value::from(options.distance_threshold)),
        (TIME_THRESHOLD_OPTION, Value::from(options.time_threshold)),
        (ACCURACY_OPTION, Value::from(options.accuracy.code())),
    ]);
    let handle: OwnedObjectPath = location.call(CREATE_SESSION_METHOD, &(session_options,)).await?;
    let updates = location.receive_signal(LOCATION_UPDATED_SIGNAL).await?;

    let unique_name = connection.unique_name().map(|name| name.as_str().to_owned()).unwrap_or_default();
    let token = request_token(serial);
    let expected = request_path(&unique_name, &token);
    let mut responses = zbus::Proxy::new(&connection, PORTAL_BUS_NAME, expected.clone(), REQUEST_INTERFACE)
        .await?
        .receive_signal(RESPONSE_SIGNAL)
        .await?;
    let start_options: HashMap<&str, Value<'_>> = HashMap::from([(HANDLE_TOKEN_OPTION, Value::from(token.as_str()))]);
    let request: OwnedObjectPath = location.call(START_METHOD, &(handle.clone(), "", start_options)).await?;
    if request.as_str() != expected {
        responses = zbus::Proxy::new(&connection, PORTAL_BUS_NAME, request, REQUEST_INTERFACE)
            .await?
            .receive_signal(RESPONSE_SIGNAL)
            .await?;
    }
    let message = responses.next().await.ok_or_else(|| {
        zbus::Error::Failure("the location portal request closed without a response".to_owned())
    })?;
    let (code, _results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize()?;
    Ok(OpenSession { connection, handle, updates, response: StartResponse::from_code(code) })
}

fn error_name(error: &zbus::Error) -> Option<String> {
    match error {
        zbus::Error::MethodError(name, _, _) => Some(name.as_str().to_owned()),
        zbus::Error::FDO(error) => Some(error.name().as_str().to_owned()),
        _ => None,
    }
}

/// reports a failed call, the settled level goes out first and then the failure
fn report_error(error: &zbus::Error, context: &str) {
    tracing::warn!(%error, context, "location: the portal call failed");
    let (next, failure) = failure_for_error(error_name(error).as_deref());
    if let Some(next) = next {
        settle(next, false);
    }
    emit(LocationEvent::Failed(failure));
}

/// opens the session and settles the level, none after a refusal or error (both reported)
async fn granted_session(options: SessionOptions, context: &str, always_report: bool) -> Option<OpenSession> {
    match open_session(options).await {
        Ok(session) => {
            settle(session.response.level(), always_report);
            if session.response == StartResponse::Granted {
                return Some(session);
            }
            if !always_report {
                emit(LocationEvent::Failed(LocationFailure::Denied));
            }
            session.close().await;
            None
        }
        Err(error) => {
            report_error(&error, context);
            None
        }
    }
}

/// permission prompt, first start shows the dialog and its answer is the level
async fn authorize() {
    if let Some(session) = granted_session(SessionOptions::one_shot(), "authorize", true).await {
        session.close().await;
    }
}

async fn request_fix() {
    let Some(mut session) = granted_session(SessionOptions::one_shot(), "request fix", false).await else {
        return;
    };
    match tokio::time::timeout(FIX_TIMEOUT, session.next_fix()).await {
        Ok(Some(fix)) => emit(LocationEvent::Fixes(vec![fix])),
        Ok(None) => emit(LocationEvent::Failed(LocationFailure::Other(BUS_FAILURE_CODE))),
        Err(_) => emit(LocationEvent::Failed(LocationFailure::LocationUnknown)),
    }
    session.close().await;
}

async fn run_updates(request: UpdateRequest, mut stopped: oneshot::Receiver<()>) {
    let opened = tokio::select! {
        session = granted_session(SessionOptions::for_updates(request), "start updates", false) => session,
        _ = &mut stopped => return,
    };
    let Some(mut session) = opened else {
        return;
    };
    loop {
        tokio::select! {
            fix = session.next_fix() => match fix {
                Some(fix) => emit(LocationEvent::Fixes(vec![fix])),
                None => {
                    emit(LocationEvent::Failed(LocationFailure::Other(BUS_FAILURE_CODE)));
                    break;
                }
            },
            _ = &mut stopped => break,
        }
    }
    session.close().await;
}
