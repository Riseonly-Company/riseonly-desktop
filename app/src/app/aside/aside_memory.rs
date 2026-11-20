use std::collections::BTreeMap;

use rise_platform::user_defaults::{DefaultsValue, UserDefaults};
use rise_platform::user_defaults_keys;

use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;

use super::aside_registry::AsideSection;

const ACCOUNT_FIELD: &str = "account";
const SECTIONS_FIELD: &str = "sections";
const KIND_FIELD: &str = "kind";
const ID_FIELD: &str = "id";
const TAG_FIELD: &str = "tag";

const KIND_MY_PROFILE: &str = "my_profile";
const KIND_USER_PAGE: &str = "user_page";
const KIND_USER_FRIENDS: &str = "user_friends";
const KIND_USER_SUBS: &str = "user_subs";
const KIND_USER_SUBSCRIBERS: &str = "user_subscribers";
const KIND_POST: &str = "post";
const KIND_CHAT_PROFILE: &str = "chat_profile";
const KIND_NOTIFICATIONS: &str = "notifications";

/// right panel stack of every section kept across launches
/// only routes made of plain ids are saved, one with a loaded payload would come back stale
pub struct AsideMemory;

impl AsideMemory {
    pub fn remember(account: Option<&str>, stacks: &[(AsideSection, Vec<RootRoute>)]) {
        let defaults = UserDefaults::standard();
        let Some(account) = account else {
            defaults.remove_object(user_defaults_keys::DESKTOP_ASIDE_STACK);
            return;
        };
        let sections: BTreeMap<String, DefaultsValue> = stacks
            .iter()
            .filter_map(|(section, routes)| {
                let encoded: Vec<DefaultsValue> = routes.iter().map_while(Self::encode).collect();
                (!encoded.is_empty()).then(|| (section.context_key().to_owned(), DefaultsValue::from(encoded)))
            })
            .collect();
        let mut fields = BTreeMap::new();
        fields.insert(ACCOUNT_FIELD.to_owned(), DefaultsValue::from(account));
        fields.insert(SECTIONS_FIELD.to_owned(), DefaultsValue::from(sections));
        defaults.set(user_defaults_keys::DESKTOP_ASIDE_STACK, DefaultsValue::from(fields));
    }

    pub fn recall(account: Option<&str>) -> Vec<(AsideSection, Vec<RootRoute>)> {
        let Some(account) = account else {
            return Vec::new();
        };
        let Some(fields) = UserDefaults::standard().dictionary(user_defaults_keys::DESKTOP_ASIDE_STACK) else {
            return Vec::new();
        };
        if fields.get(ACCOUNT_FIELD).and_then(DefaultsValue::as_str) != Some(account) {
            return Vec::new();
        }
        let Some(sections) = fields.get(SECTIONS_FIELD).and_then(DefaultsValue::as_dictionary) else {
            return Vec::new();
        };
        sections
            .iter()
            .filter_map(|(key, routes)| {
                let section = AsideSection::from_context_key(key)?;
                let routes: Vec<RootRoute> = routes.as_array()?.iter().map_while(Self::decode).collect();
                (!routes.is_empty()).then_some((section, routes))
            })
            .collect()
    }

    pub fn forget_pinned() {
        UserDefaults::standard().remove_object(user_defaults_keys::DESKTOP_ASIDE_PINNED);
    }

    fn encode(route: &RootRoute) -> Option<DefaultsValue> {
        if let RootRoute::ChatProfile { chat_id } = route
            && GetSaiServerId::is_temp_id(chat_id.as_deref())
        {
            return None;
        }
        let (kind, id, tag) = match route {
            RootRoute::MyProfile => (KIND_MY_PROFILE, None, None),
            RootRoute::UserPage { user_id, tag } => (KIND_USER_PAGE, user_id.clone(), tag.clone()),
            RootRoute::UserFriends { user_id } => (KIND_USER_FRIENDS, user_id.clone(), None),
            RootRoute::UserSubs { user_id } => (KIND_USER_SUBS, user_id.clone(), None),
            RootRoute::UserSubscribers { user_id } => (KIND_USER_SUBSCRIBERS, user_id.clone(), None),
            RootRoute::PostDetail { post_id, origin: None } => (KIND_POST, post_id.clone(), None),
            RootRoute::Chat { .. } => return None,
            RootRoute::ChatProfile { chat_id } => (KIND_CHAT_PROFILE, chat_id.clone(), None),
            RootRoute::Notifications => (KIND_NOTIFICATIONS, None, None),
            _ => return None,
        };
        let mut fields = BTreeMap::new();
        fields.insert(KIND_FIELD.to_owned(), DefaultsValue::from(kind));
        if let Some(id) = id {
            fields.insert(ID_FIELD.to_owned(), DefaultsValue::from(id));
        }
        if let Some(tag) = tag {
            fields.insert(TAG_FIELD.to_owned(), DefaultsValue::from(tag));
        }
        Some(DefaultsValue::from(fields))
    }

    fn decode(value: &DefaultsValue) -> Option<RootRoute> {
        let fields = value.as_dictionary()?;
        let text = |name: &str| fields.get(name).and_then(DefaultsValue::as_str).map(str::to_owned);
        let id = text(ID_FIELD);
        let route = match fields.get(KIND_FIELD).and_then(DefaultsValue::as_str)? {
            KIND_MY_PROFILE => RootRoute::MyProfile,
            KIND_USER_PAGE => RootRoute::UserPage { user_id: id, tag: text(TAG_FIELD) },
            KIND_USER_FRIENDS => RootRoute::UserFriends { user_id: id },
            KIND_USER_SUBS => RootRoute::UserSubs { user_id: id },
            KIND_USER_SUBSCRIBERS => RootRoute::UserSubscribers { user_id: id },
            KIND_POST => RootRoute::PostDetail { post_id: id, origin: None },
            KIND_CHAT_PROFILE => RootRoute::ChatProfile { chat_id: id },
            KIND_NOTIFICATIONS => RootRoute::Notifications,
            _ => return None,
        };
        Self::encode(&route).map(|_| route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_routes_round_trip_and_payload_routes_cut_the_stack() {
        let routes = [
            RootRoute::UserPage { user_id: Some("u1".to_owned()), tag: Some("tag".to_owned()) },
            RootRoute::PostDetail { post_id: Some("p1".to_owned()), origin: None },
            RootRoute::Settings,
            RootRoute::Notifications,
        ];
        let encoded: Vec<DefaultsValue> = routes.iter().map_while(AsideMemory::encode).collect();
        let decoded: Vec<RootRoute> = encoded.iter().map_while(AsideMemory::decode).collect();
        assert_eq!(decoded, routes[..2].to_vec());
    }

    #[test]
    fn conversation_cuts_the_stack() {
        let routes = [
            RootRoute::UserPage { user_id: Some("u1".to_owned()), tag: None },
            RootRoute::Chat { chat_id: Some("c1".to_owned()) },
            RootRoute::Notifications,
        ];
        let encoded: Vec<DefaultsValue> = routes.iter().map_while(AsideMemory::encode).collect();
        assert_eq!(encoded.len(), 1);
    }
}
