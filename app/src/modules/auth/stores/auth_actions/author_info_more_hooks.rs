use rise_canon::rise_key::events;
use rise_canon::rise_key::post as key;
use rise_canon::{RiseWireKey, rise_decode};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AuthorInfoMore {
    pub p_lang: Vec<String>,
    pub who: String,
    pub role: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_official: bool,
    pub phone: Option<String>,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
}

impl<'de> Deserialize<'de> for AuthorInfoMore {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |container| {
            Ok(Self {
                p_lang: container.rise_required::<Vec<String>>(key::P_LANG)?,
                who: container.rise_required::<String>(key::WHO)?,
                role: container.rise_required::<String>(key::ROLE)?,
                logo: container.rise_required::<String>(key::LOGO)?,
                is_premium: container.rise_required::<bool>(key::IS_PREMIUM)?,
                is_official: container.rise_required::<bool>(key::IS_OFFICIAL)?,
                phone: container.rise::<String>(events::PHONE)?,
                premium_badge_emoji: container.rise::<String>(key::PREMIUM_BADGE_EMOJI)?,
                name_color: container.rise::<String>(key::NAME_COLOR)?,
            })
        })
    }
}

impl Serialize for AuthorInfoMore {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(None)?;
        container.serialize_entry(&RiseWireKey::camel(key::P_LANG), &self.p_lang)?;
        container.serialize_entry(key::WHO, &self.who)?;
        container.serialize_entry(key::ROLE, &self.role)?;
        container.serialize_entry(key::LOGO, &self.logo)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_PREMIUM), &self.is_premium)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_OFFICIAL), &self.is_official)?;
        if let Some(phone) = &self.phone {
            container.serialize_entry(events::PHONE, phone)?;
        }
        if let Some(premium_badge_emoji) = &self.premium_badge_emoji {
            container.serialize_entry(&RiseWireKey::camel(key::PREMIUM_BADGE_EMOJI), premium_badge_emoji)?;
        }
        if let Some(name_color) = &self.name_color {
            container.serialize_entry(&RiseWireKey::camel(key::NAME_COLOR), name_color)?;
        }
        container.end()
    }
}
