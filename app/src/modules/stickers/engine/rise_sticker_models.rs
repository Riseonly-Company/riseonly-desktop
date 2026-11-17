use std::collections::{HashMap, HashSet};
use std::fmt;

use rise_canon::{RiseLimits, RiseTimingCanon};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_limits::LimitSpec as RiseLimitSpec;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    GifItem, PackKind, StickerItem, StickerPack,
};

pub type RiseStickerPickerTab = rise_catalog::StickerPickerTab;

pub trait RiseStickerPickerTabOrdinal: Sized {
    fn from_ordinal(value: i64) -> Option<Self>;
}

impl RiseStickerPickerTabOrdinal for RiseStickerPickerTab {
    fn from_ordinal(value: i64) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| i64::from(tab.ordinal()) == value)
    }
}

pub type RiseStickerLoadState = rise_catalog::StickerLoadState;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RiseStickerResourceID {
    DefaultPacks(PackKind),
    SavedPacks(PackKind),
    MyPacks(PackKind),
    Pack(String),
    PackLink(String),
    Favourites,
    Recents(PackKind),
    GifFavourites,
    GifSearch(String),
}

#[derive(Clone, PartialEq, Debug)]
pub struct RiseStickerPage<Value: Clone + PartialEq> {
    pub items: Vec<Value>,
    pub next_relative_id: Option<String>,
    pub has_more: bool,
    pub total: Option<i64>,
}

impl<Value: Clone + PartialEq> Default for RiseStickerPage<Value> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            next_relative_id: None,
            has_more: false,
            total: None,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct RiseStickerPackContent {
    pub pack: Option<StickerPack>,
    pub stickers: Vec<StickerItem>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RiseStickerMutationID {
    CreatePack(PackKind),
    AddSticker { pack_id: String },
    EditSticker { sticker_id: String },
    DeleteSticker { sticker_id: String },
    ReorderStickers { pack_id: String },
}

pub type RiseStickerMutationState = rise_catalog::StickerMutationState;

#[derive(Clone, PartialEq, Debug)]
pub struct RiseStickerUploadPresentation {
    pub request_id: String,
    pub pack_id: String,
    pub local_sticker_id: String,
    pub phase: RiseStickerMutationState,
    pub fraction: f64,
    pub completed_bytes: i64,
    pub total_bytes: i64,
}

impl RiseStickerUploadPresentation {
    pub fn new(
        request_id: String,
        pack_id: String,
        local_sticker_id: String,
        phase: RiseStickerMutationState,
        fraction: f64,
        completed_bytes: i64,
        total_bytes: i64,
    ) -> Self {
        let total_bytes_floor = total_bytes.max(0);
        Self {
            request_id,
            pack_id,
            local_sticker_id,
            phase,
            fraction: fraction.clamp(0.0, 1.0),
            total_bytes: total_bytes_floor,
            completed_bytes: completed_bytes.clamp(0, total_bytes_floor),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StickerMediaUploadDraft {
    pub local_url: Url,
    pub filename: String,
    pub mime_type: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub thumbnail_url: Option<Url>,
}

impl StickerMediaUploadDraft {
    pub fn new(
        local_url: Url,
        filename: String,
        mime_type: String,
        width: Option<i64>,
        height: Option<i64>,
        duration_ms: Option<i64>,
        thumbnail_url: Option<Url>,
    ) -> Self {
        Self {
            local_url,
            filename,
            mime_type,
            width,
            height,
            duration_ms,
            thumbnail_url,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct RiseStickerPresentationSnapshot {
    pub account_id: String,
    pub revision: u64,
    pub default_packs: HashMap<PackKind, RiseStickerPage<StickerPack>>,
    pub saved_packs: HashMap<PackKind, RiseStickerPage<StickerPack>>,
    pub my_packs: HashMap<PackKind, RiseStickerPage<StickerPack>>,
    pub packs: HashMap<String, RiseStickerPackContent>,
    pub linked_packs: HashMap<String, RiseStickerPackContent>,
    pub favourites: Vec<StickerItem>,
    pub recents: HashMap<PackKind, Vec<StickerItem>>,
    pub gif_favourites: RiseStickerPage<GifItem>,
    pub gif_searches: HashMap<String, RiseStickerPage<GifItem>>,
    pub load_states: HashMap<RiseStickerResourceID, RiseStickerLoadState>,
    pub pending_favourite_ids: HashSet<String>,
    pub pending_gif_ids: HashSet<String>,
    pub mutation_states: HashMap<RiseStickerMutationID, RiseStickerMutationState>,
    pub mutation_errors: HashMap<RiseStickerMutationID, String>,
    pub mutation_progress: HashMap<RiseStickerMutationID, f64>,
    pub sticker_uploads: HashMap<String, RiseStickerUploadPresentation>,
    pub custom_emoji: HashMap<String, StickerItem>,
}

impl RiseStickerPresentationSnapshot {
    pub fn new(account_id: String) -> Self {
        Self {
            account_id,
            ..Self::default()
        }
    }
}

pub enum RiseStickerPerformancePolicy {}

impl RiseStickerPerformancePolicy {
    pub fn recents_limit() -> i64 {
        Self::canon_limit("recent_stickers")
    }

    pub fn packs_page_limit() -> i64 {
        Self::canon_limit("sticker_packs_page")
    }

    pub fn picker_packs_limit() -> i64 {
        RiseLimits::client_value("rise_sticker_performance_policy_picker_packs_limit")
    }

    pub fn pack_summary_limit() -> i64 {
        RiseLimits::client_value("rise_sticker_performance_policy_pack_summary_limit")
    }

    pub const SAVED_PACK_SUMMARY_LIMIT: i64 = i64::MAX;

    pub fn created_packs_per_kind_limit() -> i64 {
        RiseLimits::service_value("max_created_packs_per_kind")
    }

    pub fn packs_limit() -> i64 {
        Self::picker_packs_limit()
    }

    pub fn stickers_per_pack_limit() -> i64 {
        Self::canon_limit("stickers_per_pack")
    }

    /// server accepts twenty but five is the product limit, kept in canon so clients cant disagree
    pub fn associated_emojis_limit() -> i64 {
        Self::canon_limit("sticker_associated_emojis")
    }

    pub fn gif_page_limit() -> i64 {
        RiseLimits::client_value("rise_sticker_performance_policy_gif_page_limit")
    }

    /// saved gifs are paged to the end so the cap must hold the largest list the server keeps,
    /// smaller dropped the oldest pages
    pub fn gif_limit() -> i64 {
        let Some(spec) = rise_limits::SERVICE_LIMITS
            .iter()
            .find(|spec| spec.name == "max_gif_favourites")
        else {
            panic!("Canon service limit max_gif_favourites is missing");
        };
        RiseLimits::client_value("rise_sticker_performance_policy_gif_limit")
            .max(rise_limits::signed(spec.premium.unwrap_or(spec.value)))
    }

    pub fn search_cache_limit() -> i64 {
        RiseLimits::client_value("rise_sticker_performance_policy_search_cache_limit")
    }

    pub fn search_items_limit() -> i64 {
        RiseLimits::client_value("rise_sticker_performance_policy_search_items_limit")
    }

    pub fn custom_emoji_cache_limit() -> i64 {
        RiseLimits::client_value("rise_sticker_performance_policy_custom_emoji_cache_limit")
    }

    pub fn custom_emoji_batch_limit() -> i64 {
        RiseLimits::service_value("max_custom_emoji_ids")
    }

    pub fn persisted_pack_limit() -> i64 {
        RiseLimits::value("sticker_packs_page")
    }

    pub fn initial_pack_prefetch_count() -> i64 {
        RiseLimits::client_value("rise_sticker_performance_policy_initial_pack_prefetch_count")
    }

    pub fn refresh_interval() -> f64 {
        RiseTimingCanon::freshness_seconds("sticker_packs")
    }

    pub fn initial_pack_ids(saved: &[StickerPack], defaults: &[StickerPack]) -> Vec<String> {
        let mut seen = HashSet::new();
        let prefix = usize::try_from(Self::initial_pack_prefetch_count()).unwrap_or(0);
        saved
            .iter()
            .chain(defaults.iter())
            .filter(|pack| !pack.id.is_empty() && seen.insert(pack.id.clone()))
            .take(prefix)
            .map(|pack| pack.id.clone())
            .collect()
    }

    pub fn favourites_limit(is_premium: bool) -> i64 {
        let spec = Self::canon_spec("favourite_stickers");
        if is_premium {
            rise_limits::signed(spec.premium.unwrap_or(spec.value))
        } else {
            rise_limits::signed(spec.value)
        }
    }

    fn canon_limit(name: &str) -> i64 {
        rise_limits::signed(Self::canon_spec(name).value)
    }

    fn canon_spec(name: &str) -> &'static RiseLimitSpec {
        match rise_limits::spec(name) {
            Some(spec) => spec,
            None => panic!("Canon limit {name} is missing"),
        }
    }
}

/// rejected carries an english detail for the log only, the screen always gets canon text
#[derive(Clone, PartialEq, Eq)]
pub enum RiseStickerRepositoryError {
    Rejected(String),
    InvalidPackTitle,
    InvalidMediaDraft,
    PackCreationLimitReached(PackKind),
    MutationInProgress,
}

impl RiseStickerRepositoryError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::Rejected(_) | Self::MutationInProgress => tr("retry_later_error"),
            Self::InvalidPackTitle => tr("sticker_pack_create_failed"),
            Self::InvalidMediaDraft => tr("sticker_pack_media_unsupported"),
            Self::PackCreationLimitReached(_) => tr("sticker_pack_limit_reached"),
        })
    }

    pub fn debug_description(&self) -> String {
        match self {
            Self::Rejected(detail) => format!("rejected: {detail}"),
            Self::InvalidPackTitle => "invalidPackTitle".to_owned(),
            Self::InvalidMediaDraft => "invalidMediaDraft".to_owned(),
            Self::PackCreationLimitReached(kind) => format!("packCreationLimitReached({})", kind.wire()),
            Self::MutationInProgress => "mutationInProgress".to_owned(),
        }
    }
}

impl fmt::Display for RiseStickerRepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl fmt::Debug for RiseStickerRepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.debug_description())
    }
}

impl std::error::Error for RiseStickerRepositoryError {}

#[cfg(test)]
#[path = "rise_sticker_models_tests.rs"]
mod tests;
