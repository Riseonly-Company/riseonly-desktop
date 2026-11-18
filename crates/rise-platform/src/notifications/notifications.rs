use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::app_info::AppIdentity;
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

pub use rise_notify::{
    ClientState, Decision, MuteScopes, MuteState, Notification, NotificationScope, Route,
    SelfAuthoredMode, SuppressionReason, decide, mute_is_active, mute_until_millis, scope_allows,
};

/// reads the mute_scopes column, keys come from the canon scope names
/// four keys or nothing, a missing replies follows mentions
pub fn parse_mute_scopes(raw: &str) -> Option<MuteScopes> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let flag = |scope: NotificationScope| {
        value
            .get(scope.as_str())
            .and_then(serde_json::Value::as_bool)
    };

    MuteScopes::from_parts(
        flag(NotificationScope::Messages),
        flag(NotificationScope::Mentions),
        flag(NotificationScope::Replies),
        flag(NotificationScope::Reactions),
        flag(NotificationScope::Events),
    )
}

/// column as the chat row has it, anything unreadable means never configured
pub fn mute_scopes_from_stored(raw: Option<&str>) -> MuteScopes {
    raw.and_then(parse_mute_scopes)
        .unwrap_or(MuteScopes::ALL_ENABLED)
}

pub fn mute_scopes_to_json(scopes: &MuteScopes) -> String {
    let map: serde_json::Map<String, serde_json::Value> = NotificationScope::ALL
        .into_iter()
        .map(|scope| {
            (
                scope.as_str().to_owned(),
                serde_json::Value::Bool(scope.is_enabled_in(scopes)),
            )
        })
        .collect();

    serde_json::Value::Object(map).to_string()
}

const CONVERSATION_TAG_PREFIX: &str = "conv:";
const UNIQUE_TAG_PREFIX: &str = "one:";

/// identity gpui replaces on, a post with the same tag as a live one replaces it
/// the two constructors are namespaced against each other
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NotificationTag(String);

impl NotificationTag {
    /// one tag per conversation, the newest message replaces the previous one
    pub fn conversation(route_key: &str) -> Self {
        Self(format!("{CONVERSATION_TAG_PREFIX}{route_key}"))
    }

    /// one banner per key, caller gives something already unique like the event id
    pub fn unique(key: &str) -> Self {
        Self(format!("{UNIQUE_TAG_PREFIX}{key}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// route of a conversation tag, since gpui hands the tag back in the response
    pub fn conversation_route(&self) -> Option<&str> {
        self.0.strip_prefix(CONVERSATION_TAG_PREFIX)
    }
}

/// typing side of a text input action, already translated like the labels
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NotificationTextInput {
    pub button_title: String,
    pub placeholder: String,
}

/// a button on a notification, gpui actions carry only a label and an id
/// so text input works only on a native route that supports it, elsewhere its a plain click
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
    pub text_input: Option<NotificationTextInput>,
}

impl NotificationAction {
    pub const REPLY_ID: &str = "REPLY_ACTION";
    pub const MARK_READ_ID: &str = "MARK_READ_ACTION";

    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            text_input: None,
        }
    }

    /// adds a text input to the button, with its button title and placeholder
    pub fn with_text_input(
        mut self,
        button_title: impl Into<String>,
        placeholder: impl Into<String>,
    ) -> Self {
        self.text_input = Some(NotificationTextInput {
            button_title: button_title.into(),
            placeholder: placeholder.into(),
        });
        self
    }

    /// reply button, labels come already translated since this crate has no locale
    pub fn reply(label: impl Into<String>) -> Self {
        Self::new(Self::REPLY_ID, label)
    }

    pub fn mark_read(label: impl Into<String>) -> Self {
        Self::new(Self::MARK_READ_ID, label)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LocalNotification {
    pub tag: NotificationTag,
    pub title: String,
    pub body: String,
    pub actions: Vec<NotificationAction>,
    pub avatar: Option<PathBuf>,
    pub attachment: Option<PathBuf>,
    pub thread: Option<String>,
    pub sender: Option<String>,
    pub group_author: Option<String>,
    pub group_avatar: Option<PathBuf>,
}

impl LocalNotification {
    pub fn new(tag: NotificationTag, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            tag,
            title: title.into(),
            body: body.into(),
            actions: Vec::new(),
            avatar: None,
            attachment: None,
            thread: None,
            sender: None,
            group_author: None,
            group_avatar: None,
        }
    }

    pub fn with_action(mut self, action: NotificationAction) -> Self {
        self.actions.push(action);
        self
    }

    pub fn with_avatar(mut self, path: impl Into<PathBuf>) -> Self {
        self.avatar = Some(path.into());
        self
    }

    pub fn with_attachment(mut self, path: impl Into<PathBuf>) -> Self {
        self.attachment = Some(path.into());
        self
    }

    pub fn with_thread(mut self, thread: impl Into<String>) -> Self {
        self.thread = Some(thread.into());
        self
    }

    pub fn with_sender(mut self, sender: impl Into<String>) -> Self {
        self.sender = Some(sender.into());
        self
    }

    pub fn with_group_author(mut self, author_id: impl Into<String>) -> Self {
        self.group_author = Some(author_id.into());
        self
    }

    pub fn with_group_avatar(mut self, path: impl Into<PathBuf>) -> Self {
        self.group_avatar = Some(path.into());
        self
    }

    /// member id if the banner comes from a group chat, empty id means its not a group
    pub fn group_author_id(&self) -> Option<&str> {
        self.group_author.as_deref().filter(|id| !id.is_empty())
    }
}

impl From<NotificationAction> for gpui::SystemNotificationAction {
    fn from(action: NotificationAction) -> Self {
        Self {
            id: action.id.into(),
            label: action.label.into(),
        }
    }
}

impl From<LocalNotification> for gpui::SystemNotification {
    fn from(notification: LocalNotification) -> Self {
        Self {
            tag: notification.tag.0.into(),
            title: notification.title.into(),
            body: notification.body.into(),
            actions: notification.actions.into_iter().map(Into::into).collect(),
        }
    }
}

/// what each backend needs before gpui posts anything at all
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeliveryPrecondition {
    AppBundle,
    AppIdentity,
    NotificationServer,
}

pub const fn delivery_precondition(host: HostOs) -> DeliveryPrecondition {
    match host {
        HostOs::MacOs => DeliveryPrecondition::AppBundle,
        HostOs::Windows => DeliveryPrecondition::AppIdentity,
        HostOs::Linux => DeliveryPrecondition::NotificationServer,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotificationAvailability {
    Available,
    Blocked(DeliveryPrecondition),
}

impl NotificationAvailability {
    pub const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

pub const fn availability_for(host: HostOs, precondition_met: bool) -> NotificationAvailability {
    if precondition_met {
        NotificationAvailability::Available
    } else {
        NotificationAvailability::Blocked(delivery_precondition(host))
    }
}

/// true when posting over a live notification with the same tag replaces it
/// gpui never sets the xdg replaces id so linux uses its own route that keeps the server id per tag
pub const fn tag_replaces_previous(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs | HostOs::Windows | HostOs::Linux => PlatformSupport::Performed,
    }
}

/// whether a notification can be retracted once posted
/// linux retracts with CloseNotification on the id it kept for the tag
pub const fn supports_dismissal(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs | HostOs::Windows | HostOs::Linux => PlatformSupport::Performed,
    }
}

/// authorization status, Denied and NotDetermined count as system notifications turned off
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationAuthorizationStatus {
    NotDetermined,
    Denied,
    Authorized,
    Provisional,
    Ephemeral,
}

/// status as the host reports it, none where the host has no per app consent
pub type NotificationAuthorizationQuery =
    Pin<Box<dyn Future<Output = Option<NotificationAuthorizationStatus>> + Send + 'static>>;

/// status for a host without per app consent
/// a post that reaches the os counts as allowed, one that always gets dropped counts as denied
pub const fn resolved_authorization_status(
    reported: Option<NotificationAuthorizationStatus>,
    availability: NotificationAvailability,
) -> NotificationAuthorizationStatus {
    match (reported, availability) {
        (Some(status), _) => status,
        (None, NotificationAvailability::Available) => NotificationAuthorizationStatus::Authorized,
        (None, NotificationAvailability::Blocked(_)) => NotificationAuthorizationStatus::Denied,
    }
}

/// bus service a native route needs on the engine runtime while the app runs
/// commands posted before it is spawned wait in its queue
pub type NotificationService = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// callback that gets a click on a notification body or one of its buttons
pub type NotificationResponseCallback =
    Box<dyn FnMut(gpui::SystemNotificationResponse, &mut gpui::App) + 'static>;

/// click on a notification body or button plus any text typed into a text input button
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NotificationActionResponse {
    pub tag: gpui::SharedString,
    pub action_id: Option<gpui::SharedString>,
    pub user_text: Option<String>,
}

impl From<gpui::SystemNotificationResponse> for NotificationActionResponse {
    fn from(response: gpui::SystemNotificationResponse) -> Self {
        Self {
            tag: response.tag,
            action_id: response.action_id,
            user_text: None,
        }
    }
}

pub type NotificationActionResponseCallback =
    Box<dyn FnMut(NotificationActionResponse, &mut gpui::App) + 'static>;

pub trait NativeNotificationRoute {
    /// performed means the post was handed on, not that a banner showed up
    fn present(&self, app: &gpui::App, notification: &LocalNotification) -> PlatformSupport {
        app.show_system_notification(notification.clone().into());
        PlatformSupport::Performed
    }

    fn dismiss(&self, app: &gpui::App, tag: &NotificationTag) -> PlatformSupport {
        app.dismiss_system_notification(tag.as_str());
        PlatformSupport::Performed
    }

    fn dismiss_all(&self, app: &gpui::App) -> PlatformSupport {
        let _ = app;
        PlatformSupport::Unsupported
    }

    /// callback runs on the main thread, a later call replaces the earlier one like gpui does
    fn on_response(&self, app: &gpui::App, callback: NotificationResponseCallback) {
        app.on_system_notification_response(callback);
    }

    fn on_action_response(&self, app: &gpui::App, mut callback: NotificationActionResponseCallback) {
        self.on_response(
            app,
            Box::new(move |response, app| callback(response.into(), app)),
        );
    }

    fn reply_input_support(&self) -> PlatformSupport {
        PlatformSupport::Unsupported
    }

    fn attachment_fallback_support(&self) -> PlatformSupport {
        PlatformSupport::Unsupported
    }

    fn request_authorization(&self) -> PlatformSupport {
        PlatformSupport::Unsupported
    }

    fn take_service(&self, identity: &AppIdentity) -> Option<NotificationService> {
        let _ = identity;
        None
    }

    fn authorization_status(&self) -> NotificationAuthorizationQuery {
        Box::pin(std::future::ready(None))
    }
}

pub async fn authorization_status() -> NotificationAuthorizationStatus {
    let query = route().authorization_status();
    let reported = query.await;
    resolved_authorization_status(reported, availability_for(HostOs::current(), precondition_met()))
}

/// notification settings page url
/// linux has none since every desktop keeps its own
pub const fn notification_settings_url(host: HostOs) -> Option<&'static str> {
    match host {
        HostOs::MacOs => Some("x-apple.systempreferences:com.apple.preference.notifications"),
        HostOs::Windows => Some("ms-settings:notifications"),
        HostOs::Linux => None,
    }
}

/// the page for this app where the system lets one, else the general notification page
pub fn open_notification_settings(app: &mut gpui::App) -> PlatformSupport {
    super::os::open_settings(app)
}

/// true when open_notification_settings has somewhere to go
pub fn can_open_notification_settings() -> bool {
    super::os::settings_reachable()
}

/// registers the handler for notification clicks on whichever route this host posts by
pub fn on_notification_response<F>(app: &gpui::App, callback: F)
where
    F: FnMut(NotificationActionResponse, &mut gpui::App) + 'static,
{
    route().on_action_response(app, Box::new(callback));
}

pub fn reply_input_support() -> PlatformSupport {
    route().reply_input_support()
}

pub fn attachment_fallback_support() -> PlatformSupport {
    route().attachment_fallback_support()
}

pub fn request_authorization() -> PlatformSupport {
    route().request_authorization()
}

/// service to spawn once on the engine runtime at startup, if the host needs one
/// linux is the session bus client and windows unpackaged is the aumid registration
/// without it linux posts just queue and windows has no identity
pub fn take_notification_service(identity: &AppIdentity) -> Option<NotificationService> {
    route().take_service(identity)
}

/// posting and retracting, not Send and Sync since the real one borrows the gpui app
pub trait NotificationPresenter {
    /// performed means the post reached gpui, not that a banner showed up
    fn present(&self, notification: &LocalNotification) -> PlatformSupport;
    fn dismiss(&self, tag: &NotificationTag) -> PlatformSupport;
    fn dismiss_all(&self) -> PlatformSupport {
        PlatformSupport::Unsupported
    }
    fn availability(&self) -> NotificationAvailability;
}

/// built per call site from a borrowed app
pub struct GpuiNotificationPresenter<'a> {
    app: &'a gpui::App,
}

impl<'a> GpuiNotificationPresenter<'a> {
    pub fn new(app: &'a gpui::App) -> Self {
        Self { app }
    }
}

impl NotificationPresenter for GpuiNotificationPresenter<'_> {
    fn present(&self, notification: &LocalNotification) -> PlatformSupport {
        if !self.availability().is_available() {
            return PlatformSupport::Unsupported;
        }

        route().present(self.app, notification)
    }

    fn dismiss(&self, tag: &NotificationTag) -> PlatformSupport {
        if !supports_dismissal(HostOs::current()).is_supported() {
            return PlatformSupport::Unsupported;
        }

        route().dismiss(self.app, tag)
    }

    fn dismiss_all(&self) -> PlatformSupport {
        if !self.availability().is_available() {
            return PlatformSupport::Unsupported;
        }

        route().dismiss_all(self.app)
    }

    fn availability(&self) -> NotificationAvailability {
        availability_for(HostOs::current(), precondition_met())
    }
}

static NATIVE_ROUTE: super::os::NativeNotifications = super::os::NativeNotifications;
static TESTER_ROUTE: TesterNotifications = TesterNotifications;

fn route() -> &'static dyn NativeNotificationRoute {
    if uses_tester_receiver() {
        return &TESTER_ROUTE;
    }
    &NATIVE_ROUTE
}

fn precondition_met() -> bool {
    uses_tester_receiver() || super::os::host_precondition_met()
}

static TESTER_RECEIVER: AtomicBool = AtomicBool::new(false);
static TESTER_NEXT_ID: AtomicU64 = AtomicU64::new(1);
static TESTER_DELIVERED: Mutex<Vec<TesterNotification>> = Mutex::new(Vec::new());
static TESTER_PENDING_CATEGORIES: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

thread_local! {
    static TESTER_RESPONSE: RefCell<Option<NotificationActionResponseCallback>> = const { RefCell::new(None) };
}

pub fn set_tester_receiver(on: bool) {
    TESTER_RECEIVER.store(on, Ordering::Relaxed);
}

pub fn uses_tester_receiver() -> bool {
    TESTER_RECEIVER.load(Ordering::Relaxed)
}

/// notification held by the tester receiver, as the system center would show it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TesterNotification {
    pub id: u64,
    pub tag: String,
    pub title: String,
    pub body: String,
    pub category: Option<String>,
    pub thread: Option<String>,
    pub sender: Option<String>,
    pub avatar: Option<PathBuf>,
    pub attachment: Option<PathBuf>,
    pub actions: Vec<NotificationAction>,
    pub sound: bool,
}

/// what the driver does to a held notification
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TesterResponse {
    Click,
    Action(String),
    Reply(String),
}

pub fn note_tester_category(tag: &str, category: Option<&str>) {
    if !uses_tester_receiver() {
        return;
    }
    let mut pending = TESTER_PENDING_CATEGORIES.lock().unwrap_or_else(PoisonError::into_inner);
    let map = pending.get_or_insert_with(HashMap::new);
    match category {
        Some(category) => {
            map.insert(tag.to_owned(), category.to_owned());
        }
        None => {
            map.remove(tag);
        }
    }
}

/// every notification the receiver holds, oldest first
pub fn tester_notifications() -> Vec<TesterNotification> {
    TESTER_DELIVERED.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub fn respond_to_tester_notification(
    id: u64,
    response: TesterResponse,
    app: &mut gpui::App,
) -> Result<(), String> {
    let found = tester_notifications().into_iter().find(|notification| notification.id == id);
    let Some(notification) = found else {
        return Err(format!("notifications: no notification {id}"));
    };
    let (action_id, user_text) = match response {
        TesterResponse::Click => (None, None),
        TesterResponse::Action(button) => {
            let action = notification
                .actions
                .iter()
                .find(|action| action.id == button || action.label == button)
                .ok_or_else(|| format!("notifications: {id} has no button {button}"))?;
            (Some(action.id.clone()), None)
        }
        TesterResponse::Reply(text) => {
            let action = notification
                .actions
                .iter()
                .find(|action| action.text_input.is_some())
                .ok_or_else(|| format!("notifications: {id} has no reply button"))?;
            (Some(action.id.clone()), Some(text))
        }
    };
    let callback = TESTER_RESPONSE.with(|slot| slot.borrow_mut().take());
    let Some(mut callback) = callback else {
        return Err("notifications: no response handler registered yet".to_owned());
    };
    TESTER_DELIVERED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|held| held.id != id);
    callback(
        NotificationActionResponse {
            tag: notification.tag.into(),
            action_id: action_id.map(Into::into),
            user_text,
        },
        app,
    );
    TESTER_RESPONSE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(callback);
        }
    });
    Ok(())
}

struct TesterNotifications;

impl NativeNotificationRoute for TesterNotifications {
    fn present(&self, _app: &gpui::App, notification: &LocalNotification) -> PlatformSupport {
        let category = TESTER_PENDING_CATEGORIES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .and_then(|map| map.remove(notification.tag.as_str()));
        let held = TesterNotification {
            id: TESTER_NEXT_ID.fetch_add(1, Ordering::Relaxed),
            tag: notification.tag.as_str().to_owned(),
            title: notification.title.clone(),
            body: notification.body.clone(),
            category,
            thread: notification.thread.clone(),
            sender: notification.sender.clone(),
            avatar: notification.avatar.clone(),
            attachment: notification.attachment.clone(),
            actions: notification.actions.clone(),
            sound: !cfg!(debug_assertions),
        };
        tracing::info!(
            target: "riseonly::X1979",
            id = held.id,
            tag = %held.tag,
            title = %held.title,
            "tester notification held"
        );
        let mut delivered = TESTER_DELIVERED.lock().unwrap_or_else(PoisonError::into_inner);
        delivered.retain(|known| known.tag != held.tag);
        delivered.push(held);
        PlatformSupport::Performed
    }

    fn dismiss(&self, _app: &gpui::App, tag: &NotificationTag) -> PlatformSupport {
        TESTER_DELIVERED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|known| known.tag != tag.as_str());
        PlatformSupport::Performed
    }

    fn dismiss_all(&self, _app: &gpui::App) -> PlatformSupport {
        TESTER_DELIVERED.lock().unwrap_or_else(PoisonError::into_inner).clear();
        PlatformSupport::Performed
    }

    fn on_response(&self, app: &gpui::App, mut callback: NotificationResponseCallback) {
        self.on_action_response(
            app,
            Box::new(move |response, app| {
                callback(
                    gpui::SystemNotificationResponse {
                        tag: response.tag,
                        action_id: response.action_id,
                    },
                    app,
                )
            }),
        );
    }

    fn on_action_response(&self, _app: &gpui::App, callback: NotificationActionResponseCallback) {
        TESTER_RESPONSE.with(|slot| *slot.borrow_mut() = Some(callback));
    }

    fn reply_input_support(&self) -> PlatformSupport {
        PlatformSupport::Performed
    }

    fn attachment_fallback_support(&self) -> PlatformSupport {
        PlatformSupport::Performed
    }

    fn request_authorization(&self) -> PlatformSupport {
        PlatformSupport::Performed
    }

    fn authorization_status(&self) -> NotificationAuthorizationQuery {
        Box::pin(std::future::ready(Some(NotificationAuthorizationStatus::Authorized)))
    }
}

/// records instead of posting, not a fallback since a caller here thinks it notified the user
pub struct InMemoryNotificationPresenter {
    availability: NotificationAvailability,
    presented: parking_lot::Mutex<Vec<LocalNotification>>,
    dismissed: parking_lot::Mutex<Vec<NotificationTag>>,
    dismissed_all: parking_lot::Mutex<usize>,
}

impl InMemoryNotificationPresenter {
    pub fn new() -> Self {
        Self {
            availability: NotificationAvailability::Available,
            presented: parking_lot::Mutex::new(Vec::new()),
            dismissed: parking_lot::Mutex::new(Vec::new()),
            dismissed_all: parking_lot::Mutex::new(0),
        }
    }

    pub fn blocked(precondition: DeliveryPrecondition) -> Self {
        Self {
            availability: NotificationAvailability::Blocked(precondition),
            presented: parking_lot::Mutex::new(Vec::new()),
            dismissed: parking_lot::Mutex::new(Vec::new()),
            dismissed_all: parking_lot::Mutex::new(0),
        }
    }

    pub fn presented(&self) -> Vec<LocalNotification> {
        self.presented.lock().clone()
    }

    pub fn dismissed(&self) -> Vec<NotificationTag> {
        self.dismissed.lock().clone()
    }

    /// how many times everything was retracted
    pub fn dismissed_all(&self) -> usize {
        *self.dismissed_all.lock()
    }
}

impl Default for InMemoryNotificationPresenter {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationPresenter for InMemoryNotificationPresenter {
    fn present(&self, notification: &LocalNotification) -> PlatformSupport {
        if !self.availability.is_available() {
            return PlatformSupport::Unsupported;
        }

        self.presented.lock().push(notification.clone());
        PlatformSupport::Performed
    }

    fn dismiss(&self, tag: &NotificationTag) -> PlatformSupport {
        if !self.availability.is_available() {
            return PlatformSupport::Unsupported;
        }

        self.dismissed.lock().push(tag.clone());
        PlatformSupport::Performed
    }

    fn dismiss_all(&self) -> PlatformSupport {
        if !self.availability.is_available() {
            return PlatformSupport::Unsupported;
        }

        *self.dismissed_all.lock() += 1;
        PlatformSupport::Performed
    }

    fn availability(&self) -> NotificationAvailability {
        self.availability
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_scopes_round_trip_through_the_stored_column() {
        let scopes = MuteScopes {
            messages: true,
            mentions: true,
            replies: true,
            reactions: false,
            events: false,
        };

        let stored = mute_scopes_to_json(&scopes);
        assert!(stored.contains("\"reactions\":false"), "{stored}");
        assert_eq!(parse_mute_scopes(&stored), Some(scopes));
    }

    #[test]
    fn a_partial_mute_scopes_payload_is_refused_whole_rather_than_half_applied() {
        assert_eq!(parse_mute_scopes("{\"messages\":false}"), None);
        assert_eq!(
            mute_scopes_from_stored(Some("{\"messages\":false}")),
            MuteScopes::ALL_ENABLED,
            "half a payload must not silence scopes the server never mentioned"
        );
    }

    #[test]
    fn a_non_boolean_scope_value_is_refused_whole() {
        assert_eq!(
            parse_mute_scopes(
                "{\"messages\":\"no\",\"mentions\":true,\"reactions\":true,\"events\":true}"
            ),
            None
        );
    }

    #[test]
    fn an_absent_or_unreadable_column_reads_as_all_enabled() {
        assert_eq!(mute_scopes_from_stored(None), MuteScopes::ALL_ENABLED);
        assert_eq!(mute_scopes_from_stored(Some("")), MuteScopes::ALL_ENABLED);
        assert_eq!(
            mute_scopes_from_stored(Some("not json")),
            MuteScopes::ALL_ENABLED
        );
    }

    #[test]
    fn two_messages_in_one_conversation_share_a_tag() {
        assert_eq!(
            NotificationTag::conversation("chat/42"),
            NotificationTag::conversation("chat/42"),
            "forty messages must replace one banner, not stack forty"
        );
    }

    #[test]
    fn two_conversations_never_share_a_tag() {
        assert_ne!(
            NotificationTag::conversation("chat/42"),
            NotificationTag::conversation("chat/43")
        );
    }

    #[test]
    fn a_conversation_tag_cannot_collide_with_a_standalone_one() {
        assert_ne!(
            NotificationTag::conversation("x"),
            NotificationTag::unique("x")
        );
    }

    #[test]
    fn every_standalone_notification_keeps_its_own_banner() {
        assert_ne!(
            NotificationTag::unique("event-1"),
            NotificationTag::unique("event-2")
        );
    }

    #[test]
    fn a_conversation_tag_hands_its_route_back_for_the_click() {
        let tag = NotificationTag::conversation("chat/42");
        assert_eq!(tag.conversation_route(), Some("chat/42"));
        assert_eq!(
            NotificationTag::unique("chat/42").conversation_route(),
            None
        );
    }

    #[test]
    fn a_local_notification_maps_onto_the_gpui_payload_field_for_field() {
        let tag = NotificationTag::conversation("chat/42");
        let body = "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}";
        let notification = LocalNotification::new(tag, "Ayan", body)
            .with_action(NotificationAction::mark_read("Mark as read"));

        let posted: gpui::SystemNotification = notification.into();

        assert_eq!(posted.tag, "conv:chat/42");
        assert_eq!(posted.title, "Ayan");
        assert_eq!(posted.body, body);
        assert_eq!(posted.actions.len(), 1);
        assert_eq!(posted.actions[0].id, NotificationAction::MARK_READ_ID);
        assert_eq!(posted.actions[0].label, "Mark as read");
    }

    #[test]
    fn a_notification_without_buttons_maps_to_an_empty_action_list() {
        let notification = LocalNotification::new(NotificationTag::unique("e1"), "t", "b");
        let posted: gpui::SystemNotification = notification.into();

        assert!(posted.actions.is_empty());
    }

    #[test]
    fn every_host_names_its_own_delivery_precondition() {
        for host in HostOs::ALL {
            let shared = HostOs::ALL
                .iter()
                .filter(|other| delivery_precondition(**other) == delivery_precondition(host))
                .count();

            assert_eq!(
                shared, 1,
                "{host:?} cannot be told apart from another host's precondition"
            );
        }
    }

    #[test]
    fn no_host_is_inherently_unable_to_post() {
        for host in HostOs::ALL {
            assert_eq!(
                availability_for(host, true),
                NotificationAvailability::Available,
                "gpui implements show_system_notification on all three backends"
            );
        }
    }

    #[test]
    fn an_unmet_precondition_reports_which_one() {
        assert_eq!(
            availability_for(HostOs::MacOs, false),
            NotificationAvailability::Blocked(DeliveryPrecondition::AppBundle)
        );
        assert_eq!(
            availability_for(HostOs::Windows, false),
            NotificationAvailability::Blocked(DeliveryPrecondition::AppIdentity)
        );
        assert_eq!(
            availability_for(HostOs::Linux, false),
            NotificationAvailability::Blocked(DeliveryPrecondition::NotificationServer)
        );
    }

    #[test]
    fn every_host_replaces_the_banner_of_a_repeated_tag() {
        assert!(tag_replaces_previous(HostOs::MacOs).is_supported());
        assert!(tag_replaces_previous(HostOs::Windows).is_supported());
        assert!(
            tag_replaces_previous(HostOs::Linux).is_supported(),
            "the Linux route passes the kept server id as replaces_id"
        );
    }

    #[test]
    fn every_host_can_retract_a_notification() {
        assert!(supports_dismissal(HostOs::MacOs).is_supported());
        assert!(supports_dismissal(HostOs::Windows).is_supported());
        assert!(supports_dismissal(HostOs::Linux).is_supported());
    }

    #[test]
    fn a_rich_notification_keeps_its_avatar_thread_and_sender() {
        let notification = LocalNotification::new(NotificationTag::conversation("c1"), "Ayan", "hi")
            .with_avatar("/tmp/avatar.png")
            .with_thread("c1")
            .with_sender("Ayan");
        assert_eq!(notification.avatar, Some(PathBuf::from("/tmp/avatar.png")));
        assert_eq!(notification.thread.as_deref(), Some("c1"));
        assert_eq!(notification.sender.as_deref(), Some("Ayan"));
    }

    #[test]
    fn a_plain_notification_has_no_rich_parts() {
        let notification = LocalNotification::new(NotificationTag::unique("e1"), "t", "b");
        assert_eq!(notification.avatar, None);
        assert_eq!(notification.thread, None);
        assert_eq!(notification.sender, None);
        assert_eq!(notification.group_author_id(), None);
        assert_eq!(notification.group_avatar, None);
    }

    #[test]
    fn a_group_message_keeps_the_member_and_the_group_picture() {
        let notification = LocalNotification::new(NotificationTag::conversation("g1"), "Team", "hi")
            .with_thread("g1")
            .with_sender("Ayan")
            .with_avatar("/tmp/member.png")
            .with_group_author("u7")
            .with_group_avatar("/tmp/group.png");
        assert_eq!(notification.group_author_id(), Some("u7"));
        assert_eq!(notification.avatar, Some(PathBuf::from("/tmp/member.png")));
        assert_eq!(notification.group_avatar, Some(PathBuf::from("/tmp/group.png")));
    }

    #[test]
    fn an_empty_author_id_is_not_a_group() {
        let notification = LocalNotification::new(NotificationTag::conversation("g1"), "t", "b")
            .with_group_author("");
        assert_eq!(notification.group_author_id(), None, "a direct chat keeps the sender look");
    }

    #[test]
    fn the_recording_presenter_keeps_what_it_was_handed() {
        let presenter = InMemoryNotificationPresenter::new();
        let tag = NotificationTag::conversation("chat/42");
        let notification = LocalNotification::new(tag, "Ayan", "hi");

        assert_eq!(presenter.present(&notification), PlatformSupport::Performed);
        assert_eq!(presenter.presented(), vec![notification]);
    }

    #[test]
    fn dismissing_records_the_tag_it_was_asked_to_retract() {
        let presenter = InMemoryNotificationPresenter::new();
        let tag = NotificationTag::conversation("chat/42");

        assert_eq!(presenter.dismiss(&tag), PlatformSupport::Performed);
        assert_eq!(presenter.dismissed(), vec![tag]);
    }

    #[test]
    fn dismissing_everything_is_counted_and_refused_when_blocked() {
        let presenter = InMemoryNotificationPresenter::new();
        assert_eq!(presenter.dismiss_all(), PlatformSupport::Performed);
        assert_eq!(presenter.dismissed_all(), 1);

        let blocked = InMemoryNotificationPresenter::blocked(DeliveryPrecondition::AppBundle);
        assert_eq!(blocked.dismiss_all(), PlatformSupport::Unsupported);
        assert_eq!(blocked.dismissed_all(), 0);
    }

    #[test]
    fn a_blocked_presenter_says_so_and_records_nothing() {
        let blocked = DeliveryPrecondition::AppBundle;
        let presenter = InMemoryNotificationPresenter::blocked(blocked);
        let notification = LocalNotification::new(NotificationTag::unique("e1"), "Ayan", "hi");

        assert_eq!(
            presenter.availability(),
            NotificationAvailability::Blocked(DeliveryPrecondition::AppBundle)
        );
        assert_eq!(
            presenter.present(&notification),
            PlatformSupport::Unsupported,
            "a caller must be able to fall back to in-app UI instead of \
             believing it told the user"
        );
        assert!(presenter.presented().is_empty());
    }
}
