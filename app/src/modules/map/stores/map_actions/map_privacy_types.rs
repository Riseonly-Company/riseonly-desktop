use serde::{Deserialize, Serialize};

use super::map_types::MapUser;

pub type MapPrivacyRule = rise_catalog::MapPrivacyRule;

pub trait MapPrivacyRuleIdentifiable {
    fn id(self) -> &'static str;
}

impl MapPrivacyRuleIdentifiable for MapPrivacyRule {
    fn id(self) -> &'static str {
        self.wire()
    }
}

pub mod map_privacy_rule_codable {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::MapPrivacyRule;

    pub fn serialize<S: Serializer>(rule: &MapPrivacyRule, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(rule.wire())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<MapPrivacyRule, D::Error> {
        let raw = String::deserialize(deserializer)?;
        MapPrivacyRule::ALL
            .iter()
            .copied()
            .find(|rule| rule.wire() == raw)
            .ok_or_else(|| D::Error::unknown_variant(&raw, MapPrivacyRule::WIRES))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapPrivacyKind {
    Presence,
    Tracks,
    Media,
    Friends,
}

impl MapPrivacyKind {
    pub const ALL: [MapPrivacyKind; 4] =
        [MapPrivacyKind::Presence, MapPrivacyKind::Tracks, MapPrivacyKind::Media, MapPrivacyKind::Friends];

    pub const fn raw_value(self) -> &'static str {
        match self {
            MapPrivacyKind::Presence => "presence",
            MapPrivacyKind::Tracks => "tracks",
            MapPrivacyKind::Media => "media",
            MapPrivacyKind::Friends => "friends",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.raw_value() == raw)
    }

    pub const fn id(self) -> &'static str {
        self.raw_value()
    }

    pub fn rules(self) -> Vec<MapPrivacyRule> {
        match self {
            MapPrivacyKind::Presence => {
                vec![MapPrivacyRule::MapFriends, MapPrivacyRule::Contacts, MapPrivacyRule::Selected]
            }
            MapPrivacyKind::Tracks | MapPrivacyKind::Media | MapPrivacyKind::Friends => vec![
                MapPrivacyRule::MapFriends,
                MapPrivacyRule::Contacts,
                MapPrivacyRule::Selected,
                MapPrivacyRule::Nobody,
            ],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapPrivacyAction {
    Allow,
    Deny,
}

impl MapPrivacyAction {
    pub const fn raw_value(self) -> &'static str {
        match self {
            MapPrivacyAction::Allow => "allow",
            MapPrivacyAction::Deny => "deny",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        [MapPrivacyAction::Allow, MapPrivacyAction::Deny].into_iter().find(|action| action.raw_value() == raw)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPrivacy {
    #[serde(with = "map_privacy_rule_codable")]
    pub presence_rule: MapPrivacyRule,
    #[serde(with = "map_privacy_rule_codable")]
    pub tracks_rule: MapPrivacyRule,
    pub presence_allow: Vec<MapUser>,
    pub presence_deny: Vec<MapUser>,
    pub tracks_allow: Vec<MapUser>,
    pub tracks_deny: Vec<MapUser>,
    #[serde(with = "map_privacy_rule_codable")]
    pub media_rule: MapPrivacyRule,
    #[serde(with = "map_privacy_rule_codable")]
    pub friends_rule: MapPrivacyRule,
    pub media_allow: Vec<MapUser>,
    pub media_deny: Vec<MapUser>,
    pub friends_allow: Vec<MapUser>,
    pub friends_deny: Vec<MapUser>,
}

impl MapPrivacy {
    pub fn new(
        presence_rule: MapPrivacyRule,
        tracks_rule: MapPrivacyRule,
        presence_allow: Vec<MapUser>,
        presence_deny: Vec<MapUser>,
        tracks_allow: Vec<MapUser>,
        tracks_deny: Vec<MapUser>,
    ) -> Self {
        Self {
            presence_rule,
            tracks_rule,
            presence_allow,
            presence_deny,
            tracks_allow,
            tracks_deny,
            media_rule: MapPrivacyRule::MapFriends,
            friends_rule: MapPrivacyRule::MapFriends,
            media_allow: Vec::new(),
            media_deny: Vec::new(),
            friends_allow: Vec::new(),
            friends_deny: Vec::new(),
        }
    }

    pub fn rule(&self, kind: MapPrivacyKind) -> MapPrivacyRule {
        match kind {
            MapPrivacyKind::Presence => self.presence_rule,
            MapPrivacyKind::Tracks => self.tracks_rule,
            MapPrivacyKind::Media => self.media_rule,
            MapPrivacyKind::Friends => self.friends_rule,
        }
    }

    pub fn users(&self, kind: MapPrivacyKind, action: MapPrivacyAction) -> &[MapUser] {
        match (kind, action) {
            (MapPrivacyKind::Presence, MapPrivacyAction::Allow) => &self.presence_allow,
            (MapPrivacyKind::Presence, MapPrivacyAction::Deny) => &self.presence_deny,
            (MapPrivacyKind::Tracks, MapPrivacyAction::Allow) => &self.tracks_allow,
            (MapPrivacyKind::Tracks, MapPrivacyAction::Deny) => &self.tracks_deny,
            (MapPrivacyKind::Media, MapPrivacyAction::Allow) => &self.media_allow,
            (MapPrivacyKind::Media, MapPrivacyAction::Deny) => &self.media_deny,
            (MapPrivacyKind::Friends, MapPrivacyAction::Allow) => &self.friends_allow,
            (MapPrivacyKind::Friends, MapPrivacyAction::Deny) => &self.friends_deny,
        }
    }

    pub fn set_rule(&mut self, rule: MapPrivacyRule, kind: MapPrivacyKind) {
        match kind {
            MapPrivacyKind::Presence => self.presence_rule = rule,
            MapPrivacyKind::Tracks => self.tracks_rule = rule,
            MapPrivacyKind::Media => self.media_rule = rule,
            MapPrivacyKind::Friends => self.friends_rule = rule,
        }
    }

    pub fn set_users(&mut self, users: Vec<MapUser>, kind: MapPrivacyKind, action: MapPrivacyAction) {
        match (kind, action) {
            (MapPrivacyKind::Presence, MapPrivacyAction::Allow) => self.presence_allow = users,
            (MapPrivacyKind::Presence, MapPrivacyAction::Deny) => self.presence_deny = users,
            (MapPrivacyKind::Tracks, MapPrivacyAction::Allow) => self.tracks_allow = users,
            (MapPrivacyKind::Tracks, MapPrivacyAction::Deny) => self.tracks_deny = users,
            (MapPrivacyKind::Media, MapPrivacyAction::Allow) => self.media_allow = users,
            (MapPrivacyKind::Media, MapPrivacyAction::Deny) => self.media_deny = users,
            (MapPrivacyKind::Friends, MapPrivacyAction::Allow) => self.friends_allow = users,
            (MapPrivacyKind::Friends, MapPrivacyAction::Deny) => self.friends_deny = users,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MapPrivacyListTarget {
    pub kind: MapPrivacyKind,
    pub action: MapPrivacyAction,
}

impl MapPrivacyListTarget {
    pub fn id(&self) -> String {
        format!("{}.{}", self.kind.raw_value(), self.action.raw_value())
    }
}
