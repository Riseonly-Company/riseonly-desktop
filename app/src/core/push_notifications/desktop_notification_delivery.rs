use gpui::{App, BorrowAppContext, Global};
use rise_i18n::tr;
use rise_notify::{PUSH_ACTIONS, PUSH_CATEGORIES, PushActionSpec, PushCategorySpec};
use rise_platform::PlatformSupport;
use rise_platform::app_info::AppIdentity;
use rise_platform::call_window::{IncomingCallResponse, call_notification_tag};
use rise_platform::notifications::{
    DeliveryPrecondition, GpuiNotificationPresenter, LocalNotification,
    NotificationAction, NotificationActionResponse, NotificationAuthorizationStatus,
    NotificationAvailability, NotificationPresenter, NotificationService, NotificationTag,
    authorization_status, on_notification_response, reply_input_support, request_authorization,
    take_notification_service,
};
use serde_json::{Map, Value};

use crate::app::dev_drive::DevDrive;
use crate::app::shell::handover;
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};

const TOPIC_SEPARATOR: char = ':';

const DELIVERED_MEMORY: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationCategory {
    pub identifier: String,
    pub actions: Vec<NotificationAction>,
}

fn category_action(spec: &PushActionSpec, translate: &dyn Fn(&str) -> String) -> NotificationAction {
    let action = NotificationAction::new(spec.identifier, translate(spec.title_i18n_key));
    match (spec.kind, spec.input_button_i18n_key, spec.input_placeholder_i18n_key) {
        ("text_input", Some(button), Some(placeholder)) => {
            action.with_text_input(translate(button), translate(placeholder))
        }
        _ => action,
    }
}

fn category(
    spec: &PushCategorySpec,
    translate: &dyn Fn(&str) -> String,
) -> NotificationCategory {
    NotificationCategory {
        identifier: spec.identifier.to_owned(),
        actions: PUSH_ACTIONS
            .iter()
            .filter(|action| action.category == spec.name)
            .map(|action| category_action(action, translate))
            .collect(),
    }
}

/// registers notification categories, labels go through translate
pub fn notification_categories(translate: &dyn Fn(&str) -> String) -> Vec<NotificationCategory> {
    PUSH_CATEGORIES
        .iter()
        .map(|spec| category(spec, translate))
        .collect()
}

/// one banner per chat and forum topic
/// an empty topic id is General so it shares the tag of a chat without topics
pub fn conversation_tag(chat_id: &str, topic_id: Option<&str>) -> NotificationTag {
    match topic_id.filter(|topic| !topic.is_empty()) {
        Some(topic) => {
            NotificationTag::conversation(&format!("{chat_id}{TOPIC_SEPARATOR}{topic}"))
        }
        None => NotificationTag::conversation(chat_id),
    }
}

/// notification of a chat, grouped by the chat id as thread
pub fn conversation_notification(
    chat_id: &str,
    topic_id: Option<&str>,
    title: impl Into<String>,
    body: impl Into<String>,
) -> LocalNotification {
    LocalNotification::new(conversation_tag(chat_id, topic_id), title, body).with_thread(chat_id)
}

fn conversation_parts(route: &str) -> (String, Option<String>) {
    match route.split_once(TOPIC_SEPARATOR) {
        Some((chat_id, topic_id)) => (chat_id.to_owned(), Some(topic_id.to_owned())),
        None => (route.to_owned(), None),
    }
}

fn conversation_route_of_raw(raw: &str) -> Option<&str> {
    let prefix = NotificationTag::conversation("");
    raw.strip_prefix(prefix.as_str())
}

/// action identifier and typed text of a notification response
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationResponseAction {
    Default,
    Reply { text: Option<String> },
    MarkRead,
}

impl NotificationResponseAction {
    /// an action without the foreground option runs in background
    /// so only what has to show a chat raises the window
    pub fn raises_window(&self) -> bool {
        matches!(self, Self::Default | Self::Reply { text: None })
    }
}

/// a notification response
/// chat id and topic id come from the tag so a click on an
/// old banner still knows its chat, user info is then empty
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationResponse {
    pub action: NotificationResponseAction,
    pub tag: String,
    pub chat_id: Option<String>,
    pub topic_id: Option<String>,
    pub user_info: Map<String, Value>,
}

pub type NotificationResponseHandler = fn(NotificationResponse, &mut App);

/// gets the call id from the tag and the pressed button, none for a click on the body
pub type IncomingCallResponseHandler = fn(&str, Option<IncomingCallResponse>, &mut App);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationDeliveryOutcome {
    Delivered,
    AppActive,
    Blocked(DeliveryPrecondition),
    Failed,
}

struct DeliveredNotification {
    tag: NotificationTag,
    chat_id: Option<String>,
    topic_id: Option<String>,
    user_info: Map<String, Value>,
}

/// the pure half, driven by any notification presenter
#[derive(Default)]
pub struct NotificationDeliveryState {
    categories: Vec<NotificationCategory>,
    delivered: Vec<DeliveredNotification>,
    response_handler: Option<NotificationResponseHandler>,
    incoming_call_response_handler: Option<IncomingCallResponseHandler>,
    pending_responses: Vec<NotificationResponse>,
    has_requested_permission: bool,
}

impl NotificationDeliveryState {
    pub fn register_categories(&mut self, categories: Vec<NotificationCategory>) {
        self.categories = categories;
    }

    pub fn categories(&self) -> &[NotificationCategory] {
        &self.categories
    }

    /// category is the content category id and its actions become the buttons
    pub fn deliver(
        &mut self,
        mut notification: LocalNotification,
        category: Option<&str>,
        user_info: Map<String, Value>,
        phase: ScenePhase,
        presenter: &dyn NotificationPresenter,
    ) -> NotificationDeliveryOutcome {
        if phase == ScenePhase::Active {
            return NotificationDeliveryOutcome::AppActive;
        }
        if let NotificationAvailability::Blocked(precondition) = presenter.availability() {
            return NotificationDeliveryOutcome::Blocked(precondition);
        }
        if notification.actions.is_empty()
            && let Some(category) = category
            && let Some(found) = self.categories.iter().find(|c| c.identifier == category)
        {
            notification.actions = found.actions.clone();
        }
        if !presenter.present(&notification).is_supported() {
            return NotificationDeliveryOutcome::Failed;
        }
        self.remember(&notification, user_info);
        NotificationDeliveryOutcome::Delivered
    }

    fn remember(&mut self, notification: &LocalNotification, user_info: Map<String, Value>) {
        let (chat_id, topic_id) = match notification.tag.conversation_route() {
            Some(route) => {
                let (chat_id, topic_id) = conversation_parts(route);
                (Some(chat_id), topic_id)
            }
            None => (notification.thread.clone(), None),
        };
        self.delivered.retain(|known| known.tag != notification.tag);
        if self.delivered.len() >= DELIVERED_MEMORY {
            self.delivered.remove(0);
        }
        self.delivered.push(DeliveredNotification {
            tag: notification.tag.clone(),
            chat_id,
            topic_id,
            user_info,
        });
    }

    /// removes delivered banners of a chat
    /// a topic id limits it to that topic with empty being General, none takes the whole chat
    pub fn remove_delivered(
        &mut self,
        chat_id: &str,
        topic_id: Option<&str>,
        presenter: &dyn NotificationPresenter,
    ) {
        if chat_id.is_empty() {
            return;
        }
        let matches = |known: &DeliveredNotification| {
            known.chat_id.as_deref() == Some(chat_id)
                && topic_id.is_none_or(|topic| known.topic_id.as_deref().unwrap_or("") == topic)
        };
        let mut tags: Vec<NotificationTag> = self
            .delivered
            .iter()
            .filter(|known| matches(*known))
            .map(|known| known.tag.clone())
            .collect();
        let own_tag = conversation_tag(chat_id, topic_id);
        if !tags.contains(&own_tag) {
            tags.push(own_tag);
        }
        self.delivered.retain(|known| !matches(known));
        for tag in &tags {
            presenter.dismiss(tag);
        }
    }

    /// removes all delivered banners when the account is cleared
    pub fn remove_all_delivered(&mut self, presenter: &dyn NotificationPresenter) {
        if presenter.dismiss_all().is_supported() {
            self.delivered.clear();
            return;
        }
        for known in self.delivered.drain(..) {
            presenter.dismiss(&known.tag);
        }
    }

    /// takes a response, an unknown action is dropped
    pub fn take_response(
        &mut self,
        response: &NotificationActionResponse,
    ) -> Option<NotificationResponse> {
        let action = match response.action_id.as_deref() {
            None => NotificationResponseAction::Default,
            Some(NotificationAction::REPLY_ID) => NotificationResponseAction::Reply {
                text: response.user_text.clone(),
            },
            Some(NotificationAction::MARK_READ_ID) => NotificationResponseAction::MarkRead,
            Some(_) => return None,
        };
        let raw: &str = &response.tag;
        let known = self
            .delivered
            .iter()
            .position(|known| known.tag.as_str() == raw)
            .map(|index| self.delivered.remove(index));
        let (chat_id, topic_id, user_info) = match known {
            Some(known) => (known.chat_id, known.topic_id, known.user_info),
            None => match conversation_route_of_raw(raw).map(conversation_parts) {
                Some((chat_id, topic_id)) => (Some(chat_id), topic_id, Map::new()),
                None => (None, None, Map::new()),
            },
        };
        Some(NotificationResponse {
            action,
            tag: raw.to_owned(),
            chat_id,
            topic_id,
            user_info,
        })
    }

    /// true once for the permission request, and only while the scene is active
    pub fn claim_permission_request(&mut self, phase: ScenePhase) -> bool {
        phase == ScenePhase::Active && !std::mem::replace(&mut self.has_requested_permission, true)
    }
}

/// the gpui half, one state for the app reached from the manager on the main thread
#[derive(Default)]
pub struct DesktopNotificationDelivery {
    state: NotificationDeliveryState,
    started: bool,
}

impl Global for DesktopNotificationDelivery {}

impl DesktopNotificationDelivery {
    /// run once at launch before the first post and first window
    /// windows drops toasts without an AUMID and linux posts wait for the bus service
    pub fn start(
        identity: &AppIdentity,
        spawn_service: impl FnOnce(NotificationService),
        cx: &mut App,
    ) {
        let first = cx.update_default_global::<Self, _>(|this, _| {
            !std::mem::replace(&mut this.started, true)
        });
        if !first {
            return;
        }
        cx.set_app_identity(identity.bundle_identifier, identity.display_name);
        if let Some(service) = take_notification_service(identity) {
            spawn_service(service);
        }
        on_notification_response(cx, Self::receive);
        Self::register_categories(cx);
    }

    /// registers the notification categories
    pub fn register_categories(cx: &mut App) {
        let categories = notification_categories(&tr);
        cx.update_default_global::<Self, _>(|this, _| {
            this.state.register_categories(categories);
        });
    }

    pub fn reply_input_support() -> PlatformSupport {
        reply_input_support()
    }

    pub fn deliver(
        notification: LocalNotification,
        category: Option<&str>,
        user_info: Map<String, Value>,
        cx: &mut App,
    ) -> NotificationDeliveryOutcome {
        if DevDrive::is_stealth(cx) {
            tracing::info!(
                target: "riseonly::push",
                title = %notification.title,
                body = %notification.body,
                thread = ?notification.thread,
                category = ?category,
                sender = ?notification.sender,
                avatar = ?notification.avatar,
                attachment = ?notification.attachment,
                "stealth push"
            );
            if !rise_platform::notifications::uses_tester_receiver() {
                return NotificationDeliveryOutcome::Failed;
            }
            rise_platform::notifications::note_tester_category(notification.tag.as_str(), category);
        }
        let phase = RiseAppLifecycle::scene_phase(cx);
        cx.update_default_global::<Self, _>(|this, cx| {
            let presenter = GpuiNotificationPresenter::new(cx);
            this.state
                .deliver(notification, category, user_info, phase, &presenter)
        })
    }

    pub fn remove_delivered(chat_id: &str, topic_id: Option<&str>, cx: &mut App) {
        cx.update_default_global::<Self, _>(|this, cx| {
            let presenter = GpuiNotificationPresenter::new(cx);
            this.state.remove_delivered(chat_id, topic_id, &presenter);
        });
    }

    pub fn remove_all_delivered(cx: &mut App) {
        cx.update_default_global::<Self, _>(|this, cx| {
            let presenter = GpuiNotificationPresenter::new(cx);
            this.state.remove_all_delivered(&presenter);
        });
    }

    pub fn set_response_handler(handler: NotificationResponseHandler, cx: &mut App) {
        let pending = cx.update_default_global::<Self, _>(|this, _| {
            this.state.response_handler = Some(handler);
            std::mem::take(&mut this.state.pending_responses)
        });
        for response in pending {
            handler(response, cx);
        }
    }

    pub fn set_incoming_call_response_handler(handler: IncomingCallResponseHandler, cx: &mut App) {
        cx.update_default_global::<Self, _>(|this, _| {
            this.state.incoming_call_response_handler = Some(handler);
        });
    }

    fn receive(response: NotificationActionResponse, cx: &mut App) {
        let call_tag_prefix = call_notification_tag("");
        if let Some(call_id) = response.tag.strip_prefix(call_tag_prefix.as_str()) {
            let answer = response.action_id.as_deref().and_then(IncomingCallResponse::from_action_id);
            if let Some(handler) = cx.default_global::<Self>().state.incoming_call_response_handler {
                handler(call_id, answer, cx);
            }
            return;
        }
        let parsed = cx.update_default_global::<Self, _>(|this, _| {
            this.state.take_response(&response)
        });
        let Some(parsed) = parsed else {
            return;
        };
        if parsed.action.raises_window() {
            handover::raise_app(cx);
        }
        let handler = cx.default_global::<Self>().state.response_handler;
        match handler {
            Some(handler) => handler(parsed, cx),
            None => cx.default_global::<Self>().state.pending_responses.push(parsed),
        }
    }

    /// asks for permission if needed, a host with no per app consent answers Unsupported
    pub fn request_permission_if_needed(cx: &mut App) {
        if DevDrive::is_stealth(cx) {
            return;
        }
        let phase = RiseAppLifecycle::scene_phase(cx);
        let claimed = cx.update_default_global::<Self, _>(|this, _| {
            this.state.claim_permission_request(phase)
        });
        if claimed {
            let supported = request_authorization().is_supported();
            tracing::debug!(target: "riseonly", supported, "notification permission requested");
        }
    }
}

/// true if the system denied notifications
/// needs no gpui since the manager awaits it on its own task
pub async fn system_authorization_is_denied() -> bool {
    authorization_status().await == NotificationAuthorizationStatus::Denied
}

#[cfg(test)]
#[path = "desktop_notification_delivery_tests.rs"]
mod tests;
