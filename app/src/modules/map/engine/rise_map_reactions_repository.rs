use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::fmt;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::FutureExt;
use gpui::AsyncApp;
use rise_canon::rise_blob::map_reaction_inbox::{Batch, Entry, Inbox, Page};
use rise_canon::{RiseLimits, RiseTimingCanon, rise_response};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use serde::de::DeserializeOwned;
use tokio::task::AbortHandle;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::rise_map_resource_repository::{
    RiseMapFailure, RiseMapResourceRepository, RiseMapResourceRepositoryResource,
};
use crate::modules::map::engine::core::rise_map_rpc::RiseMapRPC;
use crate::modules::map::engine::rise_map_presentation::{RiseMapPresentation, RiseMapSessionIdentity};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_wire_types::{
    StickerWireDTO, StickerWireDTOExt,
};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MapReactionInboxEntry {
    pub sender_id: String,
    pub count: i64,
    pub last_id: i64,
}

#[derive(Clone, Debug)]
pub struct MapReactionBatch {
    pub id: i64,
    pub emoji: String,
    pub count: i64,
    pub sent_at: i64,
    pub custom_emoji: Option<StickerWireDTO>,
}

impl MapReactionBatch {
    pub fn sticker(&self) -> Option<StickerItem> {
        self.custom_emoji.as_ref().map(|custom_emoji| custom_emoji.to_sticker())
    }
}

#[derive(Clone, Debug)]
pub struct MapReactionPage {
    pub claim_id: String,
    pub reactions: Vec<MapReactionBatch>,
    pub has_more: bool,
    pub next_cursor: i64,
    pub consumed_through: Option<i64>,
}

/// inbox and claimed pages arrive as the map_reaction_inbox blob
/// a page or entry missing required fields is dropped
#[derive(Debug)]
struct MapReactionBlobMissingField;

impl fmt::Display for MapReactionBlobMissingField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MapReactionBlobMissingField")
    }
}

impl std::error::Error for MapReactionBlobMissingField {}

impl MapReactionInboxEntry {
    pub fn from_canon(canon: &Entry) -> Option<Self> {
        let (Some(sender_id), Some(count), Some(last_id)) = (canon.sender_id.clone(), canon.count, canon.last_id) else {
            return None;
        };
        Some(Self { sender_id, count, last_id })
    }

    pub fn canon(&self) -> Entry {
        Entry { sender_id: Some(self.sender_id.clone()), count: Some(self.count), last_id: Some(self.last_id) }
    }
}

impl MapReactionBatch {
    pub fn from_canon(canon: &Batch) -> Option<Self> {
        let (Some(id), Some(emoji), Some(count), Some(sent_at)) = (canon.id, canon.emoji.clone(), canon.count, canon.sent_at)
        else {
            return None;
        };
        Some(Self { id, emoji, count, sent_at, custom_emoji: canon.custom_emoji.clone() })
    }
}

impl MapReactionPage {
    fn from_canon(canon: Page) -> Result<Self, MapReactionBlobMissingField> {
        let (Some(claim_id), Some(reactions), Some(has_more), Some(next_cursor)) =
            (canon.claim_id, canon.reactions, canon.has_more, canon.next_cursor)
        else {
            return Err(MapReactionBlobMissingField);
        };
        let batches: Vec<MapReactionBatch> = reactions.iter().filter_map(MapReactionBatch::from_canon).collect();
        if batches.len() != reactions.len() {
            return Err(MapReactionBlobMissingField);
        }
        Ok(Self { claim_id, reactions: batches, has_more, next_cursor, consumed_through: canon.consumed_through })
    }
}

#[derive(Clone, Debug)]
struct RiseMapReactionsRepositoryBurst {
    id: Uuid,
    user_id: String,
    emoji: String,
    count: i64,
    attempts: i64,
}

struct RiseMapReactionsRepositoryState {
    inbox: Vec<MapReactionInboxEntry>,
    revision: u64,
    refresh_generation: u64,
    refresh_task: Option<AbortHandle>,
    last_fetch: Option<Duration>,
    refresh_requested: bool,
    is_shutdown: bool,
    lifecycle_generation: u64,
    refresh_id: Option<Uuid>,
    hydrated: bool,
    outgoing: Vec<RiseMapReactionsRepositoryBurst>,
    flush_task: Option<AbortHandle>,
}

static BURST_MAXIMUM: LazyLock<i64> = LazyLock::new(|| RiseLimits::service_value("map_max_reaction_burst"));
static FLUSH_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::debounce("map_reaction_burst").client_debounce_ms);
static FLUSH_ATTEMPTS: LazyLock<i64> = LazyLock::new(|| RiseLimits::client_value("map_reaction_flush_attempts"));
static MAXIMUM_FRIENDS: LazyLock<usize> =
    LazyLock::new(|| usize::try_from(RiseLimits::service_value("map_max_friends")).unwrap_or(0));

pub struct RiseMapReactionsRepository {
    rpc: RiseMapRPC,
    resources: Arc<RiseMapResourceRepository>,
    identity: RiseMapSessionIdentity,
    state: RiseIsolated<RiseMapReactionsRepositoryState>,
}

impl RiseMapReactionsRepository {
    pub fn new(rpc: RiseMapRPC, resources: Arc<RiseMapResourceRepository>, identity: RiseMapSessionIdentity) -> Self {
        Self {
            rpc,
            resources,
            identity,
            state: RiseIsolated::new(RiseMapReactionsRepositoryState {
                inbox: Vec::new(),
                revision: 0,
                refresh_generation: 0,
                refresh_task: None,
                last_fetch: None,
                refresh_requested: false,
                is_shutdown: false,
                lifecycle_generation: 0,
                refresh_id: None,
                hydrated: false,
                outgoing: Vec::new(),
                flush_task: None,
            }),
        }
    }

    pub fn schedule_refresh(self: &Arc<Self>) {
        let this = self.clone();
        self.state.with(|s| {
            if s.is_shutdown {
                return;
            }
            s.refresh_requested = true;
            if s.refresh_task.is_some() {
                return;
            }
            let id = Uuid::new_v4();
            s.refresh_id = Some(id);
            let handle = RiseAccountRuntime::current().spawn(async move {
                loop {
                    let keep_running = this.state.with(|s| {
                        if !s.is_shutdown && s.refresh_requested {
                            return true;
                        }
                        if s.refresh_id == Some(id) {
                            s.refresh_task = None;
                            s.refresh_id = None;
                        }
                        false
                    });
                    if !keep_running {
                        break;
                    }
                    this.fetch_paced().await;
                }
            });
            s.refresh_task = Some(handle.abort_handle());
        });
    }

    async fn fetch_paced(&self) {
        if let Some(last_fetch) = self.state.with(|s| s.last_fetch) {
            let due = last_fetch + Duration::from_millis(u64::try_from(*FLUSH_MS).unwrap_or(0));
            let now = RiseClock::monotonic();
            if due > now {
                RiseRuntimeTimer::sleep(due - now).await;
            }
        }
        let now = RiseClock::monotonic();
        let is_running = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.last_fetch = Some(now);
            s.refresh_requested = false;
            true
        });
        if !is_running {
            return;
        }
        self.fetch().await;
    }

    async fn fetch(&self) {
        let (generation, hydrates) = self.state.with(|s| {
            let hydrates = !s.hydrated;
            s.hydrated = true;
            (s.refresh_generation, hydrates)
        });
        if hydrates
            && let Some(cache) = self.resources.cached::<Inbox>(RiseMapResourceRepositoryResource::Reactions).await
        {
            let applied = self.state.with(|s| {
                if generation != s.refresh_generation || s.is_shutdown {
                    return false;
                }
                s.inbox = Self::entries(&cache).into_iter().take(*MAXIMUM_FRIENDS).collect();
                true
            });
            if applied {
                self.publish().await;
            }
        }
        let Ok(next) = self.fetch_inbox().await else {
            return;
        };
        let Some(blob) = self.state.with(|s| {
            if s.is_shutdown || generation != s.refresh_generation {
                return None;
            }
            s.inbox = Self::entries(&next).into_iter().take(*MAXIMUM_FRIENDS).collect();
            Some(Self::blob(&s.inbox))
        }) else {
            return;
        };
        self.resources.store(RiseMapResourceRepositoryResource::Reactions, blob).await;
        self.publish().await;
    }

    async fn fetch_inbox(&self) -> anyhow::Result<Inbox> {
        let wire = self.rpc.get_reaction_inbox().await?;
        self.decode::<Inbox>(&wire)
    }

    pub async fn claim(&self, sender_id: &str) -> anyhow::Result<MapReactionPage> {
        let Some(generation) = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.refresh_generation = s.refresh_generation.wrapping_add(1);
            Some(s.lifecycle_generation)
        }) else {
            return Err(CancellationError.into());
        };
        let wire = self.rpc.claim_reactions(sender_id).await?;
        let page = self.prepare(MapReactionPage::from_canon(self.decode::<Page>(&wire)?)?);
        let Some(blob) = self.state.with(|s| {
            if s.is_shutdown || generation != s.lifecycle_generation {
                return None;
            }
            s.refresh_generation = s.refresh_generation.wrapping_add(1);
            if let Some(consumed) = page.consumed_through {
                s.inbox.retain(|entry| !(entry.sender_id == sender_id && entry.last_id <= consumed));
            }
            Some(Self::blob(&s.inbox))
        }) else {
            return Err(CancellationError.into());
        };
        self.resources.store(RiseMapResourceRepositoryResource::Reactions, blob).await;
        self.publish().await;
        Ok(page)
    }

    pub async fn page(&self, sender_id: &str, claim_id: &str, after: i64) -> anyhow::Result<MapReactionPage> {
        if self.state.with(|s| s.is_shutdown) {
            return Err(CancellationError.into());
        }
        let wire = self.rpc.get_reaction_page(sender_id, claim_id, Some(after)).await?;
        if self.state.with(|s| s.is_shutdown) {
            return Err(CancellationError.into());
        }
        Ok(self.prepare(MapReactionPage::from_canon(self.decode::<Page>(&wire)?)?))
    }

    pub async fn enqueue(self: &Arc<Self>, target: &str, emoji: &str) -> bool {
        let weak = Arc::downgrade(self);
        self.state.with(|s| {
            let queued_for_target: i64 =
                s.outgoing.iter().filter(|burst| burst.user_id == target).map(|burst| burst.count).sum();
            if s.is_shutdown || s.outgoing.len() >= *MAXIMUM_FRIENDS || queued_for_target >= *BURST_MAXIMUM {
                return false;
            }
            if let Some(last) = s.outgoing.last_mut()
                && last.user_id == target
                && last.emoji == emoji
            {
                last.count += 1;
            } else {
                s.outgoing.push(RiseMapReactionsRepositoryBurst {
                    id: Uuid::new_v4(),
                    user_id: target.to_owned(),
                    emoji: emoji.to_owned(),
                    count: 1,
                    attempts: 0,
                });
            }
            if s.flush_task.is_some() {
                return true;
            }
            let generation = s.lifecycle_generation;
            let flush_delay = Duration::from_millis(u64::try_from(*FLUSH_MS).unwrap_or(0));
            let handle = RiseAccountRuntime::current().spawn(async move {
                loop {
                    tokio::time::sleep(flush_delay).await;
                    let Some(this) = weak.upgrade() else {
                        break;
                    };
                    if !this.flush(generation).await {
                        break;
                    }
                }
            });
            s.flush_task = Some(handle.abort_handle());
            true
        })
    }

    async fn flush(&self, generation: u64) -> bool {
        let Some(due) = self.state.with(|s| {
            if s.is_shutdown || generation != s.lifecycle_generation {
                return None;
            }
            let mut recipients: Vec<&str> = Vec::new();
            let mut due: Vec<RiseMapReactionsRepositoryBurst> = Vec::new();
            for burst in &s.outgoing {
                if !recipients.contains(&burst.user_id.as_str()) {
                    recipients.push(burst.user_id.as_str());
                    due.push(burst.clone());
                }
            }
            Some(due)
        }) else {
            return false;
        };
        for mut burst in due {
            let is_running = self.state.with(|s| {
                if s.is_shutdown {
                    return false;
                }
                s.outgoing.retain(|queued| queued.id != burst.id);
                true
            });
            if !is_running {
                return false;
            }
            if let Err(error) = self.rpc.send_reaction(&burst.user_id, &burst.emoji, burst.count).await {
                let is_paced = RiseMapFailure::is_reaction_paced(&error);
                self.state.with(|s| {
                    if !s.is_shutdown
                        && generation == s.lifecycle_generation
                        && is_paced
                        && burst.attempts + 1 < *FLUSH_ATTEMPTS
                    {
                        burst.attempts += 1;
                        s.outgoing.insert(0, burst);
                    }
                });
            }
        }
        self.state.with(|s| {
            if generation != s.lifecycle_generation {
                return false;
            }
            let keep_running = !s.outgoing.is_empty();
            if !keep_running {
                s.flush_task = None;
            }
            keep_running
        })
    }

    pub async fn reset(&self) {
        self.state.with(|s| {
            s.lifecycle_generation = s.lifecycle_generation.wrapping_add(1);
            s.refresh_generation = s.refresh_generation.wrapping_add(1);
            if let Some(refresh_task) = s.refresh_task.take() {
                refresh_task.abort();
            }
            s.refresh_id = None;
            s.last_fetch = None;
            s.refresh_requested = false;
            if let Some(flush_task) = s.flush_task.take() {
                flush_task.abort();
            }
            s.outgoing = Vec::new();
            s.inbox = Vec::new();
        });
        self.resources.store(RiseMapResourceRepositoryResource::Reactions, Self::blob(&[])).await;
        self.publish().await;
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.lifecycle_generation = s.lifecycle_generation.wrapping_add(1);
            s.refresh_generation = s.refresh_generation.wrapping_add(1);
            if let Some(refresh_task) = s.refresh_task.take() {
                refresh_task.abort();
            }
            if let Some(flush_task) = s.flush_task.take() {
                flush_task.abort();
            }
            s.outgoing = Vec::new();
        });
    }

    fn prepare(&self, page: MapReactionPage) -> MapReactionPage {
        let mut batches: Vec<MapReactionBatch> = Vec::new();
        for item in page.reactions {
            if let Some(previous) = batches.last_mut()
                && previous.emoji == item.emoji
            {
                previous.count += item.count;
                if previous.custom_emoji.is_none() {
                    previous.custom_emoji = item.custom_emoji;
                }
            } else {
                batches.push(item);
            }
        }
        MapReactionPage {
            claim_id: page.claim_id,
            reactions: batches,
            has_more: page.has_more,
            next_cursor: page.next_cursor,
            consumed_through: page.consumed_through,
        }
    }

    fn decode<T: DeserializeOwned>(&self, wire: &rise_response::map::GetReactionInbox) -> anyhow::Result<T> {
        Ok(serde_json::from_str::<T>(&wire.snapshot_json)?)
    }

    fn entries(blob: &Inbox) -> Vec<MapReactionInboxEntry> {
        blob.inbox.iter().flatten().filter_map(MapReactionInboxEntry::from_canon).collect()
    }

    fn blob(entries: &[MapReactionInboxEntry]) -> Inbox {
        Inbox { inbox: Some(entries.iter().map(MapReactionInboxEntry::canon).collect()) }
    }

    async fn publish(&self) {
        let (entries, revision) = self.state.with(|s| {
            s.revision = s.revision.wrapping_add(1);
            (s.inbox.clone(), s.revision)
        });
        let identity = self.identity.clone();
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.publish_reactions(entries, revision, &identity, cx);
                        });
                    });
                    Ok::<_, anyhow::Error>(())
                }
                .boxed_local()
            })
            .await;
    }
}
