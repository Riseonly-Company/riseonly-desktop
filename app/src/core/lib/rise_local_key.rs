/// keys of device local state kept inside stored json objects beside canon fields
/// the server never sends or reads them
pub enum RiseLocalKey {}

impl RiseLocalKey {
    pub const SERVER_ID: &str = "server_id";

    pub const IS_TEMP: &str = "is_temp";

    pub const SEND_FAILED: &str = "send_failed";

    pub const SYSTEM_TARGET: &str = "system_target";

    pub const OLD_IS_LIKED: &str = "old_is_liked";

    pub const OLD_IS_FAVORITED: &str = "old_is_favorited";

    pub const COUNT_VISIBILITY: &str = "count_visibility";

    pub const PRESENTATION_ID: &str = "presentation_id";

    pub const IS_UPLOADING: &str = "is_uploading";

    pub const UPLOAD_PROGRESS: &str = "upload_progress";

    pub const UPLOADED_BYTES: &str = "uploaded_bytes";

    pub const TOTAL_BYTES: &str = "total_bytes";

    pub const IS_COMPRESSING: &str = "is_compressing";

    pub const COMPRESSION_PROGRESS: &str = "compression_progress";

    pub const IS_CANCELLED: &str = "is_cancelled";

    pub const COVER_UPLOAD_PROGRESS: &str = "cover_upload_progress";

    pub const COVER_UPLOADING: &str = "cover_uploading";

    pub const AUDIO_UPLOAD_PROGRESS: &str = "audio_upload_progress";

    pub const AUDIO_UPLOADING: &str = "audio_uploading";

    pub const REMOTE_AUDIO_URL: &str = "remote_audio_url";

    pub const TRANSFER_PHASE: &str = "transfer_phase";

    pub const TRANSFERRED_BYTES: &str = "transferred_bytes";

    pub const TRANSFER_TOTAL_BYTES: &str = "transfer_total_bytes";

    pub const TRANSFER_ERROR: &str = "transfer_error";

    pub const LOCAL_PREVIEW_URL: &str = "local_preview_url";

    pub const LOCAL_SLOT_INDEX: &str = "local_slot_index";
}
