use std::sync::Arc;

use gpui::{App, AppContext, Context, Entity};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::guides::guide_facts::GuideFacts;
use crate::modules::map::engine::map_guide_sandbox_hooks::map_guide_sandbox_hooks;
use crate::modules::map::engine::core::map_track_tile_store::MapTrackTileStore;
use crate::modules::map::engine::map_reaction_stage_hooks::map_reaction_stage_hooks;
use crate::modules::map::engine::rise_map_drafts_repository::RiseMapDraftsSnapshot;
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapFriendsSnapshot, RiseMapLiveSnapshot, RiseMapStateSnapshot,
};
use crate::modules::map::engine::rise_map_photos_repository::RiseMapPhotosSnapshot;
use crate::modules::map::engine::rise_map_privacy_repository::RiseMapPrivacySnapshot;
use crate::modules::map::engine::rise_map_reactions_repository::MapReactionInboxEntry;
use crate::modules::map::engine::rise_map_tracks_repository::{RiseMapTrackTilesTick, RiseMapTracksSnapshot};

/// one box per concern, a moving friend re-renders only the pins and a settings toggle only its own
pub struct RiseMapStateBox {
    snapshot: RiseMapStateSnapshot,
}

impl RiseMapStateBox {
    fn new() -> Self {
        Self { snapshot: RiseMapStateSnapshot::EMPTY }
    }

    pub fn snapshot(&self) -> &RiseMapStateSnapshot {
        &self.snapshot
    }
}

pub struct RiseMapFriendsBox {
    snapshot: RiseMapFriendsSnapshot,
}

impl RiseMapFriendsBox {
    fn new() -> Self {
        Self { snapshot: RiseMapFriendsSnapshot::EMPTY }
    }

    pub fn snapshot(&self) -> &RiseMapFriendsSnapshot {
        &self.snapshot
    }
}

pub struct RiseMapLiveBox {
    snapshot: RiseMapLiveSnapshot,
}

impl RiseMapLiveBox {
    fn new() -> Self {
        Self { snapshot: RiseMapLiveSnapshot::EMPTY }
    }

    pub fn snapshot(&self) -> &RiseMapLiveSnapshot {
        &self.snapshot
    }
}

pub struct RiseMapTracksBox {
    snapshot: RiseMapTracksSnapshot,
}

impl RiseMapTracksBox {
    fn new() -> Self {
        Self { snapshot: RiseMapTracksSnapshot::EMPTY }
    }

    pub fn snapshot(&self) -> &RiseMapTracksSnapshot {
        &self.snapshot
    }
}

pub struct RiseMapPrivacyBox {
    snapshot: RiseMapPrivacySnapshot,
}

impl RiseMapPrivacyBox {
    fn new() -> Self {
        Self { snapshot: RiseMapPrivacySnapshot::EMPTY }
    }

    pub fn snapshot(&self) -> &RiseMapPrivacySnapshot {
        &self.snapshot
    }
}

pub struct RiseMapPhotosBox {
    snapshot: RiseMapPhotosSnapshot,
}

impl RiseMapPhotosBox {
    fn new() -> Self {
        Self { snapshot: RiseMapPhotosSnapshot::EMPTY }
    }

    pub fn snapshot(&self) -> &RiseMapPhotosSnapshot {
        &self.snapshot
    }
}

pub struct RiseMapDraftsBox {
    snapshot: RiseMapDraftsSnapshot,
}

impl RiseMapDraftsBox {
    fn new() -> Self {
        Self { snapshot: RiseMapDraftsSnapshot::empty() }
    }

    pub fn snapshot(&self) -> &RiseMapDraftsSnapshot {
        &self.snapshot
    }
}

/// redraw orders for the track layer
/// separate from the tracks box so a landing tile re-renders the map layer and never the distance panel
pub struct RiseMapTrackTilesBox {
    tick: RiseMapTrackTilesTick,
    store: Option<Arc<MapTrackTileStore>>,
    source_generation: u64,
}

impl RiseMapTrackTilesBox {
    fn new() -> Self {
        Self { tick: RiseMapTrackTilesTick::IDLE, store: None, source_generation: 0 }
    }

    pub fn tick(&self) -> &RiseMapTrackTilesTick {
        &self.tick
    }

    pub fn store(&self) -> Option<&Arc<MapTrackTileStore>> {
        self.store.as_ref()
    }
}

/// server unread counters are separate from the currently playing claimed interval
pub struct RiseMapReactionBox {
    inbox: Vec<MapReactionInboxEntry>,
    revision: u64,
}

impl RiseMapReactionBox {
    fn new() -> Self {
        Self { inbox: Vec::new(), revision: 0 }
    }

    pub fn inbox(&self) -> &[MapReactionInboxEntry] {
        &self.inbox
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// main thread end of the map domain
/// immutable snapshots accepted only from the active session
pub struct RiseMapPresentation {
    pub state: Entity<RiseMapStateBox>,
    pub friends: Entity<RiseMapFriendsBox>,
    pub live: Entity<RiseMapLiveBox>,
    pub reactions: Entity<RiseMapReactionBox>,
    pub tracks: Entity<RiseMapTracksBox>,
    pub track_tiles: Entity<RiseMapTrackTilesBox>,
    pub privacy: Entity<RiseMapPrivacyBox>,
    pub photos: Entity<RiseMapPhotosBox>,
    pub drafts: Entity<RiseMapDraftsBox>,

    account_id: Option<String>,
    account_epoch: Option<String>,
    session_id: Option<Uuid>,
}

impl RiseMapPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: cx.new(|_| RiseMapStateBox::new()),
            friends: cx.new(|_| RiseMapFriendsBox::new()),
            live: cx.new(|_| RiseMapLiveBox::new()),
            reactions: cx.new(|_| RiseMapReactionBox::new()),
            tracks: cx.new(|_| RiseMapTracksBox::new()),
            track_tiles: cx.new(|_| RiseMapTrackTilesBox::new()),
            privacy: cx.new(|_| RiseMapPrivacyBox::new()),
            photos: cx.new(|_| RiseMapPhotosBox::new()),
            drafts: cx.new(|_| RiseMapDraftsBox::new()),
            account_id: None,
            account_epoch: None,
            session_id: None,
        }
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        tile_store: Arc<MapTrackTileStore>,
        cx: &mut Context<Self>,
    ) {
        let is_same_session = self.account_id.as_deref() == Some(account_id)
            && self.account_epoch.as_deref() == Some(account_epoch)
            && self.session_id == Some(session_id);
        if is_same_session {
            return;
        }
        self.account_id = Some(account_id.to_owned());
        self.account_epoch = Some(account_epoch.to_owned());
        self.session_id = Some(session_id);
        self.reset(cx);
        self.track_tiles.update(cx, |tiles, _| tiles.store = Some(tile_store));
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.account_id.as_deref() != Some(account_id) || self.session_id != Some(session_id) {
            return;
        }
        self.account_id = None;
        self.account_epoch = None;
        self.session_id = None;
        self.reset(cx);
    }

    pub fn publish_state(
        &mut self,
        snapshot: RiseMapStateSnapshot,
        session: &RiseMapSessionIdentity,
        cx: &mut Context<Self>,
    ) {
        let current = self.state.read(cx).snapshot();
        if !self.accepts(session) || snapshot.revision < current.revision || *current == snapshot {
            return;
        }
        let is_member = snapshot.is_member();
        self.state.update(cx, |state, cx| {
            state.snapshot = snapshot;
            cx.notify();
        });
        let owns_map_scene = (map_guide_sandbox_hooks().owns_map_scene)(cx);
        cx.defer(move |cx| GuideFacts::set(RiseGuideSignal::MapMember, is_member || owns_map_scene, cx));
    }

    pub fn publish_friends(
        &mut self,
        snapshot: RiseMapFriendsSnapshot,
        session: &RiseMapSessionIdentity,
        cx: &mut Context<Self>,
    ) {
        let current = self.friends.read(cx).snapshot();
        if !self.accepts(session) || snapshot.revision < current.revision || *current == snapshot {
            return;
        }
        self.friends.update(cx, |friends, cx| {
            friends.snapshot = snapshot;
            cx.notify();
        });
    }

    pub fn publish_live(&mut self, snapshot: RiseMapLiveSnapshot, session: &RiseMapSessionIdentity, cx: &mut Context<Self>) {
        let current = self.live.read(cx).snapshot();
        if !self.accepts(session) || snapshot.revision < current.revision || *current == snapshot {
            return;
        }
        self.live.update(cx, |live, cx| {
            live.snapshot = snapshot;
            cx.notify();
        });
    }

    pub fn publish_tracks(
        &mut self,
        snapshot: RiseMapTracksSnapshot,
        session: &RiseMapSessionIdentity,
        cx: &mut Context<Self>,
    ) {
        let current = self.tracks.read(cx).snapshot();
        if !self.accepts(session) || snapshot.revision < current.revision || *current == snapshot {
            return;
        }
        let is_same_content = RiseMapTracksSnapshot { revision: current.revision, ..snapshot.clone() } == *current;
        self.tracks.update(cx, |tracks, cx| {
            tracks.snapshot = snapshot;
            if !is_same_content {
                cx.notify();
            }
        });
    }

    pub fn publish_privacy(
        &mut self,
        snapshot: RiseMapPrivacySnapshot,
        session: &RiseMapSessionIdentity,
        cx: &mut Context<Self>,
    ) {
        let current = self.privacy.read(cx).snapshot();
        if !self.accepts(session) || snapshot.revision < current.revision || *current == snapshot {
            return;
        }
        self.privacy.update(cx, |privacy, cx| {
            privacy.snapshot = snapshot;
            cx.notify();
        });
    }

    pub fn publish_photos(
        &mut self,
        snapshot: RiseMapPhotosSnapshot,
        session: &RiseMapSessionIdentity,
        cx: &mut Context<Self>,
    ) {
        let current = self.photos.read(cx).snapshot();
        if !self.accepts(session) || snapshot.revision < current.revision || *current == snapshot {
            return;
        }
        self.photos.update(cx, |photos, cx| {
            photos.snapshot = snapshot;
            cx.notify();
        });
    }

    pub fn publish_drafts(
        &mut self,
        snapshot: RiseMapDraftsSnapshot,
        session: &RiseMapSessionIdentity,
        cx: &mut Context<Self>,
    ) {
        let current = self.drafts.read(cx).snapshot();
        if !self.accepts(session) || snapshot.revision < current.revision || *current == snapshot {
            return;
        }
        let is_same_content = snapshot.drafts == current.drafts && snapshot.publishing == current.publishing;
        self.drafts.update(cx, |drafts, cx| {
            drafts.snapshot = snapshot;
            if !is_same_content {
                cx.notify();
            }
        });
    }

    pub fn publish_track_tiles(
        &mut self,
        tick: RiseMapTrackTilesTick,
        session: &RiseMapSessionIdentity,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts(session) || tick.generation <= self.track_tiles.read(cx).source_generation {
            tracing::warn!(target: "riseonly::X2068", generation = tick.generation, "track tiles tick dropped");
            return;
        }
        self.track_tiles.update(cx, |tiles, cx| {
            tiles.source_generation = tick.generation;
            tiles.tick = RiseMapTrackTilesTick { generation: tiles.tick.generation + 1, ..tick };
            cx.notify();
        });
    }

    pub fn publish_reactions(
        &mut self,
        entries: Vec<MapReactionInboxEntry>,
        revision: u64,
        session: &RiseMapSessionIdentity,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts(session) || revision < self.reactions.read(cx).revision {
            return;
        }
        let unread = entries.clone();
        self.reactions.update(cx, |reactions, cx| {
            reactions.revision = revision;
            reactions.inbox = entries;
            cx.notify();
        });
        if let Some(hooks) = map_reaction_stage_hooks() {
            (hooks.set_unread)(&unread, cx);
        }
    }

    fn accepts(&self, session: &RiseMapSessionIdentity) -> bool {
        self.account_id.as_deref() == Some(session.account_id.as_str())
            && self.account_epoch.as_deref() == Some(session.account_epoch.as_str())
            && self.session_id == Some(session.session_id)
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            if state.snapshot != RiseMapStateSnapshot::EMPTY {
                state.snapshot = RiseMapStateSnapshot::EMPTY;
                cx.notify();
            }
        });
        cx.defer(|cx| GuideFacts::set(RiseGuideSignal::MapMember, false, cx));
        self.friends.update(cx, |friends, cx| {
            if friends.snapshot != RiseMapFriendsSnapshot::EMPTY {
                friends.snapshot = RiseMapFriendsSnapshot::EMPTY;
                cx.notify();
            }
        });
        self.live.update(cx, |live, cx| {
            if live.snapshot != RiseMapLiveSnapshot::EMPTY {
                live.snapshot = RiseMapLiveSnapshot::EMPTY;
                cx.notify();
            }
        });
        self.reactions.update(cx, |reactions, cx| {
            reactions.inbox = Vec::new();
            reactions.revision = 0;
            cx.notify();
        });
        if let Some(hooks) = map_reaction_stage_hooks() {
            (hooks.reset)(cx);
        }
        self.tracks.update(cx, |tracks, cx| {
            if tracks.snapshot != RiseMapTracksSnapshot::EMPTY {
                tracks.snapshot = RiseMapTracksSnapshot::EMPTY;
                cx.notify();
            }
        });
        self.privacy.update(cx, |privacy, cx| {
            if privacy.snapshot != RiseMapPrivacySnapshot::EMPTY {
                privacy.snapshot = RiseMapPrivacySnapshot::EMPTY;
                cx.notify();
            }
        });
        self.photos.update(cx, |photos, cx| {
            if photos.snapshot != RiseMapPhotosSnapshot::EMPTY {
                photos.snapshot = RiseMapPhotosSnapshot::EMPTY;
                cx.notify();
            }
        });
        self.drafts.update(cx, |drafts, cx| {
            if drafts.snapshot != RiseMapDraftsSnapshot::empty() {
                drafts.snapshot = RiseMapDraftsSnapshot::empty();
                cx.notify();
            }
        });
        self.track_tiles.update(cx, |tiles, cx| {
            tiles.store = None;
            tiles.source_generation = 0;
            tiles.tick = RiseMapTrackTilesTick {
                generation: tiles.tick.generation + 1,
                invalidated: Vec::new(),
                invalidates_all: true,
            };
            cx.notify();
        });
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RiseMapSessionIdentity {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
}
