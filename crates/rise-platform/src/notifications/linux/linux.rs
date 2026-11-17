use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::mpsc;
use futures::StreamExt as _;
use zbus::zvariant::Value;

use crate::app_info::AppIdentity;
use crate::gpui_shim::PlatformSupport;
use crate::notifications::freedesktop_notify::*;
use crate::notifications::*;

pub(crate) fn host_precondition_met() -> bool {
    true
}

const RECONNECT_DELAY: Duration = Duration::from_secs(5);

enum Command {
    Present(LocalNotification),
    Dismiss(NotificationTag),
}

/// queues between the main thread and the bus service, one per process
struct Route {
    commands: mpsc::UnboundedSender<Command>,
    pending_service: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<Command>>>,
    responses: mpsc::UnboundedSender<ResolvedResponse>,
    pending_responses: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<ResolvedResponse>>>,
    inline_reply: AtomicBool,
}

fn route() -> &'static Route {
    static ROUTE: OnceLock<Route> = OnceLock::new();
    ROUTE.get_or_init(|| {
        let (commands, service) = mpsc::unbounded();
        let (responses, receiver) = mpsc::unbounded();
        Route {
            commands,
            pending_service: parking_lot::Mutex::new(Some(service)),
            responses,
            pending_responses: parking_lot::Mutex::new(Some(receiver)),
            inline_reply: AtomicBool::new(false),
        }
    })
}

thread_local! {
    static RESPONSE_CALLBACK: RefCell<Option<NotificationActionResponseCallback>> =
        const { RefCell::new(None) };
}

pub(crate) struct NativeNotifications;

impl NativeNotificationRoute for NativeNotifications {
    fn present(&self, _app: &gpui::App, notification: &LocalNotification) -> PlatformSupport {
        enqueue(Command::Present(notification.clone()))
    }

    fn dismiss(&self, _app: &gpui::App, tag: &NotificationTag) -> PlatformSupport {
        enqueue(Command::Dismiss(tag.clone()))
    }

    fn on_response(&self, app: &gpui::App, mut callback: NotificationResponseCallback) {
        self.on_action_response(
            app,
            Box::new(move |response, app| {
                let response = gpui::SystemNotificationResponse {
                    tag: response.tag,
                    action_id: response.action_id,
                };
                callback(response, app);
            }),
        );
    }

    fn on_action_response(&self, app: &gpui::App, callback: NotificationActionResponseCallback) {
        RESPONSE_CALLBACK.with(|slot| *slot.borrow_mut() = Some(callback));
        let Some(mut responses) = route().pending_responses.lock().take() else {
            return;
        };
        app.spawn(async move |cx| {
            while let Some(response) = responses.next().await {
                deliver(cx, response);
            }
        })
        .detach();
    }

    fn reply_input_support(&self) -> PlatformSupport {
        if route().inline_reply.load(Ordering::Relaxed) {
            PlatformSupport::Performed
        } else {
            PlatformSupport::Unsupported
        }
    }

    fn take_service(&self, identity: &AppIdentity) -> Option<NotificationService> {
        let commands = route().pending_service.lock().take()?;
        let client = Client {
            app_name: identity.display_name.to_owned(),
            desktop_entry: identity.bundle_identifier.to_owned(),
            responses: route().responses.clone(),
        };
        Some(Box::pin(client.serve(commands)))
    }
}

/// queue is unbounded and lives for the process
/// so a send fails only after the service dropped it, which it doesnt do while running
fn enqueue(command: Command) -> PlatformSupport {
    match route().commands.unbounded_send(command) {
        Ok(()) => PlatformSupport::Performed,
        Err(_) => PlatformSupport::Unsupported,
    }
}

fn deliver(cx: &mut gpui::AsyncApp, response: ResolvedResponse) {
    let response = NotificationActionResponse {
        tag: response.tag.as_str().to_owned().into(),
        action_id: response.action_id.map(Into::into),
        user_text: response.user_text,
    };
    let Some(mut callback) = RESPONSE_CALLBACK.with(|slot| slot.borrow_mut().take()) else {
        return;
    };
    cx.update(|app| callback(response, app));
    RESPONSE_CALLBACK.with(|slot| {
        slot.borrow_mut().get_or_insert(callback);
    });
}

struct Client {
    app_name: String,
    desktop_entry: String,
    responses: mpsc::UnboundedSender<ResolvedResponse>,
}

impl Client {
    async fn serve(self, mut commands: mpsc::UnboundedReceiver<Command>) {
        loop {
            match self.run(&mut commands).await {
                Ok(()) => return,
                Err(error) => {
                    route().inline_reply.store(false, Ordering::Relaxed);
                    tracing::warn!(%error, "notifications: session bus client stopped, retrying");
                    tokio::time::sleep(RECONNECT_DELAY).await;
                }
            }
        }
    }

    /// ok only when the command queue closed
    async fn run(&self, commands: &mut mpsc::UnboundedReceiver<Command>) -> zbus::Result<()> {
        let connection = zbus::Connection::session().await?;
        let server = zbus::Proxy::new(
            &connection,
            NOTIFICATIONS_SERVICE,
            NOTIFICATIONS_PATH,
            NOTIFICATIONS_INTERFACE,
        )
        .await?;
        let mut invoked = std::pin::pin!(server.receive_signal(ACTION_INVOKED_SIGNAL).await?);
        let mut closed = std::pin::pin!(server.receive_signal(NOTIFICATION_CLOSED_SIGNAL).await?);
        let mut replied = std::pin::pin!(server.receive_signal(NOTIFICATION_REPLIED_SIGNAL).await?);
        let capabilities = ServerCapabilities::from_list(
            &server
                .call::<_, _, Vec<String>>(GET_CAPABILITIES_METHOD, &())
                .await?,
        );
        route().inline_reply.store(capabilities.inline_reply, Ordering::Relaxed);
        let mut live = LiveNotifications::default();

        loop {
            tokio::select! {
                command = commands.next() => match command {
                    None => return Ok(()),
                    Some(Command::Present(notification)) => {
                        self.present(&server, &capabilities, &mut live, &notification).await?;
                    }
                    Some(Command::Dismiss(tag)) => {
                        if let Some(id) = live.take_for_dismiss(&tag) {
                            Self::dismiss(&server, id).await?;
                        }
                    }
                },
                Some(message) = invoked.next() => {
                    match message.body().deserialize::<(u32, String)>() {
                        Ok((id, key)) => {
                            if let Some(response) = live.resolve(id, &key) {
                                self.responses.unbounded_send(response).ok();
                            }
                        }
                        Err(error) => tracing::warn!(%error, "notifications: malformed ActionInvoked"),
                    }
                }
                Some(message) = replied.next() => {
                    match message.body().deserialize::<(u32, String)>() {
                        Ok((id, text)) => {
                            if let Some(response) = live.resolve_reply(id, &text) {
                                self.responses.unbounded_send(response).ok();
                            }
                        }
                        Err(error) => tracing::warn!(%error, "notifications: malformed NotificationReplied"),
                    }
                }
                Some(message) = closed.next() => {
                    match message.body().deserialize::<(u32, u32)>() {
                        Ok((id, reason)) => {
                            live.closed(id, CloseReason::from_code(reason), &capabilities);
                        }
                        Err(error) => tracing::warn!(%error, "notifications: malformed NotificationClosed"),
                    }
                }
                else => {
                    return Err(zbus::Error::Failure(
                        "the notification server closed its signal streams".to_owned(),
                    ));
                }
            }
        }
    }

    /// a refused post (bad hint or server policy) is logged and dropped
    /// only a lost bus stops the client
    async fn present(
        &self,
        server: &zbus::Proxy<'_>,
        capabilities: &ServerCapabilities,
        live: &mut LiveNotifications,
        notification: &LocalNotification,
    ) -> zbus::Result<()> {
        let request = notify_request(
            notification,
            live.replaces_id(&notification.tag),
            capabilities,
            Some(self.desktop_entry.as_str()),
        );
        let hints: HashMap<&str, Value<'_>> = request
            .hints
            .iter()
            .map(|(key, value)| (*key, hint_value(value)))
            .collect();
        let actions: Vec<&str> = request.actions.iter().map(String::as_str).collect();
        let answer = server
            .call::<_, _, u32>(
                NOTIFY_METHOD,
                &(
                    self.app_name.as_str(),
                    request.replaces_id,
                    "",
                    request.summary.as_str(),
                    request.body.as_str(),
                    actions,
                    hints,
                    request.expire_timeout,
                ),
            )
            .await;
        match answer {
            Ok(id) => {
                live.record(id, notification);
                Ok(())
            }
            Err(zbus::Error::MethodError(name, detail, _)) => {
                tracing::warn!(%name, ?detail, "notifications: server refused Notify");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

impl Client {
    /// closing an id the server dropped gives an empty dbus error
    /// user can close the banner while NotificationClosed is queued
    /// refusal is only logged, a lost bus stops the client
    async fn dismiss(server: &zbus::Proxy<'_>, id: u32) -> zbus::Result<()> {
        match server.call::<_, _, ()>(CLOSE_NOTIFICATION_METHOD, &(id,)).await {
            Ok(()) => Ok(()),
            Err(zbus::Error::MethodError(name, detail, _)) => {
                tracing::warn!(%name, ?detail, "notifications: server refused CloseNotification");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

fn hint_value(value: &HintValue) -> Value<'_> {
    match value {
        HintValue::Text(text) => Value::from(text.as_str()),
        HintValue::Flag(flag) => Value::from(*flag),
    }
}
