use gpui::{App, Context, Entity};

use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiWsSliceStatus;
use crate::core::storage::rise_storage_usage_models::{
    RiseStorageBucket, RiseStorageChatUsage, RiseStorageMediaEntry, RiseStorageSite, RiseStorageUsageSnapshot,
};
use crate::core::storage::rise_storage_usage_presentation::RiseStorageUsagePresentation;

pub struct UserStorageServicesStore {}

impl UserStorageServicesStore {
    pub const CHAT_SITES: [RiseStorageSite; 2] = [RiseStorageSite::Voice, RiseStorageSite::WsPayloads];
    pub const MEDIA_SITES: [RiseStorageSite; 2] = [RiseStorageSite::Images, RiseStorageSite::RemoteMedia];

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    fn presentation(cx: &mut App) -> Entity<RiseStorageUsagePresentation> {
        RiseStorageUsagePresentation::shared(cx)
    }

    pub fn snapshot(&self, cx: &mut App) -> RiseStorageUsageSnapshot {
        Self::presentation(cx).read(cx).snapshot().clone()
    }

    pub fn status(&self, cx: &mut App) -> SaiWsSliceStatus {
        Self::presentation(cx).read(cx).status()
    }

    pub fn is_clearing(&self, cx: &mut App) -> bool {
        Self::presentation(cx).read(cx).is_clearing()
    }

    pub fn last_freed_bytes(&self, cx: &mut App) -> i64 {
        Self::presentation(cx).read(cx).last_clear_freed_bytes()
    }

    pub fn clear_completion_token(&self, cx: &mut App) -> u64 {
        Self::presentation(cx).read(cx).clear_completion_token()
    }

    pub fn chats(&self, cx: &mut App) -> Vec<RiseStorageChatUsage> {
        Self::presentation(cx).read(cx).chats().to_vec()
    }

    pub fn chats_status(&self, cx: &mut App) -> SaiWsSliceStatus {
        Self::presentation(cx).read(cx).chats_status()
    }

    pub fn chat_cache_bytes(&self, cx: &mut App) -> i64 {
        let entity = Self::presentation(cx);
        let presentation = entity.read(cx);
        let current = presentation.snapshot();
        let sites: i64 = Self::CHAT_SITES.iter().map(|site| current.bytes_site(*site)).sum();
        let chats: i64 = presentation.chats().iter().map(|chat| chat.media_bytes + chat.history_bytes).sum();
        sites + chats
    }

    pub fn unattributed_payload_bytes(&self, cx: &mut App) -> i64 {
        let entity = Self::presentation(cx);
        let presentation = entity.read(cx);
        let attributed: i64 = presentation.chats().iter().map(|chat| chat.payload_bytes).sum();
        (presentation.snapshot().bytes_site(RiseStorageSite::WsPayloads) - attributed).max(0)
    }

    pub fn media(&self, cx: &mut App) -> Vec<RiseStorageMediaEntry> {
        Self::presentation(cx).read(cx).media().to_vec()
    }

    pub fn media_total_count(&self, cx: &mut App) -> usize {
        Self::presentation(cx).read(cx).media_total_count()
    }

    pub fn media_status(&self, cx: &mut App) -> SaiWsSliceStatus {
        Self::presentation(cx).read(cx).media_status()
    }

    pub fn buckets(&self, cx: &mut App) -> Vec<RiseStorageBucketRow> {
        let entity = Self::presentation(cx);
        let current = entity.read(cx).snapshot();
        RiseStorageBucket::ALL_CASES
            .iter()
            .map(|bucket| RiseStorageBucketRow {
                bucket: *bucket,
                byte_count: current.bytes_bucket(*bucket),
            })
            .collect()
    }

    pub fn occupied_buckets(&self, cx: &mut App) -> Vec<RiseStorageBucketRow> {
        self.buckets(cx).into_iter().filter(|row| row.byte_count > 0).collect()
    }

    pub fn other_data_sites(&self, cx: &mut App) -> Vec<RiseStorageSiteRow> {
        let entity = Self::presentation(cx);
        let current = entity.read(cx).snapshot();
        let mut rows: Vec<RiseStorageSiteRow> = RiseStorageSite::ALL_CASES
            .iter()
            .filter(|site| !Self::CHAT_SITES.contains(site) && !Self::MEDIA_SITES.contains(site))
            .map(|site| RiseStorageSiteRow {
                site: *site,
                byte_count: current.bytes_site(*site),
                is_clearable: site.is_clearable(),
            })
            .filter(|row| row.byte_count > 0)
            .collect();
        rows.sort_by(|left, right| right.byte_count.cmp(&left.byte_count));
        rows
    }

    pub fn clearable_bytes(&self, cx: &mut App) -> i64 {
        Self::presentation(cx).read(cx).snapshot().clearable_bytes()
    }

    pub fn reserved_bytes(&self, cx: &mut App) -> i64 {
        Self::presentation(cx).read(cx).snapshot().reserved_bytes()
    }

    pub fn has_clearable_bytes(&self, cx: &mut App) -> bool {
        self.clearable_bytes(cx) > 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStorageBucketRow {
    pub bucket: RiseStorageBucket,
    pub byte_count: i64,
}

impl RiseStorageBucketRow {
    pub fn id(&self) -> &'static str {
        self.bucket.raw_value()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseStorageSiteRow {
    pub site: RiseStorageSite,
    pub byte_count: i64,
    pub is_clearable: bool,
}

impl RiseStorageSiteRow {
    pub fn id(&self) -> &'static str {
        self.site.raw_value()
    }
}
