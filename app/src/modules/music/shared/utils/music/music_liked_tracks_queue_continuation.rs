use gpui::{App, Context, Entity, Subscription};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::ProtoTrackRecordExt;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

struct MusicLikedTracksQueueContinuationSession {
    account_id: Option<String>,
    account_epoch: Option<String>,
    applied_ids: Vec<String>,
    retried_cursor: Option<String>,
}

/// grows the liked songs queue page by page to the oldest like while the same track plays
pub struct MusicLikedTracksQueueContinuation {
    session: Option<MusicLikedTracksQueueContinuationSession>,
    generation: u64,
    observations: Vec<Subscription>,
}

impl MusicLikedTracksQueueContinuation {
    pub const SCOPE_ID: &'static str = "liked_songs";

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {
            session: None,
            generation: 0,
            observations: Vec::new(),
        })
    }

    pub fn begin(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.observations.clear();
        self.session = None;
        let expected = self.generation;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| this.start(expected, cx));
        })
        .detach();
    }

    fn start(&mut self, expected: u64, cx: &mut Context<Self>) {
        if expected != self.generation {
            return;
        }
        let player = MusicPlayerState::shared(cx);
        let state = player.read(cx);
        if state.playback_scope_id() != Some(Self::SCOPE_ID) {
            self.session = None;
            return;
        }
        self.session = Some(MusicLikedTracksQueueContinuationSession {
            account_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            applied_ids: state.source_queue().iter().map(|track| track.id.clone()).collect(),
            retried_cursor: None,
        });
        self.sync(expected, cx);
    }

    fn end(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.session = None;
        self.observations.clear();
    }

    fn sync(&mut self, expected: u64, cx: &mut Context<Self>) {
        if expected != self.generation {
            return;
        }
        let Some(mut session) = self.session.take() else {
            return;
        };
        let player = MusicPlayerState::shared(cx);
        let current_id = {
            let state = player.read(cx);
            let valid = state.playback_scope_id() == Some(Self::SCOPE_ID)
                && state.listening_session().is_none()
                && SaiAccountCacheScope::active_user_id() == session.account_id
                && SaiAccountCacheScope::active_epoch() == session.account_epoch
                && state.source_queue().iter().map(|track| &track.id).eq(session.applied_ids.iter());
            state.current_track.as_ref().filter(|_| valid).map(|track| track.id.clone())
        };
        let Some(current_id) = current_id else {
            self.end();
            return;
        };

        let tracks = Self::liked_tracks(cx);
        let ids: Vec<String> = tracks.iter().map(|track| track.id.clone()).collect();
        if ids != session.applied_ids
            && tracks.iter().any(|track| track.id == current_id)
            && player.update(cx, |player, cx| player.replace_ad_hoc_queue(Self::SCOPE_ID, tracks, cx))
        {
            session.applied_ids = ids;
        }
        let page = MusicServicesStore::shared(cx)
            .update(cx, |services, cx| services.pagination_state(&MusicServicesStore::liked_cache_key(), cx));
        if page.has_more {
            Self::request_more(page.cursor, &mut session, cx);
        }

        self.session = Some(session);
        self.observe(expected, cx);
    }

    fn liked_tracks(cx: &mut App) -> Vec<TrackModel> {
        let liked = MusicActionsStore::slices(cx).get_liked_sai.data();
        liked
            .map(|liked| liked.tracks)
            .unwrap_or_default()
            .iter()
            .filter_map(|record| record.as_track_model())
            .collect()
    }

    fn request_more(cursor: Option<String>, session: &mut MusicLikedTracksQueueContinuationSession, cx: &mut App) {
        let pagination = SaiWs::pagination_state(&MusicServicesStore::liked_cache_key(), cx);
        let trailing = pagination.read(cx).trailing().clone();
        if trailing.is_loading {
            return;
        }
        if trailing.error.is_some() {
            if session.retried_cursor == cursor {
                return;
            }
            session.retried_cursor = cursor;
            pagination.update(cx, |pagination, cx| pagination.clear_error(SaiWsPaginationEdge::Trailing, cx));
        }
        MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more_liked(cx));
    }

    /// observation fires once, first change drops the round and reruns sync which registers the next
    fn observe(&mut self, expected: u64, cx: &mut Context<Self>) {
        let pagination = SaiWs::pagination_state(&MusicServicesStore::liked_cache_key(), cx);
        let revisions = SaiCacheRevisionTracker::shared(cx);
        let actions = MusicActionsStore::shared(cx);
        let player = MusicPlayerState::shared(cx);
        self.observations = vec![
            cx.observe(&revisions, move |this, _, cx| this.observed_change(expected, cx)),
            cx.observe(&actions, move |this, _, cx| this.observed_change(expected, cx)),
            cx.observe(&pagination, move |this, _, cx| this.observed_change(expected, cx)),
            cx.observe(&player, move |this, _, cx| this.observed_change(expected, cx)),
        ];
    }

    fn observed_change(&mut self, expected: u64, cx: &mut Context<Self>) {
        self.observations.clear();
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| this.sync(expected, cx));
        })
        .detach();
    }
}
