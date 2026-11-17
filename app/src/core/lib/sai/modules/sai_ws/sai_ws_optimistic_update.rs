use std::error::Error;
use std::sync::Arc;

use gpui::App;
use rise_canon::rise_key::{bot, file};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use serde_json::{Map, Value};

use crate::core::lib::sai::sai_outgoing_media_hooks::OutgoingMediaCompressorResult;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SaiWsOptimisticAddStrategy {
    #[default]
    Start,
    End,
}

impl SaiWsOptimisticAddStrategy {
    pub const ALL_CASES: [Self; 2] = [Self::Start, Self::End];

    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "start" => Some(Self::Start),
            "end" => Some(Self::End),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiWsOptimisticSuccessContext {
    pub temp_id: String,
    pub temp_id_key: String,
    pub temp_flag_key: String,
}

/// local file attached to an optimistic send
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaiWsOptimisticLocalFile {
    pub url: Url,
    pub filename: String,
    pub content_type: String,
    pub file_context_override: Option<String>,
    pub preferred_media_type: Option<String>,
    pub byte_count: Option<i64>,
}

impl SaiWsOptimisticLocalFile {
    /// new local file, optional parts get their defaults
    pub fn new(url: Url, filename: String, content_type: String) -> Self {
        Self {
            url,
            filename,
            content_type,
            file_context_override: None,
            preferred_media_type: None,
            byte_count: None,
        }
    }
}

pub type SaiWsExtractUploadedFiles = Arc<
    dyn Fn(&Map<String, Value>, usize, &SaiWsOptimisticLocalFile, &mut App) -> anyhow::Result<Value>,
>;
pub type SaiWsFinalizeUploadedFiles = Arc<dyn Fn(Vec<Value>, &mut App) -> anyhow::Result<Vec<Value>>>;
pub type SaiWsFileFractionProgress = Arc<dyn Fn(usize, f64, &mut App)>;
pub type SaiWsFileByteProgress = Arc<dyn Fn(usize, i64, i64, &mut App)>;
pub type SaiWsFileCompressedReady = Arc<dyn Fn(usize, &OutgoingMediaCompressorResult, &mut App)>;
pub type SaiWsFileSuccess = Arc<dyn Fn(usize, &Map<String, Value>, &mut App)>;
pub type SaiWsFileError = Arc<dyn Fn(usize, &anyhow::Error, &mut App)>;

/// files attached to an optimistic send
#[derive(Clone)]
pub struct SaiWsOptimisticFiles {
    pub enabled: bool,
    pub path_in_temp_data: String,
    pub files_param_key: String,
    pub file_context: String,
    pub resource_id: Option<String>,
    pub local_files: Vec<SaiWsOptimisticLocalFile>,
    pub compression: MediaCompressionConfiguration,
    pub extract_uploaded_files: Option<SaiWsExtractUploadedFiles>,
    pub finalize_uploaded_files: Option<SaiWsFinalizeUploadedFiles>,
    pub on_progress: Option<SaiWsFileFractionProgress>,
    pub on_byte_progress: Option<SaiWsFileByteProgress>,
    pub on_compression_progress: Option<SaiWsFileFractionProgress>,
    pub on_compressed_ready: Option<SaiWsFileCompressedReady>,
    pub on_file_success: Option<SaiWsFileSuccess>,
    pub on_file_error: Option<SaiWsFileError>,
    pub prefetch_clever_image_disk_keys: Vec<String>,
    pub merge_uploaded_slots_into_body_root: bool,
}

impl SaiWsOptimisticFiles {
    /// new one, file context is the only part without a default
    pub fn new(file_context: String) -> Self {
        Self {
            enabled: true,
            path_in_temp_data: String::new(),
            files_param_key: file::FILES.to_owned(),
            file_context,
            resource_id: None,
            local_files: Vec::new(),
            compression: MediaCompressionConfiguration::chat(),
            extract_uploaded_files: None,
            finalize_uploaded_files: None,
            on_progress: None,
            on_byte_progress: None,
            on_compression_progress: None,
            on_compressed_ready: None,
            on_file_success: None,
            on_file_error: None,
            prefetch_clever_image_disk_keys: Vec::new(),
            merge_uploaded_slots_into_body_root: false,
        }
    }
}

pub type SaiWsCreateTempData = Arc<dyn Fn(&Value, &mut App) -> anyhow::Result<Value>>;
pub type SaiWsExtractRealData =
    Arc<dyn Fn(&Value, &SaiWsOptimisticSuccessContext, &mut App) -> anyhow::Result<Value>>;
pub type SaiWsMatchTempWithReal = Arc<dyn Fn(&Value, &Value, &mut App) -> bool>;
pub type SaiWsOnAddedTempData = Arc<dyn Fn(&Value, &mut App)>;
pub type SaiWsOnOptimisticFailure = Arc<dyn Fn(Option<&Value>, &dyn Error, &mut App) -> bool>;
pub type SaiWsUpdateTempData = Arc<dyn Fn(&Value, &Value, &mut App) -> anyhow::Result<Value>>;
pub type SaiWsCreatePatch =
    Arc<dyn Fn(&Value, &Value, &mut App) -> anyhow::Result<Map<String, Value>>>;

#[derive(Clone)]
pub struct SaiWsOptimisticUpdate {
    pub enabled: bool,

    pub temp_id_key: String,
    pub temp_flag_key: String,

    pub add_strategy: SaiWsOptimisticAddStrategy,
    pub insert_after_last_temp: bool,

    pub create_temp_data: Option<SaiWsCreateTempData>,

    pub extract_real_data: Option<SaiWsExtractRealData>,

    pub extract_ignore_keys: Vec<String>,

    pub match_temp_with_real: Option<SaiWsMatchTempWithReal>,

    pub on_added_temp_data: Option<SaiWsOnAddedTempData>,

    pub sync_disk_cache: bool,

    pub on_optimistic_failure: Option<SaiWsOnOptimisticFailure>,

    pub files: Option<SaiWsOptimisticFiles>,

    pub delete_mode: bool,
    pub delete_id: Option<String>,

    pub update_mode: bool,
    pub update_id: Option<String>,
    pub update_temp_data: Option<SaiWsUpdateTempData>,

    pub target_cache_key: Option<String>,

    pub patch_root_in_place: bool,

    pub patch_in_place: bool,
    pub patch_paths: Vec<String>,
    pub match_id_key: String,
    pub patch_match_id: Option<String>,
    pub create_patch: Option<SaiWsCreatePatch>,
    pub create_rollback_snapshot: Option<SaiWsCreatePatch>,
}

impl Default for SaiWsOptimisticUpdate {
    fn default() -> Self {
        Self {
            enabled: false,
            temp_id_key: bot::ID.to_owned(),
            temp_flag_key: "isTemp".to_owned(),
            add_strategy: SaiWsOptimisticAddStrategy::Start,
            insert_after_last_temp: false,
            create_temp_data: None,
            extract_real_data: None,
            extract_ignore_keys: Vec::new(),
            match_temp_with_real: None,
            on_added_temp_data: None,
            sync_disk_cache: true,
            on_optimistic_failure: None,
            files: None,
            delete_mode: false,
            delete_id: None,
            update_mode: false,
            update_id: None,
            update_temp_data: None,
            target_cache_key: None,
            patch_root_in_place: false,
            patch_in_place: false,
            patch_paths: Vec::new(),
            match_id_key: bot::ID.to_owned(),
            patch_match_id: None,
            create_patch: None,
            create_rollback_snapshot: None,
        }
    }
}

impl SaiWsOptimisticUpdate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn requires_network_bypassing_cache_only_hit(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(files) = &self.files
            && files.enabled
            && !files.local_files.is_empty()
        {
            return true;
        }
        if self.patch_root_in_place {
            return self.create_patch.is_some()
                && self.create_rollback_snapshot.is_some()
                && !self.patch_paths.is_empty();
        }
        if self.patch_in_place {
            let mid = self.patch_match_id.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("");
            return self.create_patch.is_some()
                && self.create_rollback_snapshot.is_some()
                && !self.patch_paths.is_empty()
                && !mid.is_empty();
        }
        if self.delete_mode {
            return self.delete_id.is_some();
        }
        if self.update_mode {
            return self.update_id.is_some() && self.update_temp_data.is_some();
        }
        self.create_temp_data.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SaiWsOptimisticError {
    #[error("SaiWs optimistic updates require a non-empty pathToArray.")]
    PathToArrayRequired,
    #[error("SaiWs optimistic updates require a non-empty cacheKey.")]
    CacheKeyRequired,
    #[error("SaiWs optimistic JSON path not found: {0}")]
    JsonPathNotFound(String),
    #[error("SaiWs optimistic root must decode to a JSON object.")]
    NotJsonObject,
    #[error("SaiWs optimistic value at path is not an array: {0}")]
    NotArrayAtPath(String),
    #[error("SaiWs optimistic createTempData threw or returned invalid JSON.")]
    CreateTempDataFailed,
    #[error("SaiWs optimistic could not apply mutation to cache.")]
    ApplyMutationFailed,
    #[error(
        "SaiWs optimistic file upload requires the outbound data value to be a JSON object ([String: Any])."
    )]
    FileUploadNeedsDictionaryBody,
    #[error(
        "SaiWs optimistic file extractUploadedFiles must return a JSON object ([String: Any]) for each file."
    )]
    FileUploadInvalidSlotPayload,
    #[error("SaiWs optimistic file pathInTempData invalid: {0}")]
    FileUploadPathInTempDataInvalid(String),
    #[error(
        "SaiWs optimistic file slot index {index} out of range (count {count}); ensure createTempData builds matching placeholders."
    )]
    FileUploadSlotIndexOutOfRange { index: usize, count: usize },
}

impl SaiWsOptimisticError {
    pub fn error_description(&self) -> Option<String> {
        Some(self.to_string())
    }
}
