use std::pin::pin;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, AsyncApp, Context, Entity, Task};
use rise_canon::rise_key::{chat_action, music as music_key, push, push_route};
use rise_canon::rise_wire_methods;
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_engine::rise_wire::rise_wire::RiseWireSocketSession;
use rise_notify::{
    NOTIFICATION_SCOPE_KEYS, NOTIFICATION_SCOPES, PAYLOAD_TYPE_KEYS, SELF_AUTHORED_ACTOR_ID_KEYS,
    SELF_AUTHORED_FOLLOW_PAYLOAD_TYPES, mute_is_active,
};
use serde_json::{Map, Value};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SAI_WS_DID_CONNECT;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::sai::Sai;
use crate::core::push_notifications::desktop_notification_delivery::{
    DesktopNotificationDelivery, NotificationResponse, NotificationResponseAction,
    system_authorization_is_denied,
};
use crate::core::push_notifications::notification_service_extension::notification_service::NotificationService;
use crate::core::push_notifications::push_deep_link_store::{PushDeepLink, PushDeepLinkStore};
use crate::core::push_notifications::push_notification_presentation::{
    PushForegroundNotificationPresentation, PushNotificationRouteParser,
};
use crate::core::push_notifications::push_ownership_generation::{
    PushOwnershipGenerationPolicy, PushOwnershipGenerationStore, PushOwnershipRegistrationContext,
};
use crate::core::push_notifications::rise_app_icon_badge::RiseAppIconBadge;
use crate::core::rise_engine_bridge::rise_canon_endpoints::{
    RiseCanonEndpoints, RiseEndpointClientTimeout,
};
use crate::core::rise_engine_bridge::rise_engine_app_runtime::{
    RISE_ENGINE_CONNECTION_READY, RiseEngineAppRuntime,
};
use crate::core::widgets::notifier::notifier_store::{
    NotifierAction, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::auth::services::auth_local_storage::AuthLocalStorage;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatMuteScopes;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::moderation::stores::moderation_services::moderation_services::{
    ModerationServicesStore,
};
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::notify::stores::notify_actions::notify_wire_types::RiseAPNSRegistrationTarget;
use crate::modules::notify::stores::notify_interactions::notify_interactions::NotifyInteractionsStore;
use crate::modules::notify::stores::notify_services::notify_services::NotifyServicesStore;

const FOREGROUND_BANNER_DURATION: TimeInterval = 5.2;
const CHAT_BANNER_QUIET_INTERVAL: f64 = 10.0;
const LOG_NAME: &str = "APNS";

/// payload value as a trimmed string or number, empty counts as absent
fn normalized(value: Option<&Value>) -> Option<String> {
    let raw = match value? {
        Value::String(string) => string.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => (if *flag { "1" } else { "0" }).to_owned(),
        Value::Null | Value::Array(_) | Value::Object(_) => return None,
    };
    let result = trimming_whitespaces_and_newlines(&raw);
    (!result.is_empty()).then(|| result.to_owned())
}

fn payload_string(user_info: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| normalized(user_info.get(*key)))
}

pub enum PushRecipientAccountGate {}

impl PushRecipientAccountGate {
    const RECIPIENT_KEYS: [&str; 1] = [push::RECIPIENT_USER_ID];

    pub fn should_handle(
        user_info: &Map<String, Value>,
        active_user_id: Option<&str>,
        stored_account_count: i64,
    ) -> bool {
        let Some(active_user_id) = active_user_id
            .map(trimming_whitespaces_and_newlines)
            .filter(|active| !active.is_empty())
        else {
            return false;
        };

        for key in Self::RECIPIENT_KEYS {
            if let Some(value) = user_info.get(key) {
                return normalized(Some(value)).as_deref() == Some(active_user_id);
            }
        }

        stored_account_count == 1
    }
}

/// chat area that can be muted on its own
/// spellings are canon notification scopes, the server puts exactly those in the payload
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushChatNotificationScope {
    Messages,
    Mentions,
    Replies,
    Reactions,
    Events,
}

impl PushChatNotificationScope {
    const ALL: [Self; 5] = [Self::Messages, Self::Mentions, Self::Replies, Self::Reactions, Self::Events];

    pub fn from_raw_value(raw_value: &str) -> Option<Self> {
        NOTIFICATION_SCOPES
            .iter()
            .position(|scope| *scope == raw_value)
            .and_then(|index| Self::ALL.get(index).copied())
    }

    pub fn is_enabled(self, scopes: &ChatMuteScopes) -> bool {
        match self {
            Self::Messages => scopes.messages,
            Self::Mentions => scopes.mentions,
            Self::Replies => scopes.replies,
            Self::Reactions => scopes.reactions,
            Self::Events => scopes.events,
        }
    }
}

const _: () = assert!(NOTIFICATION_SCOPES.len() == PushChatNotificationScope::ALL.len());

pub enum PushChatNotificationPolicy {}

impl PushChatNotificationPolicy {
    pub fn allows(scope: Option<&str>, scopes: Option<&ChatMuteScopes>) -> bool {
        let (Some(scope), Some(scopes)) = (scope, scopes) else {
            return true;
        };
        let scope = trimming_whitespaces_and_newlines(scope).to_lowercase();
        PushChatNotificationScope::from_raw_value(&scope)
            .is_none_or(|scope| scope.is_enabled(scopes))
    }
}

pub struct PushNotificationManager {
    pending_device_token: Option<String>,
    device_token: Option<String>,
    registered_user_id: Option<String>,
    system_authorization_denied: bool,
    last_confirmed_registration_key: Option<String>,
    ownership_context: Option<PushOwnershipRegistrationContext>,
    connection_observer: Option<RiseSubscription>,
    connection_delivery: Option<Task<()>>,
    registration_task: Option<Task<()>>,
    registration_id: Option<Uuid>,
    last_attempt: Option<(String, RiseWireSocketSession)>,
    gift_toast: Option<(String, f64)>,
}

impl PushNotificationManager {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            pending_device_token: None,
            device_token: None,
            registered_user_id: None,
            system_authorization_denied: false,
            last_confirmed_registration_key: None,
            ownership_context: None,
            connection_observer: None,
            connection_delivery: None,
            registration_task: None,
            registration_id: None,
            last_attempt: None,
            gift_toast: None,
        }
    }

    pub fn device_token(&self) -> Option<&str> {
        self.device_token.as_deref()
    }

    pub fn registered_user_id(&self) -> Option<&str> {
        self.registered_user_id.as_deref()
    }

    pub fn system_authorization_denied(&self) -> bool {
        self.system_authorization_denied
    }

    fn registration_key(user_id: &str, token: &str) -> String {
        format!("{user_id}|{token}")
    }

    pub fn setup(&mut self, cx: &mut Context<Self>) {
        cx.defer(|cx| DesktopNotificationDelivery::set_response_handler(Self::did_receive, cx));
        if self.connection_observer.is_none() {
            let (sender, mut receiver) = mpsc::unbounded::<()>();
            self.connection_observer = Some(RiseAppEvents::default_center().observe(
                RISE_ENGINE_CONNECTION_READY,
                move |_| {
                    let _ = sender.unbounded_send(());
                },
            ));
            self.connection_delivery = Some(cx.spawn(async move |this, cx| {
                while receiver.next().await.is_some() {
                    let delivered =
                        this.update(cx, |this, cx| this.register_if_authenticated(cx));
                    if delivered.is_err() {
                        return;
                    }
                }
            }));
        }
        DesktopNotificationDelivery::register_categories(cx);
        RiseAppIconBadge::start(cx);
    }

    pub fn request_permission_if_needed(cx: &mut App) {
        DesktopNotificationDelivery::request_permission_if_needed(cx);
    }

    /// asks the system again, notifications can be turned off in
    /// settings anytime so the settings screen calls it whenever it shows
    pub fn refresh_system_authorization(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let denied = system_authorization_is_denied().await;
            let _ = this.update(cx, |this, cx| {
                this.system_authorization_denied = denied;
                cx.notify();
                NotifyServicesStore::shared(cx).update(cx, |_, cx| cx.notify());
            });
        })
        .detach();
    }

    pub fn handle_device_token(&mut self, token_data: &[u8], cx: &mut Context<Self>) {
        let token: String = token_data.iter().map(|byte| format!("{byte:02x}")).collect();
        let prefix: String = token.chars().take(8).collect();
        Sai::log(
            LOG_NAME,
            move || format!("received device token prefix={prefix}"),
            SaiLogKind::Network,
            cx,
        );
        self.device_token = Some(token.clone());
        self.pending_device_token = Some(token);
        cx.notify();
        self.register_if_authenticated(cx);
    }

    pub fn register_if_authenticated(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.pending_device_token.clone() else {
            return;
        };
        let (is_authenticated, user_id) = {
            let auth = AuthServicesStore::shared(cx);
            let auth = auth.read(cx);
            (auth.is_authenticated(), auth.effective_user_id_for_wire())
        };
        let Some(user_id) = user_id.filter(|user_id| is_authenticated && !user_id.is_empty()) else {
            return;
        };
        let Some(epoch) = SaiAccountCacheScope::active_epoch() else {
            return;
        };
        let Some(context) = self.ownership_registration_context(&user_id, cx) else {
            return;
        };
        let key = format!(
            "{}|{epoch}|{}|{}|{}",
            Self::registration_key(&user_id, &token),
            context.generation,
            RiseAPNSRegistrationTarget::bundle_id(),
            RiseAPNSRegistrationTarget::environment(),
        );
        if self.last_confirmed_registration_key.as_ref() == Some(&key)
            || self.registration_task.is_some()
        {
            return;
        }
        let identifier = Uuid::new_v4();
        self.registration_id = Some(identifier);
        self.registration_task = Some(cx.spawn(async move |this, cx| {
            Self::register(&this, identifier, &key, &token, &user_id, &epoch, &context, cx).await;
            let _ = this.update(cx, |this, cx| {
                this.registration_did_finish(identifier, &token, &user_id, cx);
            });
        }));
    }

    #[allow(clippy::too_many_arguments)]
    async fn register(
        this: &gpui::WeakEntity<Self>,
        identifier: Uuid,
        key: &str,
        token: &str,
        user_id: &str,
        epoch: &str,
        context: &PushOwnershipRegistrationContext,
        cx: &mut AsyncApp,
    ) {
        let is_current = |this: &Self| {
            this.registration_id == Some(identifier)
                && this.pending_device_token.as_deref() == Some(token)
                && SaiAccountCacheScope::is_current_account_epoch(Some(user_id), Some(epoch))
        };
        let connection = Self::ready_connection(user_id, cx).await;
        let Some(connection) = connection else {
            return;
        };
        let should_send = this.update(cx, |this, _| {
            let already_tried = this
                .last_attempt
                .as_ref()
                .is_some_and(|(last_key, last)| last_key == key && *last == connection);
            if !is_current(this) || already_tried {
                return false;
            }
            this.last_attempt = Some((key.to_owned(), connection.clone()));
            true
        });
        if !matches!(should_send, Ok(true)) {
            return;
        }
        let request = cx.update(|cx| {
            NotifyInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.register_device_token(
                    token.to_owned(),
                    &context.user_id,
                    context.generation,
                    cx,
                )
            })
        });
        match request.await {
            Ok(()) => {
                let _ = this.update(cx, |this, cx| {
                    if is_current(this) {
                        this.last_confirmed_registration_key = Some(key.to_owned());
                        this.registered_user_id = Some(user_id.to_owned());
                        cx.notify();
                    }
                });
            }
            Err(error) => cx.update(|cx| {
                NotifyServicesStore::shared(cx)
                    .update(cx, |store, cx| store.handle_register_token_error(&error, cx));
            }),
        }
    }

    fn registration_did_finish(
        &mut self,
        identifier: Uuid,
        token: &str,
        user_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.registration_id != Some(identifier) {
            return;
        }
        if let Some(task) = self.registration_task.take() {
            task.detach();
        }
        self.registration_id = None;
        if self.pending_device_token.as_deref() != Some(token) {
            self.register_if_authenticated(cx);
            return;
        }
        let Some(previous) = self.last_attempt.as_ref().map(|(_, connection)| connection.clone())
        else {
            return;
        };
        let user_id = user_id.to_owned();
        cx.spawn(async move |this, cx| {
            let current = Self::ready_connection(&user_id, cx).await;
            if current.is_some_and(|current| current != previous) {
                let _ = this.update(cx, |this, cx| this.register_if_authenticated(cx));
            }
        })
        .detach();
    }

    async fn ready_connection(user_id: &str, cx: &mut AsyncApp) -> Option<RiseWireSocketSession> {
        let connection = cx.update(|cx| {
            RiseEngineAppRuntime::shared(cx)
                .update(cx, |runtime, cx| runtime.ready_connection(user_id, cx))
        });
        connection.await
    }

    fn cancel_registration(&mut self) {
        self.registration_task = None;
        self.registration_id = None;
        self.last_attempt = None;
    }

    pub fn activate_for_authenticated_account(&mut self, cx: &mut Context<Self>) {
        if !AuthServicesStore::shared(cx).read(cx).is_authenticated() {
            return;
        }
        self.register_if_authenticated(cx);
    }

    pub fn prepare_for_account_switch(&mut self, cx: &mut Context<Self>) {
        self.cancel_registration();
        self.registered_user_id = None;
        self.last_confirmed_registration_key = None;
        self.pending_device_token = self.device_token.clone();
        cx.notify();
        DesktopNotificationDelivery::remove_all_delivered(cx);
        RiseAppIconBadge::clear(cx);
        self.register_if_authenticated(cx);
    }

    pub fn handle_account_cleared(&mut self, cx: &mut Context<Self>) {
        self.cancel_registration();
        self.registered_user_id = None;
        self.last_confirmed_registration_key = None;
        self.ownership_context = None;
        self.pending_device_token = None;
        self.device_token = None;
        cx.notify();
        DesktopNotificationDelivery::remove_all_delivered(cx);
        RiseAppIconBadge::clear(cx);
        cx.background_spawn(async { NotificationService::clear_avatar_cache() }).detach();
    }

    /// opening a chat clears its delivered banners from the notification center
    /// a topic id (empty for General) limits it to that forum topic
    pub fn remove_delivered_notifications(chat_id: &str, topic_id: Option<&str>, cx: &mut App) {
        DesktopNotificationDelivery::remove_delivered(chat_id, topic_id, cx);
    }

    fn ownership_registration_context(
        &mut self,
        user_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<PushOwnershipRegistrationContext> {
        let Some(decision) = PushOwnershipGenerationPolicy::registration(
            user_id,
            self.ownership_context.as_ref(),
            PushOwnershipGenerationStore::load_high_water(),
            RiseClock::now().ms(),
        ) else {
            Sai::log(
                LOG_NAME,
                || "ownership generation exhausted or user is invalid".to_owned(),
                SaiLogKind::Network,
                cx,
            );
            return None;
        };

        if let Some(high_water) = decision.high_water_to_persist
            && let Err(error) = PushOwnershipGenerationStore::persist_high_water(high_water)
        {
            let description = error.to_string();
            Sai::log(
                LOG_NAME,
                move || format!("ownership generation persistence failed: {description}"),
                SaiLogKind::Network,
                cx,
            );
            return None;
        }

        self.ownership_context = Some(decision.context.clone());
        Some(decision.context)
    }

    pub fn should_handle_notification(user_info: &Map<String, Value>, cx: &mut App) -> bool {
        let active_user_id = AuthServicesStore::shared(cx)
            .read(cx)
            .active_account_id()
            .map(str::to_owned)
            .or_else(AuthLocalStorage::active_account_id);
        let stored_account_count = AuthLocalStorage::load_accounts().len();
        PushRecipientAccountGate::should_handle(
            user_info,
            active_user_id.as_deref(),
            i64::try_from(stored_account_count).unwrap_or(i64::MAX),
        )
    }

    pub fn will_present(
        &mut self,
        user_info: &Map<String, Value>,
        fallback_title: &str,
        fallback_body: &str,
        cx: &mut Context<Self>,
    ) {
        if !Self::should_handle_notification(user_info, cx)
            || Self::is_self_friend_notification(user_info, cx)
        {
            return;
        }
        self.present_foreground_notification(user_info, fallback_title, fallback_body, cx);
    }

    /// suppresses the account own follow event
    /// canon declares both the type list and the actor key order as the policy
    pub fn is_self_friend_notification(user_info: &Map<String, Value>, cx: &mut App) -> bool {
        let type_ = PAYLOAD_TYPE_KEYS
            .iter()
            .find_map(|key| user_info.get(*key).and_then(Value::as_str))
            .unwrap_or("");
        if !SELF_AUTHORED_FOLLOW_PAYLOAD_TYPES.contains(&type_) {
            return false;
        }
        let actor_id = SELF_AUTHORED_ACTOR_ID_KEYS
            .iter()
            .filter_map(|key| user_info.get(*key).and_then(Value::as_str))
            .find(|actor_id| !actor_id.is_empty());
        let Some(actor_id) = actor_id else {
            return false;
        };
        AuthServicesStore::shared(cx).read(cx).is_current_user(actor_id)
    }

    pub fn did_receive(response: NotificationResponse, cx: &mut App) {
        if !Self::should_handle_notification(&response.user_info, cx) {
            return;
        }
        match response.action.clone() {
            NotificationResponseAction::Default | NotificationResponseAction::Reply { text: None } => {
                Self::handle_notification_tap(&response, cx);
            }
            NotificationResponseAction::Reply { text: Some(text) } => {
                Self::handle_notification_reply(&response, &text, cx);
            }
            NotificationResponseAction::MarkRead => {
                cx.spawn(async move |cx: &mut AsyncApp| {
                    Self::handle_notification_mark_read(&response, cx).await;
                })
                .detach();
            }
        }
    }

    fn handle_notification_tap(response: &NotificationResponse, cx: &mut App) {
        let user_info = &response.user_info;
        let type_ = user_info
            .get(push_route::TYPE)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let logged_type = type_.clone();
        Sai::log(
            LOG_NAME,
            move || format!("notification tapped type={logged_type}"),
            SaiLogKind::Network,
            cx,
        );

        if Self::is_self_friend_notification(user_info, cx) {
            return;
        }
        let route = if user_info.is_empty() {
            Self::conversation_route(response)
        } else {
            PushNotificationRouteParser::route(user_info)
        };
        let Some(route) = route else {
            Sai::log(
                LOG_NAME,
                move || format!("unhandled notification type={type_}"),
                SaiLogKind::Network,
                cx,
            );
            return;
        };
        let wants_composer = matches!(response.action, NotificationResponseAction::Reply { text: None });
        PushDeepLinkStore::shared(cx).update(cx, |store, cx| {
            if wants_composer {
                store.enqueue_for_composer(route, cx);
            } else {
                store.enqueue(route, cx);
            }
        });
    }

    fn conversation_route(response: &NotificationResponse) -> Option<PushDeepLink> {
        let chat_id = response.chat_id.clone().filter(|chat_id| !chat_id.is_empty())?;
        Some(match response.topic_id.clone() {
            Some(topic_id) => PushDeepLink::ChatTopic { chat_id, topic_id },
            None => PushDeepLink::Chat { chat_id },
        })
    }

    fn response_chat_id(response: &NotificationResponse) -> Option<String> {
        payload_string(&response.user_info, &[push_route::CHAT_ID])
            .or_else(|| response.chat_id.clone().filter(|chat_id| !chat_id.is_empty()))
    }

    fn present_foreground_notification(
        &mut self,
        user_info: &Map<String, Value>,
        fallback_title: &str,
        fallback_body: &str,
        cx: &mut Context<Self>,
    ) {
        if !Self::should_present_for_chat_notification_settings(user_info, cx) {
            return;
        }
        let Some(presentation) = PushForegroundNotificationPresentation::make(
            user_info,
            fallback_title,
            fallback_body,
            cx,
        ) else {
            return;
        };
        if !self.admits_chat_banner(user_info, cx)
            || !self.admits_gift_banner(user_info)
            || !Self::admits_listening_permissions_banner(user_info, cx)
        {
            return;
        }

        let replace_key = payload_string(user_info, &[push_route::CHAT_ID]).map(|chat_id| format!("chat:{chat_id}"));
        let title = presentation.title;
        let options = NotifierSendOptions {
            content: presentation.body,
            r#type: NotifierType::Info,
            position: NotifierPosition::Top,
            icon: NotifierIcon::Avatar {
                source: presentation.avatar_url,
                name: presentation.avatar_name,
                user_id: presentation.avatar_user_id,
            },
            action: presentation.route.map(NotifierAction::DeepLink),
            duration: FOREGROUND_BANNER_DURATION,
            content_is_markup: true,
            author: presentation.author,
            replace_key,
            is_notification: true,
            ..NotifierSendOptions::default()
        };
        ModerationServicesStore::send_after_sanction_toast(title, options, cx);
    }

    pub fn note_gift_toast(&mut self, instance_id: String) {
        self.gift_toast = Some((instance_id, RiseClock::monotonic().as_secs_f64()));
    }

    fn admits_listening_permissions_banner(user_info: &Map<String, Value>, cx: &mut App) -> bool {
        if payload_string(user_info, &[push_route::TYPE]).as_deref()
            != Some(rise_notification_texts::payload::PLAYLIST_SESSION_PERMISSIONS_UPDATED)
        {
            return true;
        }
        let Some(session_id) = payload_string(user_info, &[music_key::SESSION_ID]) else {
            return true;
        };
        let state = MusicPlayerState::shared(cx);
        state
            .read(cx)
            .listening_session
            .as_ref()
            .is_none_or(|session| session.session_id != session_id)
    }

    pub(crate) fn admits_gift_banner(&self, user_info: &Map<String, Value>) -> bool {
        if payload_string(user_info, &[push_route::TYPE]).as_deref() != Some(rise_events::name::GIFT_RECEIVED) {
            return true;
        }
        let Some(instance_id) = payload_string(user_info, &[chat_action::INSTANCE_ID]) else {
            return true;
        };
        let now = RiseClock::monotonic().as_secs_f64();
        !matches!(
            self.gift_toast.as_ref(),
            Some((shown, uptime)) if *shown == instance_id && now - uptime < CHAT_BANNER_QUIET_INTERVAL
        )
    }

    fn admits_chat_banner(&self, user_info: &Map<String, Value>, cx: &mut Context<Self>) -> bool {
        let Some(chat_id) = payload_string(user_info, &[push_route::CHAT_ID]) else {
            return true;
        };
        let now = RiseClock::monotonic().as_secs_f64();
        let services = ChatServicesStore::shared(cx);
        if let Some(removed) = services.read(cx).removed_from_chat_notice.as_ref()
            && removed.chat_id == chat_id
            && now - removed.uptime < CHAT_BANNER_QUIET_INTERVAL
        {
            return false;
        }
        true
    }

    pub fn should_present_for_chat_notification_settings(
        user_info: &Map<String, Value>,
        cx: &mut App,
    ) -> bool {
        let (Some(chat_id), Some(scope)) = (
            payload_string(user_info, &[push_route::CHAT_ID]),
            payload_string(user_info, &NOTIFICATION_SCOPE_KEYS),
        ) else {
            return true;
        };

        let services = ChatServicesStore::shared(cx);
        let chat = services
            .read(cx)
            .current_chat_infos(cx)
            .into_iter()
            .find(|chat| chat.id == chat_id);
        if let Some(mute_until) = chat.as_ref().and_then(|chat| chat.mute_until)
            && mute_is_active(Some(mute_until as i64), RiseClock::now().ms())
        {
            return false;
        }

        let scopes = match payload_string(user_info, &[push_route::TOPIC_ID]) {
            Some(topic_id) => {
                let topic = services
                    .update(cx, |store, cx| store.topic_from_any_cache(&chat_id, &topic_id, cx));
                let Some(topic) = topic else {
                    return true;
                };
                Some(services.read(cx).effective_topic_notification_scopes(&chat_id, &topic, cx))
            }
            None => chat.as_ref().map(|chat| {
                services.read(cx).effective_notification_scopes_for_chat(
                    &chat_id,
                    chat.notification_scopes(),
                    cx,
                )
            }),
        };

        PushChatNotificationPolicy::allows(Some(scope.as_str()), scopes.as_ref())
    }

    async fn handle_notification_mark_read(response: &NotificationResponse, cx: &mut AsyncApp) {
        let Some(chat_id) = Self::response_chat_id(response) else {
            return;
        };
        let session = cx.update(|cx| {
            let auth = AuthServicesStore::shared(cx);
            let auth = auth.read(cx);
            let user_id = auth
                .effective_user_id_for_wire()
                .or_else(|| auth.profile().map(|profile| profile.id.clone()))
                .filter(|user_id| !user_id.is_empty());
            user_id.map(|user_id| (user_id, auth.tokens_and_other_data().access_token.clone()))
        });
        let Some((user_id, access_token)) = session else {
            return;
        };
        let topic_id = payload_string(&response.user_info, &[push_route::TOPIC_ID])
            .or_else(|| response.topic_id.clone());
        let transport_owner = format!("push.mark_read.{}", Uuid::new_v4().uuid_string());
        cx.update(|cx| {
            SaiWs::set_transport_hold(&transport_owner, true, cx);
            SaiWs::connect_if_needed(Some(access_token.as_str()), cx);
        });

        let (sender, mut receiver) = mpsc::unbounded::<()>();
        let _observers = [SAI_WS_DID_CONNECT, RISE_ENGINE_CONNECTION_READY].map(|name| {
            let sender = sender.clone();
            RiseAppEvents::default_center().observe(name, move |_| {
                let _ = sender.unbounded_send(());
            })
        });
        let timeout: Duration = RiseCanonEndpoints::required(
            rise_wire_methods::message::SERVICE,
            rise_wire_methods::message::MARK_CHAT_AS_READ,
        )
        .client_timeout();
        let deadline = RiseClock::monotonic() + timeout;
        loop {
            if Self::ready_connection(&user_id, cx).await.is_some() {
                break;
            }
            let remaining = deadline.saturating_sub(RiseClock::monotonic());
            if remaining.is_zero() {
                break;
            }
            let timer = cx.background_executor().timer(remaining);
            futures::future::select(receiver.next(), pin!(timer)).await;
        }

        let logged_chat_id = chat_id.clone();
        let read = cx.update(|cx| {
            Sai::log(
                LOG_NAME,
                move || format!("mark read chat={logged_chat_id}"),
                SaiLogKind::Network,
                cx,
            );
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.mark_chat_as_read(&chat_id, topic_id.as_deref(), false, cx)
            })
        });
        if let Some(read) = read {
            read.await;
        }
        cx.update(|cx| SaiWs::set_transport_hold(&transport_owner, false, cx));
    }

    fn handle_notification_reply(response: &NotificationResponse, reply_text: &str, cx: &mut App) {
        let Some(chat_id) = Self::response_chat_id(response) else {
            return;
        };
        if trimming_whitespaces_and_newlines(reply_text).is_empty() {
            return;
        }
        let logged_chat_id = chat_id.clone();
        Sai::log(
            LOG_NAME,
            move || format!("quick reply chat={logged_chat_id}"),
            SaiLogKind::Network,
            cx,
        );
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.send_text(
                &chat_id, reply_text, None, None, None, None, None, None, None, None, None, None, cx,
            );
        });
    }
}

#[cfg(test)]
#[path = "push_notification_manager_tests.rs"]
mod tests;
