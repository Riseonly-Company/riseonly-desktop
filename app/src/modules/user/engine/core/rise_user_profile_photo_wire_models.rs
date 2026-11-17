use rise_canon::rise_key::user as key;
use rise_canon::rise_proto::user::ProfilePhotoDto;
use rise_canon::{RiseWireKey, RiseWireObject, rise_decode};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;

/// one photo from a profile album as the device stores it, stored with the canon keys
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RiseProfilePhoto {
    pub id: String,
    pub file_url: String,
    pub created_at_ms: i64,
    pub is_main: bool,
    pub is_pending: bool,
}

impl RiseProfilePhoto {
    pub fn new(id: String, file_url: String, created_at_ms: i64, is_main: bool, is_pending: bool) -> Self {
        Self { id, file_url, created_at_ms, is_main, is_pending }
    }

    pub fn from_canon(canon: &ProfilePhotoDto) -> Self {
        Self::new(canon.id.clone(), canon.file_url.clone(), canon.created_at_ms, canon.is_main, false)
    }
}

impl<'de> Deserialize<'de> for RiseProfilePhoto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                id: c.rise_required::<String>(key::ID).unwrap_or_default(),
                file_url: c.rise_required::<String>(key::FILE_URL).unwrap_or_default(),
                created_at_ms: rise_profile_photo_int64(c, key::CREATED_AT_MS).unwrap_or(0),
                is_main: c.rise_required::<bool>(key::IS_MAIN).unwrap_or(false),
                is_pending: false,
            })
        })
    }
}

impl Serialize for RiseProfilePhoto {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(Some(4))?;
        container.serialize_entry(key::ID, &self.id)?;
        container.serialize_entry(&RiseWireKey::camel(key::FILE_URL), &self.file_url)?;
        container.serialize_entry(&RiseWireKey::camel(key::CREATED_AT_MS), &self.created_at_ms)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_MAIN), &self.is_main)?;
        container.end()
    }
}

fn rise_profile_photo_int64(container: &RiseWireObject, key: &str) -> Option<i64> {
    if let Some(value) = container.rise::<i64>(key).ok().flatten() {
        return Some(value);
    }
    if let Some(value) = container.rise::<String>(key).ok().flatten() {
        return value.parse::<i64>().ok();
    }
    if let Some(value) = container.rise::<f64>(key).ok().flatten() {
        return Some(value as i64);
    }
    None
}
