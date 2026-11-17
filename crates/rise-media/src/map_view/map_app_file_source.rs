use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Once;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use maplibre_native as mln;
use mln::file_source::{ErrorReason, Response};
use rise_core::foundation::foundation_date::RiseClock;

use super::map_http::*;
use super::map_http_hooks::{MapHttpCancel, MapHttpHooks, map_http_hooks};

pub(super) fn use_app_http_client() {
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        let Some(hooks) = map_http_hooks() else {
            tracing::warn!(target: "riseonly", "map view: no HTTP hooks installed, tiles go through the core's own client");
            return;
        };
        let (commands, inbox) = mpsc::channel();
        let driver_commands = commands.clone();
        let driver = thread::Builder::new()
            .name("riseonly-map-http".to_owned())
            .spawn(move || drive_transfers(hooks, &inbox, &driver_commands));
        match driver {
            Ok(_) => mln::register_file_source(
                mln::FileSourceType::Network,
                AppHttpFileSource { commands, next_id: AtomicU64::new(0) },
            ),
            Err(error) => {
                tracing::warn!(target: "riseonly", %error, "map view: no HTTP thread, tiles go through the core's own client");
            }
        }
    });
}

struct AppHttpFileSource {
    commands: Sender<TransferCommand>,
    next_id: AtomicU64,
}

impl mln::FileSource for AppHttpFileSource {
    fn can_request(&self, request: &mln::ResourceRequest) -> bool {
        request.loading_methods.has_network() && is_http_url(&request.url)
    }

    fn request(&self, request: mln::ResourceRequest, responder: mln::Responder) -> mln::RequestHandle {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let is_tile = request.kind == mln::ResourceKind::Tile;
        let validates = is_tile || request.prior_data.is_some();
        let prior = MapHttpPrior {
            modified: request.prior_modified.and_then(epoch_seconds).filter(|_| validates),
            expires: request.prior_expires.and_then(epoch_seconds),
            etag: request.prior_etag.clone().filter(|_| validates),
        };
        let first_delay = first_request_delay(prior.expires, request.minimum_update_interval, epoch_now());
        let transfer = MapTransfer {
            url: request.url.clone(),
            data_range: request.data_range.map(|range| (*range.start(), *range.end())),
            is_tile,
            low_priority: matches!(request.priority, mln::file_source::Priority::Low),
            first_delay,
            attempts: MapHttpAttempts::new(prior, request.prior_data, request.minimum_update_interval),
            responder,
            cancel: None,
        };
        if self.commands.send(TransferCommand::Add(id, Box::new(transfer))).is_err() {
            return mln::RequestHandle::Done;
        }
        let commands = self.commands.clone();
        mln::RequestHandle::pending(move || {
            let _ = commands.send(TransferCommand::Cancel(id));
        })
    }
}

/// one resource the core asked for, from request until the core drops it
struct MapTransfer {
    url: String,
    data_range: Option<(u64, u64)>,
    is_tile: bool,
    low_priority: bool,
    first_delay: Duration,
    attempts: MapHttpAttempts,
    responder: mln::Responder,
    cancel: Option<MapHttpCancel>,
}

enum TransferCommand {
    Add(u64, Box<MapTransfer>),
    Cancel(u64),
    Finished(u64, MapHttpOutcome),
}

fn drive_transfers(hooks: &'static MapHttpHooks, inbox: &Receiver<TransferCommand>, commands: &Sender<TransferCommand>) {
    let mut scheduler = MapHttpScheduler::new();
    let mut transfers: HashMap<u64, MapTransfer> = HashMap::new();
    loop {
        scheduler.release_due(RiseClock::monotonic());
        while let Some(id) = scheduler.next_start() {
            match transfers.get_mut(&id) {
                Some(transfer) => {
                    let mut answer = TransferAnswer {
                        id,
                        is_tile: transfer.is_tile,
                        commands: Some(commands.clone()),
                    };
                    let prior = &transfer.attempts.prior;
                    let http = map_http_request(&transfer.url, prior.etag.as_deref(), prior.modified, transfer.data_range);
                    transfer.cancel = Some((hooks.start)(http, Box::new(move |reply| answer.send(reply))));
                }
                None => scheduler.finished(id),
            }
        }
        let command = match scheduler.next_due() {
            Some(due) => match inbox.recv_timeout(due.saturating_sub(RiseClock::monotonic())) {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match inbox.recv() {
                Ok(command) => command,
                Err(_) => return,
            },
        };
        match command {
            TransferCommand::Add(id, transfer) => {
                scheduler.schedule(id, transfer.low_priority, due_after(transfer.first_delay));
                transfers.insert(id, *transfer);
            }
            TransferCommand::Cancel(id) => {
                scheduler.cancel(id);
                if let Some(cancel) = transfers.remove(&id).and_then(|transfer| transfer.cancel) {
                    cancel();
                }
            }
            TransferCommand::Finished(id, outcome) => {
                scheduler.finished(id);
                let Some(transfer) = transfers.get_mut(&id) else {
                    continue;
                };
                transfer.cancel = None;
                let (outcome, next) = transfer.attempts.completed(outcome, epoch_now());
                if let Some(delay) = next {
                    scheduler.schedule(id, transfer.low_priority, due_after(delay));
                }
                transfer.responder.send(native_response(outcome));
            }
        }
    }
}

fn due_after(delay: Duration) -> Duration {
    RiseClock::monotonic().saturating_add(delay)
}

struct TransferAnswer {
    id: u64,
    is_tile: bool,
    commands: Option<Sender<TransferCommand>>,
}

impl TransferAnswer {
    fn send(&mut self, answer: MapHttpAnswer) {
        if let Some(commands) = self.commands.take() {
            let outcome = map_http_outcome(answer, self.is_tile, epoch_now());
            let _ = commands.send(TransferCommand::Finished(self.id, outcome));
        }
    }
}

impl Drop for TransferAnswer {
    fn drop(&mut self) {
        self.send(Err(MapHttpFailure::Connection("the transfer ended with its runtime".to_owned())));
    }
}

fn epoch_now() -> i64 {
    RiseClock::now().time_interval_since_1970() as i64
}

fn native_response(outcome: MapHttpOutcome) -> Response {
    let mut response = match outcome.content {
        MapHttpContent::Data(body) => Response::data(body),
        MapHttpContent::NoContent => Response::no_content(),
        MapHttpContent::NotModified => Response::not_modified(),
        MapHttpContent::Failed { kind, message, retry_after } => {
            let failed = Response::error(error_reason(kind), message);
            match retry_after.and_then(system_time) {
                Some(time) => failed.with_retry_after(time),
                None => failed,
            }
        }
    };
    if let Some(time) = outcome.modified.and_then(system_time) {
        response = response.with_modified(time);
    }
    if let Some(time) = outcome.expires.and_then(system_time) {
        response = response.with_expires(time);
    }
    if let Some(etag) = outcome.etag {
        response = response.with_etag(etag);
    }
    response.with_must_revalidate(outcome.must_revalidate)
}

fn error_reason(kind: MapHttpErrorKind) -> ErrorReason {
    match kind {
        MapHttpErrorKind::NotFound => ErrorReason::NotFound,
        MapHttpErrorKind::Server => ErrorReason::Server,
        MapHttpErrorKind::Connection => ErrorReason::Connection,
        MapHttpErrorKind::RateLimit => ErrorReason::RateLimit,
        MapHttpErrorKind::Other => ErrorReason::Other,
    }
}

fn epoch_seconds(time: SystemTime) -> Option<i64> {
    i64::try_from(time.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()
}

fn system_time(epoch: i64) -> Option<SystemTime> {
    UNIX_EPOCH.checked_add(Duration::from_secs(u64::try_from(epoch).ok()?))
}
