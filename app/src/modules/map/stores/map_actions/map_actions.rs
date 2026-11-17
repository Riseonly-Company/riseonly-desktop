use std::collections::HashSet;

use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::modules::map::components::map_editing_session::{MapEditingSession, MapTrackPeriod};
use crate::modules::map::engine::map_track_erasure_geometry::MapEraseStroke;
use crate::modules::map::engine::rise_map_domain::{RiseMapActionExecutor, RiseMapDomainRegistry};
use crate::modules::map::engine::rise_map_engine_models::{
    RiseMapFriendRequestResult, RiseMapInvitePreviewResult, RiseMapMergePreviewResult, RiseMapMutationOutcome,
};
use crate::modules::map::engine::rise_map_friends_repository::RiseMapFriendsRepositoryList;
use crate::modules::map::engine::rise_map_journeys_repository::{MapJourneySelection, MapJourneySnapshot};
use crate::modules::map::engine::rise_map_reactions_repository::MapReactionPage;
use crate::modules::map::stores::map_actions::map_photos_types::MapPhotoDraft;
use crate::modules::map::stores::map_actions::map_privacy_types::{MapPrivacyAction, MapPrivacyKind, MapPrivacyRule};
use crate::modules::map::stores::map_actions::map_tracks_types::MapTileKey;
use crate::modules::map::stores::map_actions::map_types::{
    MapInvite, MapMergePreviewRequest, MapSettingsPatch, MapUser,
};
use crate::modules::map::stores::map_services::map_services::MapServicesStore;

pub struct MapActionsStore {}

impl MapActionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {}
    }

    pub fn open_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.open(force).await;
        })
        .detach();
    }

    pub fn set_map_visible_action(&mut self, visible: bool, sequence: u64, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.set_map_visible(visible, sequence).await;
        })
        .detach();
    }

    pub fn join_action(&mut self, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            domain.join().await
        })
    }

    pub fn leave_action(&mut self, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            domain.leave().await
        })
    }

    pub fn update_settings_action(
        &mut self,
        patch: MapSettingsPatch,
        cx: &mut Context<Self>,
    ) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let domain = match domain.await {
                Ok(domain) => domain,
                Err(error) => {
                    tracing::warn!(target: "riseonly::X1453", error = %format!("{error:#}"), "map settings update without map domain");
                    return RiseMapMutationOutcome::Failed(None);
                }
            };
            let redraws_lines = patch.redraws_lines();
            let outcome = domain.state.update_settings(patch).await;
            if outcome == RiseMapMutationOutcome::Done && redraws_lines {
                let tracks = domain.tracks.clone();
                cx.spawn(async move |_| tracks.revalidate(true).await).detach();
            }
            outcome
        })
    }

    pub fn preview_track_merge_action(
        &mut self,
        request: MapMergePreviewRequest,
        cx: &mut Context<Self>,
    ) -> Task<RiseMapMergePreviewResult> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMergePreviewResult::Failed(None);
            };
            domain.tracks.preview_merge(request).await
        })
    }

    pub fn load_more_action(&mut self, list: RiseMapFriendsRepositoryList, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.friends.load_more(list).await;
        })
        .detach();
    }

    pub fn resume_friends_action(&mut self, lists: Vec<RiseMapFriendsRepositoryList>, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.open(false).await;
            for list in lists {
                domain.friends.load_more(list).await;
            }
        })
        .detach();
    }

    pub fn send_friend_request_action(
        &mut self,
        target_user_id: String,
        cx: &mut Context<Self>,
    ) -> Task<RiseMapFriendRequestResult> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapFriendRequestResult::Failed(None);
            };
            let result = domain.friends.send_request(&target_user_id).await;
            let state = domain.state.clone();
            cx.spawn(async move |_| state.refresh().await).detach();
            result
        })
    }

    pub fn respond_friend_request_action(
        &mut self,
        request_id: String,
        accept: bool,
        cx: &mut Context<Self>,
    ) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            let outcome = domain.friends.respond(&request_id, accept).await;
            let state = domain.state.clone();
            cx.spawn(async move |_| state.refresh().await).detach();
            outcome
        })
    }

    pub fn cancel_friend_request_action(
        &mut self,
        request_id: String,
        cx: &mut Context<Self>,
    ) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            domain.friends.cancel(&request_id).await
        })
    }

    pub fn remove_friend_action(&mut self, user_id: String, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            let outcome = domain.friends.remove_friend(&user_id).await;
            if outcome == RiseMapMutationOutcome::Done {
                domain.live.apply_remote_hidden(&user_id).await;
                domain.profiles.remove_friend(&user_id, &domain.account_id).await;
            }
            let state = domain.state.clone();
            cx.spawn(async move |_| state.refresh().await).detach();
            outcome
        })
    }

    pub fn load_invite_action(&mut self, force: bool, cx: &mut Context<Self>) -> Task<Option<MapInvite>> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return None;
            };
            domain.friends.load_invite(force).await
        })
    }

    pub fn reset_invite_action(&mut self, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            domain.friends.reset_invite().await
        })
    }

    pub fn preview_invite_action(&mut self, token: String, cx: &mut Context<Self>) -> Task<RiseMapInvitePreviewResult> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapInvitePreviewResult::Failed(None);
            };
            domain.friends.preview_invite(&token).await
        })
    }

    pub fn use_invite_action(&mut self, token: String, cx: &mut Context<Self>) -> Task<RiseMapFriendRequestResult> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapFriendRequestResult::Failed(None);
            };
            let result = domain.friends.use_invite(&token).await;
            let state = domain.state.clone();
            cx.spawn(async move |_| state.refresh().await).detach();
            result
        })
    }

    pub fn claim_reactions_action(
        &mut self,
        sender_id: String,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<MapReactionPage>> {
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = domain.await?;
            if Some(&domain.account_id) != account.as_ref() || Some(&domain.account_epoch) != epoch.as_ref() {
                return Err(CancellationError.into());
            }
            domain.reactions.claim(&sender_id).await
        })
    }

    pub fn reaction_page_action(
        &mut self,
        sender_id: String,
        claim_id: String,
        after: i64,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<MapReactionPage>> {
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = domain.await?;
            if Some(&domain.account_id) != account.as_ref() || Some(&domain.account_epoch) != epoch.as_ref() {
                return Err(CancellationError.into());
            }
            domain.reactions.page(&sender_id, &claim_id, after).await
        })
    }

    pub fn enqueue_reaction_action(&mut self, target: String, emoji: String, cx: &mut Context<Self>) -> Task<bool> {
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return false;
            };
            if Some(&domain.account_id) != account.as_ref() || Some(&domain.account_epoch) != epoch.as_ref() {
                return false;
            }
            domain.reactions.enqueue(&target, &emoji).await
        })
    }

    pub fn open_tracks_action(&mut self, owner_id: Option<String>, force: bool, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.tracks.open(owner_id, force).await;
        })
        .detach();
    }

    pub fn set_track_period_action(&mut self, period: MapTrackPeriod, cx: &mut Context<Self>) {
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            if Some(&domain.account_id) != account.as_ref() || Some(&domain.account_epoch) != epoch.as_ref() {
                return;
            }
            domain.tracks.set_period(period).await;
        })
        .detach();
    }

    pub fn toggle_track_lines_action(&mut self, cx: &mut Context<Self>) -> Task<Option<bool>> {
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return None;
            };
            if Some(&domain.account_id) != account.as_ref() || Some(&domain.account_epoch) != epoch.as_ref() {
                return None;
            }
            domain.tracks.toggle_lines_visibility().await
        })
    }

    pub fn show_track_lines_action(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            if Some(&domain.account_id) != account.as_ref() || Some(&domain.account_epoch) != epoch.as_ref() {
                return;
            }
            domain.tracks.set_lines_visible(true).await;
        })
    }

    pub fn request_track_tiles_action(&mut self, keys: Vec<MapTileKey>, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.tracks.request(keys).await;
        })
        .detach();
    }

    pub fn apply_own_track_colouring_action(&mut self, cooling_days: i64, heat_walks: i64, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.tracks.apply_own_colouring(cooling_days, heat_walks).await;
        })
        .detach();
    }

    pub fn open_privacy_action(&mut self, force: bool, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.privacy.open(force).await;
        })
        .detach();
    }

    pub fn set_privacy_rule_action(
        &mut self,
        rule: MapPrivacyRule,
        kind: MapPrivacyKind,
        cx: &mut Context<Self>,
    ) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = match domain.await {
                Ok(domain) => domain,
                Err(error) => {
                    tracing::warn!(target: "riseonly::X1174", error = %format!("{error:#}"), "privacy edit without map domain");
                    return RiseMapMutationOutcome::Failed(None);
                }
            };
            domain.privacy.set_rule(rule, kind).await
        })
    }

    pub fn set_privacy_users_action(
        &mut self,
        users: Vec<MapUser>,
        kind: MapPrivacyKind,
        action: MapPrivacyAction,
        cx: &mut Context<Self>,
    ) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = match domain.await {
                Ok(domain) => domain,
                Err(error) => {
                    tracing::warn!(target: "riseonly::X1174", error = %format!("{error:#}"), "privacy edit without map domain");
                    return RiseMapMutationOutcome::Failed(None);
                }
            };
            domain.privacy.set_users(users, kind, action).await
        })
    }

    pub fn set_home_action(&mut self, latitude: f64, longitude: f64, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            domain.tracks.set_home(latitude, longitude).await
        })
    }

    pub fn delete_home_action(&mut self, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            domain.tracks.delete_home().await
        })
    }

    pub fn erase_tracks_action(
        &mut self,
        strokes: Vec<MapEraseStroke>,
        session: Entity<MapEditingSession>,
        cx: &mut Context<Self>,
    ) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            let is_session_current = cx.update(|cx| {
                let session = session.read(cx);
                session.is_current()
                    && session.account_id.as_ref() == Some(&domain.account_id)
                    && session.account_epoch.as_ref() == Some(&domain.account_epoch)
            });
            if !is_session_current {
                return RiseMapMutationOutcome::Failed(None);
            }
            domain.tracks.erase_tracks(strokes).await
        })
    }

    pub fn delete_all_tracks_action(&mut self, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            domain.tracks.delete_all_tracks().await
        })
    }

    pub fn open_photos_action(&mut self, owner_id: Option<String>, force: bool, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.photos.open(owner_id.as_deref(), force).await;
        })
        .detach();
    }

    pub fn add_photo_action(&mut self, draft: MapPhotoDraft, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            if Some(&domain.account_id) != account.as_ref()
                || Some(&domain.account_epoch) != epoch.as_ref()
                || !SaiAccountCacheScope::is_current_account_epoch(account.as_deref(), epoch.as_deref())
            {
                return RiseMapMutationOutcome::Failed(None);
            }
            let result = domain.photos.add(draft).await;
            if result == RiseMapMutationOutcome::Done {
                let grids = domain.photos.grids_snapshot();
                domain.profiles.synchronize_grids(grids).await;
            }
            result
        })
    }

    pub fn delete_photo_action(&mut self, id: String, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            let grid_id =
                cx.update(|cx| MapServicesStore::photos(cx).open_grid.map(|open_grid| open_grid.grid_id));
            let result = domain.photos.delete(&id).await;
            if result == RiseMapMutationOutcome::Done {
                match grid_id {
                    Some(grid_id) => match domain.photos.grid(&grid_id) {
                        Some(grid) => domain.profiles.update_grid(grid).await,
                        None => domain.profiles.remove_grid(&grid_id).await,
                    },
                    None => {
                        let grids = domain.photos.grids_snapshot();
                        domain.profiles.synchronize_grids(grids).await;
                    }
                }
            }
            result
        })
    }

    pub fn scan_gallery_action(&mut self, joined_at_ms: i64, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.drafts.scan(joined_at_ms).await;
        })
        .detach();
    }

    pub fn mark_draft_publishing_action(&mut self, id: String, is_publishing: bool, cx: &mut Context<Self>) -> Task<()> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.drafts.mark_publishing(&id, is_publishing).await;
        })
    }

    pub fn remove_drafts_action(&mut self, ids: HashSet<String>, cx: &mut Context<Self>) -> Task<()> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.drafts.remove(ids).await;
        })
    }

    pub fn remove_all_drafts_action(&mut self, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.drafts.remove_all().await;
        })
        .detach();
    }

    pub fn open_grid_action(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.photos.open_grid(id.as_deref()).await;
        })
        .detach();
    }

    pub fn load_more_grid_photos_action(&mut self, cx: &mut Context<Self>) {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.photos.load_more_open_photos().await;
        })
        .detach();
    }

    pub fn delete_grid_action(&mut self, id: String, cx: &mut Context<Self>) -> Task<RiseMapMutationOutcome> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return RiseMapMutationOutcome::Failed(None);
            };
            let outcome = domain.photos.delete_grid(&id).await;
            if outcome == RiseMapMutationOutcome::Done {
                domain.profiles.remove_grid(&id).await;
            }
            outcome
        })
    }

    pub fn prepare_journey_action(
        &mut self,
        selection: MapJourneySelection,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<MapJourneySnapshot>> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = domain.await?;
            domain.journeys.prepare(&selection).await
        })
    }

    pub fn open_journey_action(&mut self, post_id: i64, scope: Uuid, cx: &mut Context<Self>) -> Task<()> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let Ok(domain) = domain.await else {
                return;
            };
            domain.journeys.open(post_id, scope).await;
        })
    }

    pub fn close_journey_action(&mut self, scope: Uuid, cx: &mut Context<Self>) -> Task<()> {
        let Some(domain) = RiseMapDomainRegistry::shared(cx).read(cx).active().cloned() else {
            return Task::ready(());
        };
        cx.spawn(async move |_, _: &mut AsyncApp| {
            domain.journeys.close(Some(scope)).await;
        })
    }

    pub fn revoke_journey_action(&mut self, id: String, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        let domain = RiseMapActionExecutor::require_domain(cx);
        cx.spawn(async move |_, _: &mut AsyncApp| {
            let domain = domain.await?;
            domain.journeys.revoke(&id).await
        })
    }
}
