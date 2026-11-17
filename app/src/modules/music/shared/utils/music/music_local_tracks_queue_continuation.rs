use gpui::{App, Context, Entity, Subscription};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pagination::SaiWsPaginationEdge;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

struct MusicLocalTracksQueueContinuationSession {
    account_id: Option<String>,
    account_epoch: Option<String>,
    applied_ids: Vec<String>,
    retried_track_id: Option<String>,
}

/// grows the local tracks queue to the whole list while the same track plays
/// next page is asked for when playback nears the end
pub struct MusicLocalTracksQueueContinuation {
    session: Option<MusicLocalTracksQueueContinuationSession>,
    generation: u64,
    observations: Vec<Subscription>,
}

impl MusicLocalTracksQueueContinuation {
    pub const SCOPE_ID: &'static str = "local_tracks";
    const END_DISTANCE: usize = 2;

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
        self.session = Some(MusicLocalTracksQueueContinuationSession {
            account_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
            applied_ids: state.source_queue().iter().map(|track| track.id.clone()).collect(),
            retried_track_id: None,
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

        let (tracks, has_more) = MusicServicesStore::shared(cx)
            .update(cx, |services, cx| (services.local_tracks(cx), services.local_tracks_page(cx).has_more));
        if let Some(anchor) = tracks.iter().position(|track| track.id == current_id) {
            let count = tracks.len();
            let ids: Vec<String> = tracks.iter().map(|track| track.id.clone()).collect();
            if ids != session.applied_ids
                && player.update(cx, |player, cx| player.replace_ad_hoc_queue(Self::SCOPE_ID, tracks, cx))
            {
                session.applied_ids = ids;
            }
            if has_more && anchor as i64 >= count as i64 - 1 - Self::END_DISTANCE as i64 {
                Self::request_more(&current_id, &mut session, cx);
            }
        } else if has_more {
            Self::request_more(&current_id, &mut session, cx);
        }

        self.session = Some(session);
        self.observe(expected, cx);
    }

    fn request_more(track_id: &str, session: &mut MusicLocalTracksQueueContinuationSession, cx: &mut App) {
        let pagination = SaiWs::pagination_state(&MusicServicesStore::my_local_tracks_cache_key(), cx);
        let trailing = pagination.read(cx).trailing().clone();
        if trailing.is_loading {
            return;
        }
        if trailing.error.is_some() {
            if session.retried_track_id.as_deref() == Some(track_id) {
                return;
            }
            session.retried_track_id = Some(track_id.to_owned());
            pagination.update(cx, |pagination, cx| pagination.clear_error(SaiWsPaginationEdge::Trailing, cx));
        }
        MusicInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.load_more_my_local_tracks(cx));
    }

    /// observation fires once, first change drops the round and reruns sync which registers the next
    fn observe(&mut self, expected: u64, cx: &mut Context<Self>) {
        let pagination = SaiWs::pagination_state(&MusicServicesStore::my_local_tracks_cache_key(), cx);
        let revisions = SaiCacheRevisionTracker::shared(cx);
        let services = MusicServicesStore::shared(cx);
        let player = MusicPlayerState::shared(cx);
        self.observations = vec![
            cx.observe(&revisions, move |this, _, cx| this.observed_change(expected, cx)),
            cx.observe(&services, move |this, _, cx| this.observed_change(expected, cx)),
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
