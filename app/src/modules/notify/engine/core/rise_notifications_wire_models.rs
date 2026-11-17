use std::collections::BTreeMap;

use rise_canon::rise_key::events::TYPE as EVENTS_TYPE;
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};
use serde_json::Value;

use crate::modules::notify::stores::notify_actions::notify_types::{
    NotificationActorItem, NotificationItem, NotificationKind,
};

/// one feed row as the gateway spells it
/// data is a json document inside a string, the service stores it as jsonb and prints it verbatim
pub type RiseNotificationDTO = rise_canon::rise_proto::notify::ProtoNotification;
pub type RiseNotificationActorDTO = rise_canon::rise_proto::notify::ProtoNotificationActor;
pub type RiseNotificationsPageDTO = rise_canon::rise_response::notify::GetNotifications;

pub trait RiseNotificationActorDTOItem {
    fn to_actor_item(&self) -> NotificationActorItem;
}

impl RiseNotificationActorDTOItem for RiseNotificationActorDTO {
    fn to_actor_item(&self) -> NotificationActorItem {
        NotificationActorItem {
            id: self.id,
            actor_id: self.actor_id.clone(),
            actor_name: self.actor_name.clone(),
            actor_tag: self.actor_tag.clone(),
            actor_avatar: self.actor_avatar.clone().filter(|avatar| !avatar.is_empty()),
        }
    }
}

/// extension that turns a notification dto into an item
pub trait RiseNotificationDTOItem {
    /// parsed once
    /// data is decoded here and the timestamp becomes milliseconds
    /// nothing downstream needs a formatter or json parser
    fn to_notification_item(&self) -> Option<NotificationItem>;
}

impl RiseNotificationDTOItem for RiseNotificationDTO {
    fn to_notification_item(&self) -> Option<NotificationItem> {
        if self.id == 0 {
            return None;
        }
        let mut data_map: BTreeMap<String, String> = BTreeMap::new();
        if !self.data.is_empty()
            && let Ok(Value::Object(parsed)) = serde_json::from_str::<Value>(&self.data)
        {
            for (key, value) in &parsed {
                data_map.insert(key.clone(), string_value(value));
            }
        }
        let kind = kind(&self.notification_type, &data_map);
        Some(NotificationItem {
            id: self.id.to_string(),
            kind,
            title: self.title.clone(),
            body: self.body.clone(),
            data: data_map,
            is_read: self.is_read,
            created_at_ms: milliseconds(Some(self.created_at.as_str()))
                .max(milliseconds(self.actors.first().map(|actor| actor.created_at.as_str()))),
            group_count: self.group_count.max(1),
            actors: self.actors.iter().map(RiseNotificationActorDTOItem::to_actor_item).collect(),
        })
    }
}

/// server stores a post comment under the reply type, only data.type tells them apart
fn kind(wire: &str, data: &BTreeMap<String, String>) -> NotificationKind {
    let kind = NotificationKind::from_raw_string(wire);
    if kind == NotificationKind::CommentReply
        && data.get(EVENTS_TYPE).map(String::as_str) == Some(NotificationKind::PostComment.raw_value())
    {
        return NotificationKind::PostComment;
    }
    kind
}

fn string_value(value: &Value) -> String {
    match value {
        Value::String(string) => string.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => (if *flag { "1" } else { "0" }).to_owned(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// gateway prints RFC 3339 with fractional seconds, the plain form is accepted for older rows
/// parsers are made per call
fn milliseconds(value: Option<&str>) -> i64 {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return 0;
    };
    let mut fractional = ISO8601DateFormatter::new();
    fractional.set_format_options(
        ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME | ISO8601DateFormatterOptions::WITH_FRACTIONAL_SECONDS,
    );
    if let Some(date) = fractional.date_from(value) {
        return (date.time_interval_since_1970() * 1_000.0) as i64;
    }
    let mut plain = ISO8601DateFormatter::new();
    plain.set_format_options(ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME);
    let Some(date) = plain.date_from(value) else {
        return 0;
    };
    (date.time_interval_since_1970() * 1_000.0) as i64
}
