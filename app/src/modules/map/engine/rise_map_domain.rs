use std::future::Future;
use std::sync::{Arc, LazyLock, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::rise_event::map::MapReaction as RiseEventMapReaction;
use rise_canon::rise_key;
use rise_canon::rise_proto::map::{ProtoFriendPosition, ProtoMapFriend, ProtoMapFriendRequest, ProtoMapUser};
use rise_canon::{RiseEvent, RiseTimingCanon};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_sync::RiseSyncPushHandler;
use rise_engine::{CancellationError, RiseEngine, RiseJSONValue, RiseWirePushEnvelope};
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use tokio::task::JoinHandle;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::map::engine::core::rise_map_resource_repository::RiseMapResourceRepository;
use crate::modules::map::engine::core::rise_map_rpc::RiseMapRPC;
use crate::modules::map::engine::rise_map_drafts_repository::RiseMapDraftsRepository;
use crate::modules::map::engine::rise_map_engine_models::{RiseMapError, RiseMapMutationOutcome};
use crate::modules::map::engine::rise_map_friends_repository::RiseMapFriendsRepository;
use crate::modules::map::engine::rise_map_journeys_repository::RiseMapJourneysRepository;
use crate::modules::map::engine::rise_map_live_repository::RiseMapLiveRepository;
use crate::modules::map::engine::rise_map_photos_repository::RiseMapPhotosRepository;
use crate::modules::map::engine::rise_map_presentation::{RiseMapPresentation, RiseMapSessionIdentity};
use crate::modules::map::engine::rise_map_privacy_repository::RiseMapPrivacyRepository;
use crate::modules::map::engine::rise_map_profiles_repository::RiseMapProfilesRepository;
use crate::modules::map::engine::rise_map_reactions_repository::RiseMapReactionsRepository;
use crate::modules::map::engine::rise_map_state_repository::RiseMapStateRepository;
use crate::modules::map::engine::rise_map_tracks_repository::RiseMapTracksRepository;
use crate::modules::map::recorder::rise_map_recorder::{
    MapRecorderConfiguration, MapRecorderConfigurationHome, RiseMapRecorder,
};
use crate::modules::map::shared::map_gallery_scout::{MapGalleryObserver, MapGalleryScout};
use crate::modules::map::stores::map_actions::map_tracks_types::MapPlaceKind;
use crate::modules::map::stores::map_actions::map_types::{
    MapFriend, MapFriendPosition, MapFriendRequest, MapReaction,
};
use crate::modules::map::stores::map_actions::map_wire_types::{
    ProtoFriendPositionExt, ProtoMapFriendExt, ProtoMapFriendRequestExt, ProtoMapUserExt,
};

static MAINTENANCE_INTERVAL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::freshness_ms("map_track_summary"));

struct RiseMapDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
    is_map_visible: bool,
    visibility_sequence: u64,
    refresh_task: Option<JoinHandle<()>>,
    maintenance_task: Option<JoinHandle<()>>,
    gallery_observer: Option<MapGalleryObserver>,
    recorder_generation: u64,
}

pub struct RiseMapDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub state: Arc<RiseMapStateRepository>,
    pub friends: Arc<RiseMapFriendsRepository>,
    pub reactions: Arc<RiseMapReactionsRepository>,
    pub live: Arc<RiseMapLiveRepository>,
    pub tracks: Arc<RiseMapTracksRepository>,
    pub privacy: Arc<RiseMapPrivacyRepository>,
    pub photos: Arc<RiseMapPhotosRepository>,
    pub journeys: Arc<RiseMapJourneysRepository>,
    pub profiles: Arc<RiseMapProfilesRepository>,
    pub drafts: Arc<RiseMapDraftsRepository>,
    resources: Arc<RiseMapResourceRepository>,
    this: Weak<RiseMapDomain>,
    isolated: RiseIsolated<RiseMapDomainState>,
}

impl RiseMapDomain {
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let session = Uuid::new_v4();
        let identity = RiseMapSessionIdentity {
            account_id: engine.account.account_id.clone(),
            account_epoch: engine.account.epoch.clone(),
            session_id: session,
        };
        let rpc = RiseMapRPC::new(engine.clone());
        let resources = Arc::new(RiseMapResourceRepository::new(
            engine.clone(),
            engine.account.account_id.clone(),
        ));
        let state = Arc::new(RiseMapStateRepository::new(rpc.clone(), resources.clone(), identity.clone()));
        let friends = Arc::new(RiseMapFriendsRepository::new(rpc.clone(), resources.clone(), identity.clone()));
        let reactions = Arc::new(RiseMapReactionsRepository::new(rpc.clone(), resources.clone(), identity.clone()));
        let live = Arc::new(RiseMapLiveRepository::new(
            rpc.clone(),
            resources.clone(),
            identity.clone(),
            reactions.clone(),
        ));
        let tracks = Arc::new(RiseMapTracksRepository::new(
            rpc.clone(),
            engine.clone(),
            resources.clone(),
            identity.clone(),
        ));
        let privacy = Arc::new(RiseMapPrivacyRepository::new(rpc.clone(), resources.clone(), identity.clone()));
        let photos = Arc::new(RiseMapPhotosRepository::new(rpc.clone(), identity.clone()));
        let journeys = Arc::new(RiseMapJourneysRepository::new(rpc.clone(), identity.clone()));
        let profiles = Arc::new(RiseMapProfilesRepository::new(rpc));
        let drafts = Arc::new(RiseMapDraftsRepository::new(resources.clone(), identity.clone()));
        Arc::new_cyclic(|this| Self {
            account_id: identity.account_id,
            account_epoch: identity.account_epoch,
            session_id: session,
            engine,
            state,
            friends,
            reactions,
            live,
            tracks,
            privacy,
            photos,
            journeys,
            profiles,
            drafts,
            resources,
            this: this.clone(),
            isolated: RiseIsolated::new(RiseMapDomainState {
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
                is_map_visible: false,
                visibility_sequence: 0,
                refresh_task: None,
                maintenance_task: None,
                gallery_observer: None,
                recorder_generation: 0,
            }),
        })
    }

    /// reducers install here and never in activate
    /// the runtime snapshots the push claim set once right after bootstrap returns
    pub async fn prepare(&self, context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared) =
            self.isolated.with(|state| (state.is_shutdown, state.is_prepared));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if is_prepared {
            return Ok(());
        }

        let state = self.state.clone();
        let friends = self.friends.clone();
        self.register(RiseMapRealtime::FRIEND_REQUEST_EVENT_TYPE, context, move |envelope| {
            let state = state.clone();
            let friends = friends.clone();
            async move {
                let Some(request) = RiseMapRealtime::decode_request(&envelope) else {
                    return;
                };
                friends.apply_remote_request(request).await;
                RiseAccountRuntime::current().spawn(async move { state.refresh().await });
            }
        })
        .await?;

        let state = self.state.clone();
        let friends = self.friends.clone();
        self.register(RiseMapRealtime::FRIEND_REQUEST_SETTLED_EVENT_TYPE, context, move |envelope| {
            let state = state.clone();
            let friends = friends.clone();
            async move {
                let Some(request_id) = RiseMapRealtime::decode_settled(&envelope) else {
                    return;
                };
                friends.apply_remote_settled(&request_id).await;
                RiseAccountRuntime::current().spawn(async move { state.refresh().await });
            }
        })
        .await?;

        let state = self.state.clone();
        let friends = self.friends.clone();
        let live = self.live.clone();
        self.register(RiseMapRealtime::FRIEND_ACCEPTED_EVENT_TYPE, context, move |envelope| {
            let state = state.clone();
            let friends = friends.clone();
            let live = live.clone();
            async move {
                let Some(friend) = RiseMapRealtime::decode_accepted(&envelope) else {
                    return;
                };
                friends.apply_remote_accepted(friend).await;
                RiseAccountRuntime::current().spawn(async move { state.refresh().await });
                RiseAccountRuntime::current().spawn(async move { live.resume().await });
            }
        })
        .await?;

        let state = self.state.clone();
        let friends = self.friends.clone();
        let live = self.live.clone();
        self.register(RiseMapRealtime::FRIEND_REMOVED_EVENT_TYPE, context, move |envelope| {
            let state = state.clone();
            let friends = friends.clone();
            let live = live.clone();
            async move {
                let Some(user_id) = RiseMapRealtime::decode_user_id(&envelope) else {
                    return;
                };
                friends.apply_remote_removed(&user_id).await;
                live.apply_remote_hidden(&user_id).await;
                RiseAccountRuntime::current().spawn(async move { state.refresh().await });
            }
        })
        .await?;

        let live = self.live.clone();
        let tracks = self.tracks.clone();
        self.register(RiseMapRealtime::FRIEND_MOVED_EVENT_TYPE, context, move |envelope| {
            let live = live.clone();
            let tracks = tracks.clone();
            async move {
                let Some(position) = RiseMapRealtime::decode_moved(&envelope) else {
                    return;
                };
                let user_id = position.user_id.clone();
                live.apply_remote_moved(position).await;
                RiseAccountRuntime::current().spawn(async move { tracks.friend_moved(&user_id).await });
            }
        })
        .await?;

        let live = self.live.clone();
        self.register(RiseMapRealtime::FRIEND_HIDDEN_EVENT_TYPE, context, move |envelope| {
            let live = live.clone();
            async move {
                let Some(user_id) = RiseMapRealtime::decode_user_id(&envelope) else {
                    return;
                };
                live.apply_remote_hidden(&user_id).await;
            }
        })
        .await?;

        let live = self.live.clone();
        self.register(RiseMapRealtime::REACTION_EVENT_TYPE, context, move |envelope| {
            let live = live.clone();
            async move {
                let Some(reaction) = RiseMapRealtime::decode_reaction(&envelope) else {
                    return;
                };
                live.apply_remote_reaction(reaction).await;
            }
        })
        .await?;

        self.isolated.with(|state| state.is_prepared = true);
        Ok(())
    }

    pub async fn activate(&self) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared, is_active) =
            self.isolated.with(|state| (state.is_shutdown, state.is_prepared, state.is_active));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if !is_prepared {
            return Err(RiseMapError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }

        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let is_current_account = main_actor_run(move |_| {
            SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch))
        })
        .await
        .unwrap_or(false);
        if !is_current_account {
            return Err(RiseMapError::StaleAccount.into());
        }

        let activated = self.isolated.with(|state| {
            if state.is_shutdown {
                return false;
            }
            state.is_active = true;
            true
        });
        if !activated {
            return Err(CancellationError.into());
        }
        let Some(domain) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let tile_store = self.tracks.tile_store.clone();
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        main_actor_run(move |cx| {
            RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.activate(&account_id, &account_epoch, session_id, tile_store, cx);
            });
            RiseMapDomainRegistry::shared(cx).update(cx, |registry, cx| registry.install(domain, cx));
        })
        .await;
        self.tracks.load_view_preferences().await;
        if self.isolated.with(|state| state.is_shutdown) {
            return Err(CancellationError.into());
        }
        let on_state_changed = self.this.clone();
        self.state
            .set_on_state_changed(move |_| {
                let domain = on_state_changed.clone();
                async move {
                    if let Some(domain) = domain.upgrade() {
                        domain.sync_recorder().await;
                    }
                }
                .boxed()
            })
            .await;
        let on_home_changed = self.this.clone();
        self.tracks
            .set_on_home_changed(move |_| {
                let domain = on_home_changed.clone();
                async move {
                    if let Some(domain) = domain.upgrade() {
                        domain.sync_recorder().await;
                    }
                }
                .boxed()
            })
            .await;
        self.state.hydrate_from_store().await;
        Ok(())
    }

    pub async fn did_activate_ownership(&self) {
        if !self.is_live() {
            return;
        }
        let domain = self.this.clone();
        self.isolated.with(|state| {
            state.refresh_task = Some(RiseAccountRuntime::current().spawn(async move {
                let Some(domain) = domain.upgrade() else {
                    return;
                };
                domain.state.open(false).await;
                domain.sync_gallery().await;
            }));
        });
    }

    /// started and never awaited
    /// an rpc awaited from the runtime's lifecycle queue blocks delivery of its own reply
    /// viewer resources are revalidated only with a map screen up
    pub async fn socket_connected(&self) {
        if !self.is_live() {
            return;
        }
        let state = self.state.clone();
        let friends = self.friends.clone();
        let live = self.live.clone();
        let tracks = self.tracks.clone();
        self.isolated.with(|domain_state| {
            let is_map_visible = domain_state.is_map_visible;
            if let Some(refresh_task) = domain_state.refresh_task.take() {
                refresh_task.abort();
            }
            domain_state.refresh_task = Some(RiseAccountRuntime::current().spawn(async move {
                if !is_map_visible {
                    return;
                }
                state.open(true).await;
                if !state.is_member().await {
                    return;
                }
                futures::join!(friends.open(true), live.resume(), tracks.revalidate(true));
            }));
        });
    }

    /// foreground edge with a live socket
    /// viewer session lapses after a minute of silence so a visible map takes its picture again
    pub async fn application_did_become_active(&self) {
        if !self.is_live() {
            return;
        }
        let domain = self.this.clone();
        self.isolated.with(|state| {
            if let Some(refresh_task) = state.refresh_task.take() {
                refresh_task.abort();
            }
            state.refresh_task = Some(RiseAccountRuntime::current().spawn(async move {
                let Some(domain) = domain.upgrade() else {
                    return;
                };
                let map_visible = domain.isolated.with(|state| state.is_map_visible);
                domain.state.open(map_visible).await;
                if !domain.state.is_member().await {
                    return;
                }
                domain.sync_gallery().await;
                if !domain.isolated.with(|state| state.is_map_visible) {
                    return;
                }
                futures::join!(
                    domain.live.resume(),
                    domain.tracks.revalidate(true),
                    domain.photos.revalidate(true),
                );
            }));
        });
    }

    async fn sync_gallery(&self) {
        if !self.is_live() {
            return;
        }
        let Some(current) = self.state.current().await else {
            return;
        };
        if !current.is_member || self.isolated.with(|state| state.is_shutdown) {
            return;
        }
        if MapGalleryScout::has_full_access() {
            let domain = self.this.clone();
            self.isolated.with(|state| {
                if state.gallery_observer.is_some() {
                    return;
                }
                state.gallery_observer =
                    Some(MapGalleryObserver::new(move || spawn_sync_gallery(domain.clone())));
            });
        }
        self.drafts.scan(current.joined_at).await;
    }

    fn start_maintenance(&self) {
        let domain = self.this.clone();
        self.isolated.with(|state| {
            if let Some(maintenance_task) = state.maintenance_task.take() {
                maintenance_task.abort();
            }
            state.maintenance_task = Some(RiseAccountRuntime::current().spawn(async move {
                loop {
                    let interval = u64::try_from(*MAINTENANCE_INTERVAL_MS).unwrap_or(0);
                    tokio::time::sleep(Duration::from_millis(interval)).await;
                    let Some(domain) = domain.upgrade() else {
                        return;
                    };
                    domain.refresh_visible_resources().await;
                }
            }));
        });
    }

    async fn refresh_visible_resources(&self) {
        if !self.isolated.with(|state| state.is_active && !state.is_shutdown && state.is_map_visible) {
            return;
        }
        if !self.engine.is_ready_for_calls(false).await.unwrap_or(false) {
            return;
        }
        futures::join!(self.tracks.revalidate(true), self.photos.revalidate(false));
    }

    pub async fn set_map_visible(&self, visible: bool, sequence: u64) {
        let accepted = self.isolated.with(|state| {
            if !state.is_active || state.is_shutdown || sequence <= state.visibility_sequence {
                return false;
            }
            state.visibility_sequence = sequence;
            state.is_map_visible = visible;
            true
        });
        if !accepted {
            return;
        }
        self.tracks.set_watching(visible, sequence).await;
        if !self.isolated.with(|state| !state.is_shutdown && sequence == state.visibility_sequence) {
            return;
        }
        if visible {
            self.start_maintenance();
            spawn_sync_gallery(self.this.clone());
        } else {
            self.isolated.with(|state| {
                if let Some(maintenance_task) = state.maintenance_task.take() {
                    maintenance_task.abort();
                }
            });
        }
        self.live.set_watching(visible, sequence).await;
    }

    pub async fn open(&self, force: bool) {
        if !self.is_live() {
            return;
        }
        self.state.hydrate_from_store().await;
        if self.state.is_member().await {
            self.friends.hydrate_from_store().await;
        }
        self.state.open(force).await;
        if !self.state.is_member().await {
            return;
        }
        self.sync_gallery().await;
        futures::join!(
            self.friends.open(force),
            self.tracks.revalidate(force),
            self.photos.revalidate(force),
        );
        self.tracks.drain_uploads(false).await;
    }

    /// recorder is configured from map state and home
    /// members with recording or sharing on and nobody else
    async fn sync_recorder(&self) {
        let generation = self.isolated.with(|state| {
            if !state.is_active || state.is_shutdown {
                return None;
            }
            state.recorder_generation = state.recorder_generation.wrapping_add(1);
            Some(state.recorder_generation)
        });
        let Some(generation) = generation else {
            return;
        };
        let current = self.state.current().await;
        let home = self.tracks.current_home().await;
        let places = self.tracks.current_places().await;
        let account_id = self.account_id.clone();
        let home_is_known = self.tracks.has_loaded_home().await;
        let mut configuration = current.as_ref().and_then(|state| {
            if !state.is_member {
                return None;
            }
            Some(MapRecorderConfiguration {
                account_id: account_id.clone(),
                tracks_enabled: state.settings.tracks_enabled,
                sharing_enabled: state.settings.sharing_enabled,
                battery_visible: state.settings.battery_visible,
                home: home.as_ref().map(|home| MapRecorderConfigurationHome {
                    latitude: home.latitude,
                    longitude: home.longitude,
                    radius_meters: home.radius_meters as f64,
                }),
                home_track_exclusion_radius_m: Some(state.settings.home_track_exclusion_radius_m),
                place_track_zones: Some(
                    places
                        .iter()
                        .filter(|place| place.kind == MapPlaceKind::Custom && !place.record_tracks)
                        .map(|place| MapRecorderConfigurationHome {
                            latitude: place.latitude,
                            longitude: place.longitude,
                            radius_meters: place.track_exclusion_radius_meters as f64,
                        })
                        .collect(),
                ),
                precise_recording: Some(state.settings.track_precise_recording),
            })
        });
        if !self.is_current_generation(generation) {
            return;
        }
        let identity_account = self.account_id.clone();
        let identity_epoch = self.account_epoch.clone();
        if !home_is_known {
            let account = identity_account.clone();
            let saved = main_actor_run(move |cx| {
                let recorder = RiseMapRecorder::shared(cx);
                let recorder = recorder.read(cx);
                (recorder.home(&account), recorder.place_track_zones(&account))
            })
            .await;
            if let (Some(configuration), Some((saved_home, saved_zones))) = (configuration.as_mut(), saved) {
                configuration.home = saved_home;
                configuration.place_track_zones = saved_zones;
            }
        }
        let next_configuration = configuration;
        if !self.is_current_generation(generation) {
            return;
        }
        main_actor_run(move |cx| {
            if !SaiAccountCacheScope::is_current_account_epoch(Some(&identity_account), Some(&identity_epoch)) {
                return;
            }
            RiseMapRecorder::shared(cx).update(cx, |recorder, cx| recorder.apply(next_configuration, cx));
        })
        .await;
        if current.is_some_and(|state| state.is_member) {
            spawn_sync_gallery(self.this.clone());
        } else {
            self.isolated.with(|state| {
                if let Some(observer) = state.gallery_observer.take() {
                    observer.stop();
                }
            });
        }
    }

    pub async fn join(&self) -> RiseMapMutationOutcome {
        if !self.is_live() {
            return RiseMapMutationOutcome::Failed(None);
        }
        let outcome = self.state.join().await;
        if outcome == RiseMapMutationOutcome::Done {
            let live = self.live.clone();
            RiseAccountRuntime::current().spawn(async move { live.resume().await });
            self.friends.open(true).await;
        }
        outcome
    }

    pub async fn leave(&self) -> RiseMapMutationOutcome {
        if !self.is_live() {
            return RiseMapMutationOutcome::Failed(None);
        }
        let outcome = self.state.leave().await;
        if outcome == RiseMapMutationOutcome::Done {
            self.reactions.reset().await;
            self.live.reset().await;
            self.friends.reset().await;
            self.tracks.reset().await;
            self.privacy.reset().await;
            self.photos.reset().await;
            self.drafts.reset().await;
            let account_id = self.account_id.clone();
            main_actor_run(move |cx| {
                RiseMapRecorder::shared(cx).update(cx, |recorder, cx| {
                    recorder.apply(None, cx);
                    recorder.erase_journal(&account_id, cx);
                });
            })
            .await;
        }
        outcome
    }

    pub async fn shutdown(&self) {
        let tasks = self.isolated.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.is_shutdown = true;
            state.recorder_generation = state.recorder_generation.wrapping_add(1);
            let maintenance_task = state.maintenance_task.take();
            let gallery_observer = state.gallery_observer.take();
            state.is_active = false;
            let refresh_task = state.refresh_task.take();
            state.is_map_visible = false;
            Some((maintenance_task, gallery_observer, refresh_task))
        });
        let Some((maintenance_task, gallery_observer, refresh_task)) = tasks else {
            return;
        };
        if let Some(maintenance_task) = maintenance_task {
            maintenance_task.abort();
        }
        if let Some(gallery_observer) = gallery_observer {
            gallery_observer.stop();
        }
        if let Some(refresh_task) = refresh_task {
            refresh_task.abort();
        }
        self.journeys.shutdown().await;
        self.reactions.shutdown().await;
        self.live.shutdown();
        self.tracks.shutdown().await;
        self.privacy.shutdown().await;
        self.photos.shutdown();
        self.profiles.shutdown().await;
        self.drafts.shutdown().await;
        self.friends.shutdown();
        self.state.shutdown().await;
        self.resources.shutdown().await;
        let domain = self.this.clone();
        let account_id = self.account_id.clone();
        let session_id = self.session_id;
        main_actor_run(move |cx| {
            RiseMapDomainRegistry::shared(cx).update(cx, |registry, cx| registry.remove(&domain, cx));
            RiseMapPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.deactivate(&account_id, session_id, cx);
            });
        })
        .await;
    }

    async fn register<Reduce, Reduction>(
        &self,
        event_type: &str,
        context: &RiseAppEngineBootstrapContext,
        reduce: Reduce,
    ) -> anyhow::Result<()>
    where
        Reduce: Fn(RiseWirePushEnvelope) -> Reduction + Send + Sync + 'static,
        Reduction: Future<Output = ()> + Send + 'static,
    {
        context
            .register_push_handler(RiseSyncPushHandler::new(event_type, move |envelope| {
                let reduction = reduce(envelope);
                async move {
                    reduction.await;
                    Ok(None)
                }
            }))
            .await
    }

    fn is_shut_down(&self) -> bool {
        self.isolated.with(|state| state.is_shutdown)
    }

    fn is_live(&self) -> bool {
        self.isolated.with(|state| state.is_active && !state.is_shutdown)
    }

    fn is_current_generation(&self, generation: u64) -> bool {
        self.isolated
            .with(|state| !state.is_shutdown && generation == state.recorder_generation)
    }
}

/// spawns a gallery sync task that holds the domain weakly
fn spawn_sync_gallery(domain: Weak<RiseMapDomain>) {
    RiseAccountRuntime::current().spawn(async move {
        if let Some(domain) = domain.upgrade() {
            domain.sync_gallery().await;
        }
    });
}

/// runs work on the main thread with the app
/// none means the main thread is gone
fn main_actor_run<R, W>(work: W) -> BoxFuture<'static, Option<R>>
where
    R: Send + 'static,
    W: FnOnce(&mut App) -> R + Send + 'static,
{
    let Some(main) = RiseEngineMainHop::installed() else {
        return async { None }.boxed();
    };
    let run = main.run(move |cx: AsyncApp| async move { Ok(cx.update(work)) }.boxed_local());
    async move { run.await.ok() }.boxed()
}

pub struct RiseMapDomainRegistry {
    active: Option<Arc<RiseMapDomain>>,
}

impl RiseMapDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseMapDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseMapDomain>, cx: &mut Context<Self>) {
        if domain.is_shut_down() {
            return;
        }
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RiseMapDomain>, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        if !std::ptr::eq(Arc::as_ptr(active), domain.as_ptr()) {
            return;
        }
        self.active = None;
        cx.notify();
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseMapDomain>>> {
        if self.active.as_ref().is_some_and(|active| active.is_shut_down()) {
            self.active = None;
        }
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx)
            .update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseMapError::MissingActiveSession.into())
        })
    }
}

pub enum RiseMapActionExecutor {}

impl RiseMapActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseMapDomain>>> {
        let account = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        let domain = RiseMapDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx));
        cx.spawn(async move |_| {
            let domain = domain.await?;
            let is_current = Some(&domain.account_id) == account.as_ref()
                && Some(&domain.account_epoch) == epoch.as_ref()
                && SaiAccountCacheScope::is_current_account_epoch(account.as_deref(), epoch.as_deref());
            if !is_current {
                return Err(RiseMapError::StaleAccount.into());
            }
            Ok(domain)
        })
    }
}

/// pure and total, a reducer that throws makes the sync require a full resync
/// gateway may lay the event flat into data or nest it one level
/// so payloads are looked up at the root first
pub enum RiseMapRealtime {}

impl RiseMapRealtime {
    pub const FRIEND_REQUEST_EVENT_TYPE: &'static str = rise_events::name::MAP_FRIEND_REQUEST;
    pub const FRIEND_REQUEST_SETTLED_EVENT_TYPE: &'static str = rise_events::name::MAP_FRIEND_REQUEST_SETTLED;
    pub const FRIEND_ACCEPTED_EVENT_TYPE: &'static str = rise_events::name::MAP_FRIEND_ACCEPTED;
    pub const FRIEND_REMOVED_EVENT_TYPE: &'static str = rise_events::name::MAP_FRIEND_REMOVED;
    pub const FRIEND_MOVED_EVENT_TYPE: &'static str = rise_events::name::MAP_FRIEND_MOVED;
    pub const FRIEND_HIDDEN_EVENT_TYPE: &'static str = rise_events::name::MAP_FRIEND_HIDDEN;
    pub const REACTION_EVENT_TYPE: &'static str = rise_events::name::MAP_REACTION;

    fn event(envelope: &RiseWirePushEnvelope) -> Option<RiseEvent> {
        let data = envelope.data.data(false).ok()?;
        RiseEvent::decode(&envelope.r#type, &data).ok()
    }

    fn now_ms() -> i64 {
        (Date::now().time_interval_since_1970() * 1_000.0) as i64
    }

    pub fn decode_location_demand(envelope: &RiseWirePushEnvelope) -> Option<i64> {
        if let Some(RiseEvent::MapLocationRequested(demand)) = Self::event(envelope) {
            return (demand.expires_at > Self::now_ms()).then_some(demand.expires_at);
        }
        let root = Self::payload(envelope, rise_key::events::EXPIRES_AT)?;
        let RiseJSONValue::Integer(expires_at) = root.get(rise_key::bot::EXPIRES_AT)? else {
            return None;
        };
        (*expires_at > Self::now_ms()).then_some(*expires_at)
    }

    pub fn decode_request(envelope: &RiseWirePushEnvelope) -> Option<MapFriendRequest> {
        Self::decode::<ProtoMapFriendRequest>(rise_key::map::REQUEST, rise_key::map::FROM_USER, envelope)?
            .model()
    }

    pub fn decode_accepted(envelope: &RiseWirePushEnvelope) -> Option<MapFriend> {
        Self::decode::<ProtoMapFriend>(rise_key::map::FRIEND, rise_key::map::USER, envelope)?.model()
    }

    pub fn decode_moved(envelope: &RiseWirePushEnvelope) -> Option<MapFriendPosition> {
        Self::decode::<ProtoFriendPosition>(rise_key::map::POSITION, rise_key::map::FIX, envelope)?.model()
    }

    pub fn decode_reaction(envelope: &RiseWirePushEnvelope) -> Option<MapReaction> {
        let Some(RiseEvent::MapReaction(reaction)) = Self::event(envelope) else {
            return None;
        };
        Self::reaction_model(reaction)
    }

    fn reaction_model(reaction: RiseEventMapReaction) -> Option<MapReaction> {
        if reaction.emoji.is_empty() {
            return None;
        }
        let from_user = reaction
            .from_user
            .as_ref()
            .and_then(|value| RiseJSONValue::encode(value).ok())
            .and_then(|value| value.decode_as::<ProtoMapUser>().ok())
            .and_then(|user| user.model())?;
        Some(MapReaction {
            id: Uuid::new_v4(),
            from_user,
            emoji: reaction.emoji,
            sent_at: reaction.sent_at,
            count: reaction.count.max(1),
        })
    }

    pub fn decode_settled(envelope: &RiseWirePushEnvelope) -> Option<String> {
        if let Some(RiseEvent::MapFriendRequestSettled(settled)) = Self::event(envelope) {
            return (!settled.request_id.is_empty()).then_some(settled.request_id);
        }
        let root = Self::payload(envelope, rise_key::map::REQUEST_ID)?;
        Self::string(root.get(rise_key::bot::REQUEST_ID))
    }

    pub fn decode_user_id(envelope: &RiseWirePushEnvelope) -> Option<String> {
        match Self::event(envelope) {
            Some(RiseEvent::MapFriendRemoved(removed)) => {
                return (!removed.user_id.is_empty()).then_some(removed.user_id);
            }
            Some(RiseEvent::MapFriendHidden(hidden)) => {
                return (!hidden.user_id.is_empty()).then_some(hidden.user_id);
            }
            _ => {}
        }
        let root = Self::payload(envelope, rise_key::map::USER_ID)?;
        Self::string(root.get(rise_key::bot::USER_ID))
    }

    /// key holds the object when the frame nests it, shape_key recognises it laid flat
    fn decode<Value: DeserializeOwned>(
        key: &str,
        shape_key: &str,
        envelope: &RiseWirePushEnvelope,
    ) -> Option<Value> {
        let nested = Self::payload(envelope, key)
            .and_then(|root| root.get(key))
            .filter(|nested| !matches!(nested, RiseJSONValue::Null));
        let source = match nested {
            Some(nested) => nested.clone(),
            None => RiseJSONValue::Object(Self::payload(envelope, shape_key)?.clone()),
        };
        source.decode_as::<Value>().ok()
    }

    fn payload<'a>(
        envelope: &'a RiseWirePushEnvelope,
        key: &str,
    ) -> Option<&'a BTreeMap<String, RiseJSONValue>> {
        let RiseJSONValue::Object(root) = &envelope.data else {
            return None;
        };
        if root.contains_key(key) {
            return Some(root);
        }
        if let Some(RiseJSONValue::Object(inner)) = root.get(rise_key::bot::DATA)
            && inner.contains_key(key)
        {
            return Some(inner);
        }
        None
    }

    fn string(value: Option<&RiseJSONValue>) -> Option<String> {
        match value? {
            RiseJSONValue::String(text) => (!text.is_empty()).then(|| text.clone()),
            RiseJSONValue::Integer(number) => Some(number.to_string()),
            RiseJSONValue::UnsignedInteger(number) => Some(number.to_string()),
            _ => None,
        }
    }
}
