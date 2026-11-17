use std::collections::{HashMap, HashSet};

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::modules::music::engine::rise_music_repository::{
    RiseMusicRepositorySnapshot, RiseMusicSoundSnapshot,
};
use crate::modules::music::shared::utils::music::featured_profile_track_store::FeaturedProfileTrackStore;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

#[derive(Clone, Debug, PartialEq)]
pub struct RiseMusicPresentationSnapshot {
    pub account_id: Option<String>,
    pub account_epoch: Option<String>,
    pub session_id: Option<Uuid>,
    pub generation: u64,
    pub track_overrides: HashMap<String, RiseStoredMusicTrack>,
    pub updating_track_ids: HashSet<String>,
    pub upload_progress_by_track_id: HashMap<String, f64>,
    pub error_by_track_id: HashMap<String, String>,
    pub sound: RiseMusicSoundSnapshot,
}

impl RiseMusicPresentationSnapshot {
    /// empty presentation snapshot
    pub fn empty() -> Self {
        Self::cleared(None, None, None, 0)
    }

    fn cleared(
        account_id: Option<String>,
        account_epoch: Option<String>,
        session_id: Option<Uuid>,
        generation: u64,
    ) -> Self {
        Self {
            account_id,
            account_epoch,
            session_id,
            generation,
            track_overrides: HashMap::new(),
            updating_track_ids: HashSet::new(),
            upload_progress_by_track_id: HashMap::new(),
            error_by_track_id: HashMap::new(),
            sound: RiseMusicSoundSnapshot::default(),
        }
    }
}

type RiseMusicPresentationChanges = (HashMap<String, RiseStoredMusicTrack>, HashSet<String>);

pub struct RiseMusicPresentation {
    snapshot: RiseMusicPresentationSnapshot,
}

impl RiseMusicPresentation {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: RiseMusicPresentationSnapshot::empty(),
        }
    }

    pub fn snapshot(&self) -> &RiseMusicPresentationSnapshot {
        &self.snapshot
    }

    pub fn activate(
        &mut self,
        account_id: &str,
        account_epoch: &str,
        session_id: Uuid,
        cx: &mut Context<Self>,
    ) {
        self.snapshot = RiseMusicPresentationSnapshot::cleared(
            Some(account_id.to_owned()),
            Some(account_epoch.to_owned()),
            Some(session_id),
            self.snapshot.generation.wrapping_add(1),
        );
        cx.notify();
    }

    /// publishes the snapshot
    /// stores are told after the update returned, a store reading this while still borrowed would panic
    pub fn publish(value: RiseMusicRepositorySnapshot, cx: &mut App) -> bool {
        let changed = Self::shared(cx).update(cx, |presentation, cx| presentation.install(value, cx));
        let Some((changed_tracks_by_id, updating_track_ids)) = changed else {
            return false;
        };
        for (track_id, track) in changed_tracks_by_id {
            let is_optimistic = updating_track_ids.contains(&track_id);
            MusicServicesStore::shared(cx).update(cx, |store, cx| {
                store.apply_local_track_update_projection(&track, !is_optimistic, cx);
            });
            if let Some(model) = track.as_track_model() {
                if !is_optimistic {
                    FeaturedProfileTrackStore::shared(cx)
                        .update(cx, |store, cx| store.refresh_if_matching(&model, cx));
                }
                MusicPlayerState::shared(cx).update(cx, |state, cx| state.refresh_track_metadata(&model, cx));
            }
        }
        true
    }

    pub fn deactivate(&mut self, account_id: &str, session_id: Uuid, cx: &mut Context<Self>) {
        if self.snapshot.account_id.as_deref() != Some(account_id)
            || self.snapshot.session_id != Some(session_id)
        {
            return;
        }
        self.snapshot =
            RiseMusicPresentationSnapshot::cleared(None, None, None, self.snapshot.generation.wrapping_add(1));
        cx.notify();
    }

    fn install(
        &mut self,
        value: RiseMusicRepositorySnapshot,
        cx: &mut Context<Self>,
    ) -> Option<RiseMusicPresentationChanges> {
        if self.snapshot.account_id.as_deref() != Some(value.account_id.as_str())
            || self.snapshot.account_epoch.as_deref() != Some(value.account_epoch.as_str())
            || self.snapshot.session_id != Some(value.session_id)
        {
            return None;
        }
        let mut changed_tracks_by_id: HashMap<String, RiseStoredMusicTrack> = value
            .track_overrides
            .iter()
            .filter(|(id, track)| self.snapshot.track_overrides.get(*id) != Some(*track))
            .map(|(id, track)| (id.clone(), track.clone()))
            .collect();
        for track_id in self.snapshot.updating_track_ids.difference(&value.updating_track_ids) {
            if let Some(track) = value.track_overrides.get(track_id) {
                changed_tracks_by_id.insert(track_id.clone(), track.clone());
            }
        }
        let updating_track_ids = value.updating_track_ids.clone();
        self.snapshot = RiseMusicPresentationSnapshot {
            account_id: Some(value.account_id),
            account_epoch: Some(value.account_epoch),
            session_id: Some(value.session_id),
            generation: self.snapshot.generation.wrapping_add(1),
            track_overrides: value.track_overrides,
            updating_track_ids: value.updating_track_ids,
            upload_progress_by_track_id: value.upload_progress_by_track_id,
            error_by_track_id: value.error_by_track_id,
            sound: value.sound,
        };
        cx.notify();
        Some((changed_tracks_by_id, updating_track_ids))
    }
}
