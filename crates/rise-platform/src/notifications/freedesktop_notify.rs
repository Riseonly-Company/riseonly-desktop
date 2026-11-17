use std::collections::HashMap;

use crate::notifications::{LocalNotification, NotificationTag};

pub const NOTIFICATIONS_SERVICE: &str = "org.freedesktop.Notifications";
pub const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
pub const NOTIFICATIONS_INTERFACE: &str = "org.freedesktop.Notifications";
pub const NOTIFY_METHOD: &str = "Notify";
pub const CLOSE_NOTIFICATION_METHOD: &str = "CloseNotification";
pub const GET_CAPABILITIES_METHOD: &str = "GetCapabilities";
pub const ACTION_INVOKED_SIGNAL: &str = "ActionInvoked";
pub const NOTIFICATION_CLOSED_SIGNAL: &str = "NotificationClosed";
pub const NOTIFICATION_REPLIED_SIGNAL: &str = "NotificationReplied";
pub const INLINE_REPLY_ACTION_KEY: &str = "inline-reply";
pub const REPLY_PLACEHOLDER_HINT: &str = "x-kde-reply-placeholder-text";
pub const REPLY_SUBMIT_BUTTON_HINT: &str = "x-kde-reply-submit-button-text";

pub const DEFAULT_ACTION_KEY: &str = "default";
pub const CATEGORY_IM_RECEIVED: &str = "im.received";
pub const EXPIRE_BY_SERVER: i32 = -1;

const TRANSPORT_ACTION_PREFIX: &str = "rise-action-";

/// capability strings from the spec that this route acts on
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ServerCapabilities {
    pub actions: bool,
    pub body_markup: bool,
    pub persistence: bool,
    pub inline_reply: bool,
}

impl ServerCapabilities {
    pub fn from_list<S: AsRef<str>>(capabilities: &[S]) -> Self {
        let has = |name: &str| capabilities.iter().any(|c| c.as_ref() == name);
        Self {
            actions: has("actions"),
            body_markup: has("body-markup"),
            persistence: has("persistence"),
            inline_reply: has("inline-reply"),
        }
    }
}

/// one hint value, the bus layer turns it into a dbus variant
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HintValue {
    Text(String),
    Flag(bool),
}

/// arguments of one Notify call without app_name and app_icon, the route owns those
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NotifyRequest {
    pub replaces_id: u32,
    pub summary: String,
    pub body: String,
    pub actions: Vec<String>,
    pub hints: Vec<(&'static str, HintValue)>,
    pub expire_timeout: i32,
}

/// key a button travels under
/// caller ids never go on the wire so an id of default cant pose as a body click
pub fn transport_action_key(index: usize) -> String {
    format!("{TRANSPORT_ACTION_PREFIX}{index}")
}

/// escapes the five characters body markup servers parse
pub fn escape_markup(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

pub fn image_path_uri(path: &std::path::Path) -> Option<String> {
    let text = path.to_str()?;
    if !path.is_absolute() {
        return None;
    }
    Some(crate::photo_library::xdg_library::file_uri(text))
}

fn inline_reply_index(
    notification: &LocalNotification,
    capabilities: &ServerCapabilities,
) -> Option<usize> {
    if !capabilities.actions || !capabilities.inline_reply {
        return None;
    }
    notification
        .actions
        .iter()
        .position(|action| action.text_input.is_some())
}

/// builds the Notify arguments for one post
/// desktop_entry is the .desktop name without suffix, gnome and kde use it for icon and settings
pub fn notify_request(
    notification: &LocalNotification,
    replaces_id: u32,
    capabilities: &ServerCapabilities,
    desktop_entry: Option<&str>,
) -> NotifyRequest {
    let body = if capabilities.body_markup {
        escape_markup(&notification.body)
    } else {
        notification.body.clone()
    };

    let inline_reply = inline_reply_index(notification, capabilities);
    let mut actions = Vec::new();
    if capabilities.actions {
        actions.push(DEFAULT_ACTION_KEY.to_owned());
        actions.push(String::new());
        for (index, action) in notification.actions.iter().enumerate() {
            actions.push(if inline_reply == Some(index) {
                INLINE_REPLY_ACTION_KEY.to_owned()
            } else {
                transport_action_key(index)
            });
            actions.push(action.label.clone());
        }
    }

    let mut hints = Vec::new();
    if let Some(input) = inline_reply.and_then(|index| notification.actions[index].text_input.as_ref()) {
        if !input.placeholder.is_empty() {
            hints.push((REPLY_PLACEHOLDER_HINT, HintValue::Text(input.placeholder.clone())));
        }
        if !input.button_title.is_empty() {
            hints.push((REPLY_SUBMIT_BUTTON_HINT, HintValue::Text(input.button_title.clone())));
        }
    }
    if notification.tag.conversation_route().is_some() {
        hints.push(("category", HintValue::Text(CATEGORY_IM_RECEIVED.to_owned())));
    }
    if let Some(uri) = notification.avatar.as_deref().and_then(image_path_uri) {
        hints.push(("image-path", HintValue::Text(uri)));
    }
    if let Some(entry) = desktop_entry.filter(|entry| !entry.is_empty()) {
        hints.push(("desktop-entry", HintValue::Text(entry.to_owned())));
    }

    NotifyRequest {
        replaces_id,
        summary: notification.title.clone(),
        body,
        actions,
        hints,
        expire_timeout: EXPIRE_BY_SERVER,
    }
}

/// the NotificationClosed reasons from the spec
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CloseReason {
    Expired,
    DismissedByUser,
    ClosedByCall,
    Undefined,
}

impl CloseReason {
    pub fn from_code(code: u32) -> Self {
        match code {
            1 => Self::Expired,
            2 => Self::DismissedByUser,
            3 => Self::ClosedByCall,
            _ => Self::Undefined,
        }
    }
}

/// what a click resolved to, the caller tag and button id (none for the body)
/// plus any text typed into its field
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResolvedResponse {
    pub tag: NotificationTag,
    pub action_id: Option<String>,
    pub user_text: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct LiveNotification {
    tag: NotificationTag,
    action_ids: Vec<String>,
    reply_action_id: Option<String>,
}

/// server ids of notifications on screen per tag
/// replaces_id and CloseNotification need the server id and only this table knows it
#[derive(Default, Debug)]
pub struct LiveNotifications {
    by_id: HashMap<u32, LiveNotification>,
    id_by_tag: HashMap<NotificationTag, u32>,
}

impl LiveNotifications {
    /// id to replace for this tag, 0 for a new notification
    pub fn replaces_id(&self, tag: &NotificationTag) -> u32 {
        self.id_by_tag.get(tag).copied().unwrap_or(0)
    }

    /// remembers the id the server answered Notify with
    pub fn record(&mut self, id: u32, notification: &LocalNotification) {
        if let Some(previous) = self.id_by_tag.insert(notification.tag.clone(), id) {
            if previous != id {
                self.by_id.remove(&previous);
            }
        }
        self.by_id.insert(
            id,
            LiveNotification {
                tag: notification.tag.clone(),
                action_ids: notification.actions.iter().map(|a| a.id.clone()).collect(),
                reply_action_id: notification
                    .actions
                    .iter()
                    .find(|action| action.text_input.is_some())
                    .map(|action| action.id.clone()),
            },
        );
    }

    /// id to close for a dismiss, forgotten at once
    /// the NotificationClosed that follows then finds nothing
    pub fn take_for_dismiss(&mut self, tag: &NotificationTag) -> Option<u32> {
        let id = self.id_by_tag.remove(tag)?;
        self.by_id.remove(&id);
        Some(id)
    }

    /// forgets a closed notification, except an expired popup on a server with history
    /// it is still in the shade so a click or replace can reach it
    pub fn closed(&mut self, id: u32, reason: CloseReason, capabilities: &ServerCapabilities) {
        if reason == CloseReason::Expired && capabilities.persistence {
            return;
        }
        if let Some(live) = self.by_id.remove(&id) {
            if self.id_by_tag.get(&live.tag) == Some(&id) {
                self.id_by_tag.remove(&live.tag);
            }
        }
    }

    /// maps ActionInvoked back to the caller
    /// none for an id this process didnt post (the signal goes to every client) or an unknown key
    pub fn resolve(&self, id: u32, action_key: &str) -> Option<ResolvedResponse> {
        let live = self.by_id.get(&id)?;
        let action_id = if action_key == DEFAULT_ACTION_KEY {
            None
        } else if action_key == INLINE_REPLY_ACTION_KEY {
            Some(live.reply_action_id.clone()?)
        } else {
            let index: usize = action_key
                .strip_prefix(TRANSPORT_ACTION_PREFIX)?
                .parse()
                .ok()?;
            Some(live.action_ids.get(index)?.clone())
        };
        Some(ResolvedResponse {
            tag: live.tag.clone(),
            action_id,
            user_text: None,
        })
    }

    /// maps NotificationReplied back to the caller
    /// none for a foreign id or one without a text input button, the server closes the banner after
    pub fn resolve_reply(&self, id: u32, text: &str) -> Option<ResolvedResponse> {
        let live = self.by_id.get(&id)?;
        Some(ResolvedResponse {
            tag: live.tag.clone(),
            action_id: Some(live.reply_action_id.clone()?),
            user_text: Some(text.to_owned()),
        })
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}
