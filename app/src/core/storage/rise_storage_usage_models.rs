use std::collections::HashMap;

use rise_core::foundation::foundation_date::Date;

use crate::core::media::rise_media_kind_classifier::RiseMediaKind;

/// the five rows of the storage screen
/// buckets of bytes and not of stores, one store can feed several and several can feed one
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseStorageBucket {
    Videos,
    Images,
    Files,
    Audio,
    Other,
}

impl RiseStorageBucket {
    pub const ALL_CASES: [Self; 5] = [Self::Videos, Self::Images, Self::Files, Self::Audio, Self::Other];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Videos => "videos",
            Self::Images => "images",
            Self::Files => "files",
            Self::Audio => "audio",
            Self::Other => "other",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|bucket| bucket.raw_value() == raw)
    }

    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Videos => "memory_settings_videos",
            Self::Images => "memory_settings_images",
            Self::Files => "memory_settings_files",
            Self::Audio => "memory_settings_audio",
            Self::Other => "memory_settings_other",
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            Self::Videos => "play.rectangle.fill",
            Self::Images => "photo.fill",
            Self::Files => "doc.fill",
            Self::Audio => "music.note",
            Self::Other => "tray.full.fill",
        }
    }

    pub fn from(kind: RiseMediaKind) -> Self {
        match kind {
            RiseMediaKind::Video => Self::Videos,
            RiseMediaKind::Image => Self::Images,
            RiseMediaKind::Audio => Self::Audio,
            RiseMediaKind::File => Self::Files,
        }
    }
}

/// one physical place where bytes pile up
/// everything the screen reports or may delete is named here only
/// a store missing from this list is invisible and a purge never touches it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseStorageSite {
    Images,
    RemoteMedia,
    Voice,
    Music,
    Stickers,
    StickerPosters,
    ReleaseVideo,
    WsPayloads,
    HttpResponses,
    EngineDatabase,
    LocalState,
}

impl RiseStorageSite {
    pub const ALL_CASES: [Self; 11] = [
        Self::Images,
        Self::RemoteMedia,
        Self::Voice,
        Self::Music,
        Self::Stickers,
        Self::StickerPosters,
        Self::ReleaseVideo,
        Self::WsPayloads,
        Self::HttpResponses,
        Self::EngineDatabase,
        Self::LocalState,
    ];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Images => "images",
            Self::RemoteMedia => "remoteMedia",
            Self::Voice => "voice",
            Self::Music => "music",
            Self::Stickers => "stickers",
            Self::StickerPosters => "stickerPosters",
            Self::ReleaseVideo => "releaseVideo",
            Self::WsPayloads => "wsPayloads",
            Self::HttpResponses => "httpResponses",
            Self::EngineDatabase => "engineDatabase",
            Self::LocalState => "localState",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        Self::ALL_CASES.into_iter().find(|site| site.raw_value() == raw)
    }

    /// false means the screen shows the bytes but cant delete them
    /// engine db holds the outbound queue so only a full account wipe removes it
    /// its user content and undelivered work, not cache
    pub fn is_clearable(&self) -> bool {
        !matches!(self, Self::EngineDatabase | Self::LocalState)
    }

    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Images => "storage_site_images",
            Self::RemoteMedia => "storage_site_remote_media",
            Self::Voice => "storage_site_voice",
            Self::Music => "storage_site_music",
            Self::Stickers => "storage_site_stickers",
            Self::StickerPosters => "storage_site_sticker_posters",
            Self::ReleaseVideo => "storage_site_release_video",
            Self::WsPayloads => "storage_site_ws_payloads",
            Self::HttpResponses => "storage_site_http_responses",
            Self::EngineDatabase => "storage_site_engine_database",
            Self::LocalState => "storage_site_local_state",
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            Self::Images => "photo.fill",
            Self::RemoteMedia => "play.rectangle.fill",
            Self::Voice => "waveform",
            Self::Music => "music.note",
            Self::Stickers => "face.smiling.fill",
            Self::StickerPosters => "square.on.square",
            Self::ReleaseVideo => "sparkles",
            Self::WsPayloads => "curlybraces",
            Self::HttpResponses => "network",
            Self::EngineDatabase => "externaldrive.fill",
            Self::LocalState => "shippingbox.fill",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStorageChatUsage {
    pub chat_id: String,
    pub title: String,
    pub voice_bytes: i64,
    pub payload_bytes: i64,
    pub media_bytes: i64,
    pub history_bytes: i64,
}

impl RiseStorageChatUsage {
    /// plain constructor, callers that skip history bytes pass 0
    pub fn new(
        chat_id: String,
        title: String,
        voice_bytes: i64,
        payload_bytes: i64,
        media_bytes: i64,
        history_bytes: i64,
    ) -> Self {
        Self {
            chat_id,
            title,
            voice_bytes,
            payload_bytes,
            media_bytes,
            history_bytes,
        }
    }

    pub fn id(&self) -> &str {
        &self.chat_id
    }

    pub fn total_bytes(&self) -> i64 {
        self.voice_bytes + self.payload_bytes + self.media_bytes + self.history_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStorageMediaEntry {
    pub id: String,
    pub site: RiseStorageSite,
    pub kind: RiseMediaKind,
    pub preview_source: Option<String>,
    pub byte_count: i64,
    pub modified_at: Date,
}

impl RiseStorageMediaEntry {
    pub const REMOTE_MEDIA_ID_PREFIX: &'static str = "media:";

    /// key digest of a remote media cache entry, none for any other site
    pub fn remote_media_digest(&self) -> Option<String> {
        if self.site != RiseStorageSite::RemoteMedia {
            return None;
        }
        self.id.strip_prefix(Self::REMOTE_MEDIA_ID_PREFIX).map(str::to_owned)
    }

    /// true when a single entry delete can free this entry bytes
    pub fn is_deletable(&self) -> bool {
        match self.site {
            RiseStorageSite::Images => self.preview_source.is_some(),
            RiseStorageSite::RemoteMedia => self.remote_media_digest().is_some(),
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStorageUsageSnapshot {
    pub account_key: String,
    pub bucket_bytes: HashMap<RiseStorageBucket, i64>,
    pub site_bytes: HashMap<RiseStorageSite, i64>,
    pub scanned_at: Date,
}

impl RiseStorageUsageSnapshot {
    /// empty usage with all zeros
    pub fn empty() -> Self {
        Self {
            account_key: String::new(),
            bucket_bytes: HashMap::new(),
            site_bytes: HashMap::new(),
            scanned_at: Date::distant_past(),
        }
    }

    pub fn bytes_bucket(&self, bucket: RiseStorageBucket) -> i64 {
        self.bucket_bytes.get(&bucket).copied().unwrap_or(0)
    }

    pub fn bytes_site(&self, site: RiseStorageSite) -> i64 {
        self.site_bytes.get(&site).copied().unwrap_or(0)
    }

    /// what clear all gets back
    pub fn clearable_bytes(&self) -> i64 {
        RiseStorageSite::ALL_CASES
            .into_iter()
            .filter(RiseStorageSite::is_clearable)
            .map(|site| self.bytes_site(site))
            .sum()
    }

    pub fn reserved_bytes(&self) -> i64 {
        RiseStorageSite::ALL_CASES
            .into_iter()
            .filter(|site| !site.is_clearable())
            .map(|site| self.bytes_site(site))
            .sum()
    }

    pub fn total_bytes(&self) -> i64 {
        self.clearable_bytes() + self.reserved_bytes()
    }

    pub fn is_empty(&self) -> bool {
        self.total_bytes() == 0
    }
}
