use rise_canon::rise_proto;

use super::map_privacy_types::{MapPrivacy, MapPrivacyRule};
use super::map_wire_types::ProtoMapUserExt;
use crate::modules::map::engine::rise_map_engine_models::RiseMapError;

fn map_privacy_rule_or_default(raw: &str) -> MapPrivacyRule {
    MapPrivacyRule::from_wire(raw).unwrap_or(MapPrivacyRule::MapFriends)
}

pub trait ProtoMapPrivacyExt {
    fn model(&self) -> MapPrivacy;
}

impl ProtoMapPrivacyExt for rise_proto::map::ProtoMapPrivacy {
    fn model(&self) -> MapPrivacy {
        MapPrivacy {
            presence_rule: map_privacy_rule_or_default(&self.presence_rule),
            tracks_rule: map_privacy_rule_or_default(&self.tracks_rule),
            presence_allow: self.presence_allow.iter().filter_map(ProtoMapUserExt::model).collect(),
            presence_deny: self.presence_deny.iter().filter_map(ProtoMapUserExt::model).collect(),
            tracks_allow: self.tracks_allow.iter().filter_map(ProtoMapUserExt::model).collect(),
            tracks_deny: self.tracks_deny.iter().filter_map(ProtoMapUserExt::model).collect(),
            media_rule: self.media_rule.as_deref().map(map_privacy_rule_or_default).unwrap_or(MapPrivacyRule::MapFriends),
            friends_rule: self
                .friends_rule
                .as_deref()
                .map(map_privacy_rule_or_default)
                .unwrap_or(MapPrivacyRule::MapFriends),
            media_allow: self.media_allow.iter().filter_map(ProtoMapUserExt::model).collect(),
            media_deny: self.media_deny.iter().filter_map(ProtoMapUserExt::model).collect(),
            friends_allow: self.friends_allow.iter().filter_map(ProtoMapUserExt::model).collect(),
            friends_deny: self.friends_deny.iter().filter_map(ProtoMapUserExt::model).collect(),
        }
    }
}

pub trait ProtoPrivacyResponseExt {
    fn model(&self) -> Result<MapPrivacy, RiseMapError>;
}

impl ProtoPrivacyResponseExt for rise_proto::map::ProtoPrivacyResponse {
    fn model(&self) -> Result<MapPrivacy, RiseMapError> {
        let Some(privacy) = self.privacy.as_ref() else {
            return Err(RiseMapError::InvalidResponse);
        };
        Ok(privacy.model())
    }
}
