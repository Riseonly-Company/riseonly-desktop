use std::collections::HashSet;

use rise_canon::rise_key::theme as key;
use rise_canon::rise_proto::theme::{ProtoWallpaperPreferencesResponse, ProtoWallpaperRecord};
use rise_canon::{RiseWireKey, RiseWireObject, rise_decode};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::lib::rise_wire_key_camel::RiseWireKeyCamel;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperLook;

/// one wallpaper row from the gateway,
/// every field has a default on decode so one odd member cant fail the whole catalogue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseChatWallpaper {
    pub id: String,
    pub name: String,
    pub preview_url: String,
    pub image_url: String,
    pub is_premium: bool,
    pub is_built_in: bool,
    pub created_at_ms: i64,
    pub dim_percent: i64,
    pub blur_percent: i64,
    pub is_pending: bool,
}

impl Default for RiseChatWallpaper {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            preview_url: String::new(),
            image_url: String::new(),
            is_premium: false,
            is_built_in: false,
            created_at_ms: 0,
            dim_percent: ChatWallpaperLook::DEFAULT_DIM_PERCENT,
            blur_percent: 0,
            is_pending: false,
        }
    }
}

impl RiseChatWallpaper {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        name: String,
        preview_url: String,
        image_url: String,
        is_premium: bool,
        is_built_in: bool,
        created_at_ms: i64,
        dim_percent: i64,
        blur_percent: i64,
        is_pending: bool,
    ) -> Self {
        Self {
            id,
            name,
            preview_url,
            image_url,
            is_premium,
            is_built_in,
            created_at_ms,
            dim_percent,
            blur_percent,
            is_pending,
        }
    }

    /// a pending row has no server preview so it draws from the picker exported file
    pub fn display_url(&self) -> &str {
        if self.preview_url.is_empty() { &self.image_url } else { &self.preview_url }
    }

    pub fn look(&self) -> ChatWallpaperLook {
        ChatWallpaperLook::new(self.dim_percent, self.blur_percent)
    }

    pub fn from_canon(row: &ProtoWallpaperRecord) -> Self {
        Self::new(
            row.id.clone(),
            row.name.clone(),
            if row.preview_url.is_empty() { row.image_url.clone() } else { row.preview_url.clone() },
            row.image_url.clone(),
            row.is_premium,
            row.is_built_in,
            row.created_at_ms,
            ChatWallpaperLook::clamp_percent(row.dim_percent),
            ChatWallpaperLook::clamp_percent(row.blur_percent),
            false,
        )
    }
}

impl<'de> Deserialize<'de> for RiseChatWallpaper {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            let image = c.rise_required::<String>(key::IMAGE_URL).unwrap_or_default();
            let preview = c.rise_required::<String>(key::PREVIEW_URL).unwrap_or_default();
            Ok(Self {
                id: c.rise_required::<String>(key::ID).unwrap_or_default(),
                name: c.rise_required::<String>(key::NAME).unwrap_or_default(),
                preview_url: if preview.is_empty() { image.clone() } else { preview },
                image_url: image,
                is_premium: c.rise_required::<bool>(key::IS_PREMIUM).unwrap_or(false),
                is_built_in: c.rise_required::<bool>(key::IS_BUILT_IN).unwrap_or(false),
                created_at_ms: rise_chat_wallpaper_int64(c, key::CREATED_AT_MS).unwrap_or(0),
                dim_percent: ChatWallpaperLook::clamp_percent(
                    rise_chat_wallpaper_int64(c, key::DIM_PERCENT).unwrap_or(ChatWallpaperLook::DEFAULT_DIM_PERCENT),
                ),
                blur_percent: ChatWallpaperLook::clamp_percent(
                    rise_chat_wallpaper_int64(c, key::BLUR_PERCENT).unwrap_or(0),
                ),
                is_pending: false,
            })
        })
    }
}

impl Serialize for RiseChatWallpaper {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(Some(9))?;
        container.serialize_entry(key::ID, &self.id)?;
        container.serialize_entry(key::NAME, &self.name)?;
        container.serialize_entry(&RiseWireKey::camel(key::PREVIEW_URL), &self.preview_url)?;
        container.serialize_entry(&RiseWireKey::camel(key::IMAGE_URL), &self.image_url)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_PREMIUM), &self.is_premium)?;
        container.serialize_entry(&RiseWireKey::camel(key::IS_BUILT_IN), &self.is_built_in)?;
        container.serialize_entry(&RiseWireKey::camel(key::CREATED_AT_MS), &self.created_at_ms)?;
        container.serialize_entry(&RiseWireKey::camel(key::DIM_PERCENT), &self.dim_percent)?;
        container.serialize_entry(&RiseWireKey::camel(key::BLUR_PERCENT), &self.blur_percent)?;
        container.end()
    }
}

/// how this account treats wallpapers others set on shared chats
/// own_only is the mode and each set holds the exceptions for one mode so flipping never loses them
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseWallpaperPreferences {
    pub own_only: bool,
    pub show_chat_wallpaper_in: HashSet<String>,
    pub use_own_wallpaper_in: HashSet<String>,
}

impl RiseWallpaperPreferences {
    pub fn new(own_only: bool, show_chat_wallpaper_in: HashSet<String>, use_own_wallpaper_in: HashSet<String>) -> Self {
        Self { own_only, show_chat_wallpaper_in, use_own_wallpaper_in }
    }

    pub fn default_preferences() -> Self {
        Self::new(false, HashSet::new(), HashSet::new())
    }

    /// true when a chat own wallpaper is drawn for this account
    pub fn shows_chat_wallpaper(&self, chat_id: &str) -> bool {
        if self.own_only {
            self.show_chat_wallpaper_in.contains(chat_id)
        } else {
            !self.use_own_wallpaper_in.contains(chat_id)
        }
    }

    pub fn set_shows_chat_wallpaper(&mut self, shows: bool, chat_id: &str) {
        if self.own_only {
            if shows {
                self.show_chat_wallpaper_in.insert(chat_id.to_owned());
            } else {
                self.show_chat_wallpaper_in.remove(chat_id);
            }
        } else if shows {
            self.use_own_wallpaper_in.remove(chat_id);
        } else {
            self.use_own_wallpaper_in.insert(chat_id.to_owned());
        }
    }

    pub fn from_canon(reply: &ProtoWallpaperPreferencesResponse) -> Self {
        Self::new(
            reply.own_only,
            reply.show_chat_wallpaper_in.iter().cloned().collect(),
            reply.use_own_wallpaper_in.iter().cloned().collect(),
        )
    }
}

impl<'de> Deserialize<'de> for RiseWallpaperPreferences {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        rise_decode(deserializer, |c| {
            Ok(Self {
                own_only: c.rise_required::<bool>(key::OWN_ONLY)?,
                show_chat_wallpaper_in: c.rise_required::<HashSet<String>>(key::SHOW_CHAT_WALLPAPER_IN)?,
                use_own_wallpaper_in: c.rise_required::<HashSet<String>>(key::USE_OWN_WALLPAPER_IN)?,
            })
        })
    }
}

impl Serialize for RiseWallpaperPreferences {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut container = serializer.serialize_map(Some(3))?;
        container.serialize_entry(&RiseWireKey::camel(key::OWN_ONLY), &self.own_only)?;
        container.serialize_entry(&RiseWireKey::camel(key::SHOW_CHAT_WALLPAPER_IN), &self.show_chat_wallpaper_in)?;
        container.serialize_entry(&RiseWireKey::camel(key::USE_OWN_WALLPAPER_IN), &self.use_own_wallpaper_in)?;
        container.end()
    }
}

/// ms come as a number from the gateway, a string from legacy json paths
/// both are read so a row age is never zero, legacy is cursed
fn rise_chat_wallpaper_int64(container: &RiseWireObject, key: &str) -> Option<i64> {
    if let Some(value) = container.rise::<i64>(key).ok().flatten() {
        return Some(value);
    }
    if let Some(value) = container.rise::<f64>(key).ok().flatten() {
        return Some(value as i64);
    }
    if let Some(value) = container.rise::<String>(key).ok().flatten() {
        return value.parse::<i64>().ok();
    }
    None
}
