use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{SinkExt, StreamExt};
use rise_core::RequestIdAllocator;
use rise_engine::{FrameSender, RiseWire, WireError};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Message;

use super::socket_policy::{
    BackoffSchedule, ConnectionState, SocketCredential, requires_reconnect,
};

// The gateway's 60s reaper counts application frames only; a protocol Pong does not reset it.
const HEARTBEAT: Duration = Duration::from_secs(25);

const OUTBOUND_CAPACITY: usize = 512;

pub(crate) enum SocketCommand {
    Send(String),
    Authenticate(SocketCredential),
    Shutdown,
}

pub struct SocketHost {
    commands: mpsc::Sender<SocketCommand>,
    state: watch::Receiver<ConnectionState>,
}

impl SocketHost {
    pub fn spawn(
        runtime: &Handle,
        url: String,
        credential: SocketCredential,
        requests: Arc<RequestIdAllocator>,
        seed: u64,
        make_wire: impl FnOnce(Arc<dyn FrameSender>) -> Arc<RiseWire>,
    ) -> (Self, Arc<RiseWire>) {
        let (commands, inbox) = mpsc::channel(OUTBOUND_CAPACITY);
        let (state_tx, state) = watch::channel(ConnectionState::Idle);

        let sender: Arc<dyn FrameSender> = Arc::new(ChannelFrameSender {
            commands: commands.clone(),
        });
        let wire = make_wire(sender);

        runtime.spawn(run(
            url,
            credential,
            Arc::clone(&wire),
            requests,
            BackoffSchedule::new(seed),
            inbox,
            state_tx,
        ));

        (Self { commands, state }, wire)
    }

    pub fn subscribe(&self) -> watch::Receiver<ConnectionState> {
        self.state.clone()
    }

    pub(crate) fn commands(&self) -> mpsc::Sender<SocketCommand> {
        self.commands.clone()
    }

    // The gateway reads user and session once at upgrade: a rotated token needs a new connection.
    pub(crate) fn authenticate_through(
        commands: &mpsc::Sender<SocketCommand>,
        credential: SocketCredential,
    ) {
        let _ = commands.try_send(SocketCommand::Authenticate(credential));
    }

    pub fn shutdown(&self) {
        let _ = self.commands.try_send(SocketCommand::Shutdown);
    }
}

struct ChannelFrameSender {
    commands: mpsc::Sender<SocketCommand>,
}

impl FrameSender for ChannelFrameSender {
    fn send(&self, frame: String) -> Result<(), WireError> {
        self.commands
            .try_send(SocketCommand::Send(frame))
            .map_err(|_| WireError::NotConnected)
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    url: String,
    mut credential: SocketCredential,
    wire: Arc<RiseWire>,
    requests: Arc<RequestIdAllocator>,
    backoff: BackoffSchedule,
    mut inbox: mpsc::Receiver<SocketCommand>,
    state: watch::Sender<ConnectionState>,
) {
    let mut attempt: u32 = 0;
    let mut queued: Vec<String> = Vec::new();

    loop {
        let delay = backoff.delay_for(attempt);
        if !delay.is_zero() {
            let _ = state.send(ConnectionState::Reconnecting);
            match wait_for_delay(&mut inbox, &mut queued, &mut credential, delay).await {
                Wait::Elapsed => {}
                Wait::Reauthenticated => attempt = 0,
                Wait::ShuttingDown => break,
            }
        } else {
            let _ = state.send(ConnectionState::Connecting);
        }

        let socket = match connect(&url, &credential).await {
            Ok(socket) => socket,
            Err(error) => {
                attempt = attempt.saturating_add(1);
                tracing::warn!(
                    target: "riseonly",
                    "socket connect failed (attempt {attempt}): {error}"
                );
                continue;
            }
        };

        attempt = 0;
        let _ = state.send(ConnectionState::Connected);
        tracing::info!(target: "riseonly", "socket connected to {url}");

        let outcome = pump(
            socket,
            &wire,
            &requests,
            &mut inbox,
            &mut queued,
            &mut credential,
        )
        .await;

        wire.fail_all_pending();

        match outcome {
            Pump::Reauthenticate => {}
            Pump::Dropped => attempt = 1,
            Pump::ShuttingDown => break,
        }
    }

    let _ = state.send(ConnectionState::Idle);
    wire.fail_all_pending();
}

enum Wait {
    Elapsed,
    Reauthenticated,
    ShuttingDown,
}

async fn wait_for_delay(
    inbox: &mut mpsc::Receiver<SocketCommand>,
    queued: &mut Vec<String>,
    credential: &mut SocketCredential,
    delay: Duration,
) -> Wait {
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);

    loop {
        tokio::select! {
            () = &mut sleep => return Wait::Elapsed,
            command = inbox.recv() => match command {
                Some(SocketCommand::Send(frame)) => push_queued(queued, frame),
                Some(SocketCommand::Authenticate(next)) => {
                    let changed = requires_reconnect(credential, &next);
                    *credential = next;
                    if changed {
                        return Wait::Reauthenticated;
                    }
                }
                Some(SocketCommand::Shutdown) | None => return Wait::ShuttingDown,
            },
        }
    }
}

fn push_queued(queued: &mut Vec<String>, frame: String) {
    if queued.len() >= OUTBOUND_CAPACITY {
        queued.remove(0);
    }
    queued.push(frame);
}

async fn connect(
    url: &str,
    credential: &SocketCredential,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
> {
    let mut request = url
        .into_client_request()
        .map_err(|error| error.to_string())?;

    if let Some(subprotocol) = credential.subprotocol() {
        let value = HeaderValue::from_str(&subprotocol).map_err(|error| error.to_string())?;
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", value);
    }

    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(socket)
}

enum Pump {
    Reauthenticate,
    Dropped,
    ShuttingDown,
}

async fn pump(
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    wire: &Arc<RiseWire>,
    requests: &Arc<RequestIdAllocator>,
    inbox: &mut mpsc::Receiver<SocketCommand>,
    queued: &mut Vec<String>,
    credential: &mut SocketCredential,
) -> Pump {
    let (mut writer, mut reader) = socket.split();

    for frame in queued.drain(..) {
        if writer.send(Message::text(frame)).await.is_err() {
            return Pump::Dropped;
        }
    }

    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;

    loop {
        tokio::select! {
            command = inbox.recv() => match command {
                Some(SocketCommand::Send(frame)) => {
                    if writer.send(Message::text(frame.clone())).await.is_err() {
                        push_queued(queued, frame);
                        return Pump::Dropped;
                    }
                }
                Some(SocketCommand::Authenticate(next)) => {
                    let changed = requires_reconnect(credential, &next);
                    *credential = next;
                    if changed {
                        let _ = writer.close().await;
                        return Pump::Reauthenticate;
                    }
                }
                Some(SocketCommand::Shutdown) | None => {
                    let _ = writer.close().await;
                    return Pump::ShuttingDown;
                }
            },

            _ = heartbeat.tick() => {
                if writer.send(Message::text(heartbeat_frame(requests))).await.is_err() {
                    return Pump::Dropped;
                }
            }

            inbound = reader.next() => match inbound {
                Some(Ok(Message::Text(text))) => wire.accept(text.as_str()),
                Some(Ok(Message::Binary(_))) => tracing::warn!(
                    target: "riseonly",
                    "binary frame received; this client speaks the gateway's text protocol"
                ),
                Some(Ok(Message::Close(_))) | None => return Pump::Dropped,
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    tracing::warn!(target: "riseonly", "socket read failed: {error}");
                    return Pump::Dropped;
                }
            },
        }
    }
}

fn heartbeat_frame(requests: &Arc<RequestIdAllocator>) -> String {
    let id = requests.next().get();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or_default();

    format!(
        r#"{{"id":{id},"type":"service_call","service":"gateway","method":"ping","data":{{}},"timestamp":{timestamp}}}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn the_heartbeat_is_the_gateway_action_an_anonymous_socket_may_call() {
        let requests = Arc::new(RequestIdAllocator::new());
        let frame: Value = serde_json::from_str(&heartbeat_frame(&requests)).unwrap();

        assert_eq!(frame["type"], "service_call");
        assert_eq!(frame["service"], "gateway");
        assert_eq!(frame["method"], "ping");
        assert!(frame["id"].as_u64().is_some());
    }

    #[test]
    fn heartbeats_never_reuse_a_correlation_id() {
        let requests = Arc::new(RequestIdAllocator::new());
        let first: Value = serde_json::from_str(&heartbeat_frame(&requests)).unwrap();
        let second: Value = serde_json::from_str(&heartbeat_frame(&requests)).unwrap();
        assert_ne!(first["id"], second["id"]);
    }

    #[test]
    fn the_heartbeat_period_stays_under_the_gateways_silence_reaper() {
        assert!(
            HEARTBEAT < Duration::from_secs(60),
            "the gateway evicts after sixty seconds of application silence"
        );
    }

    #[test]
    fn a_full_queue_drops_the_oldest_frame_rather_than_the_newest() {
        let mut queued: Vec<String> = (0..OUTBOUND_CAPACITY).map(|n| n.to_string()).collect();
        push_queued(&mut queued, "newest".into());

        assert_eq!(queued.len(), OUTBOUND_CAPACITY);
        assert_eq!(queued.last().map(String::as_str), Some("newest"));
        assert_eq!(queued.first().map(String::as_str), Some("1"));
    }

    #[tokio::test]
    async fn a_frame_sent_while_the_host_is_gone_fails_instead_of_hanging() {
        let (commands, inbox) = mpsc::channel::<SocketCommand>(1);
        drop(inbox);

        let sender = ChannelFrameSender { commands };
        assert!(matches!(
            sender.send("{}".into()),
            Err(WireError::NotConnected)
        ));
    }
}
