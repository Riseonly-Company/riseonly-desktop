use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::{CancellationError, RiseEngine};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiNavigationBridge;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::music::engine::rise_music_presentation::RiseMusicPresentation;
use crate::modules::music::engine::rise_music_repository::{
    RiseMusicEditError, RiseMusicLocalCover, RiseMusicRepository,
};
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;

const MUSIC_SCREEN_PREFIX: &str = "Music";
const AUDIO_PAGE_SCREEN: &str = "AudioPage";
const STUDIO_TRACK_DETAIL_SCREEN: &str = "StudioTrackDetail";

struct RiseMusicDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
}

pub struct RiseMusicDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub repository: Arc<RiseMusicRepository>,
    this: Weak<RiseMusicDomain>,
    state: RiseIsolated<RiseMusicDomainState>,
}

impl RiseMusicDomain {
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let session_id = Uuid::new_v4();
        let repository = Arc::new(RiseMusicRepository::new(
            engine,
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        ));
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id,
            repository,
            this: this.clone(),
            state: RiseIsolated::new(RiseMusicDomainState {
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
            }),
        })
    }

    pub async fn prepare(&self, _context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        self.state.with(|state| {
            if state.is_shutdown {
                return Err(CancellationError.into());
            }
            state.is_prepared = true;
            Ok(())
        })
    }

    pub async fn activate(&self) -> anyhow::Result<()> {
        let (is_prepared, is_shutdown, is_active) =
            self.state.with(|state| (state.is_prepared, state.is_shutdown, state.is_active));
        if !is_prepared || is_shutdown {
            return Err(CancellationError.into());
        }
        if is_active {
            return Ok(());
        }
        let is_current = SaiAccountCacheScope::is_current_account_epoch(
            Some(&self.account_id),
            Some(&self.account_epoch),
        );
        if !is_current {
            return Err(RiseMusicEditError::StaleAccount.into());
        }
        self.state.with(|state| state.is_active = true);
        let Some(domain) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let Some(main) = RiseEngineMainHop::installed() else {
            return Ok(());
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        main.run(move |cx: AsyncApp| {
            async move {
                cx.update(|cx| {
                    RiseMusicPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                    });
                    RiseMusicDomainRegistry::shared(cx).update(cx, |registry, cx| registry.install(domain, cx));
                });
                Ok::<(), anyhow::Error>(())
            }
            .boxed_local()
        })
        .await?;
        let seed = self.repository.activate().await;
        main.run(move |cx: AsyncApp| {
            async move {
                cx.update(|cx| {
                    let _ = RiseMusicPresentation::publish(seed, cx);
                });
                Ok::<(), anyhow::Error>(())
            }
            .boxed_local()
        })
        .await
    }

    pub async fn did_activate_ownership(&self) {
        self.socket_connected().await;
    }

    pub fn is_music_screen(id: &str) -> bool {
        id.starts_with(MUSIC_SCREEN_PREFIX) || id == AUDIO_PAGE_SCREEN || id == STUDIO_TRACK_DETAIL_SCREEN
    }

    pub async fn socket_connected(&self) {
        if !self.is_live() {
            return;
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let screen = main
            .run(|cx: AsyncApp| {
                async move { Ok::<_, anyhow::Error>(cx.update(SaiNavigationBridge::current_screen)) }.boxed_local()
            })
            .await
            .ok()
            .flatten();
        let Some(screen) = screen else {
            return;
        };
        if !Self::is_music_screen(&screen.id) {
            return;
        }
        self.repository.revalidate_cached_tracks(Date::now()).await;
    }

    pub async fn load_sound_track(&self, track_id: String, force: bool) {
        if !self.is_live() {
            return;
        }
        self.repository.load_sound_track(&track_id, force).await;
    }

    pub async fn update_local_track(
        &self,
        source: RiseStoredMusicTrack,
        request_id: String,
        title: String,
        artist_name: String,
        cover: Option<RiseMusicLocalCover>,
    ) -> anyhow::Result<RiseStoredMusicTrack> {
        if !self.is_live() {
            return Err(RiseMusicEditError::StaleAccount.into());
        }
        self.repository
            .update_local_track(source, &request_id, &title, &artist_name, cover)
            .await
    }

    pub async fn shutdown(&self) {
        let first = self.state.with(|state| {
            if state.is_shutdown {
                return false;
            }
            state.is_shutdown = true;
            state.is_active = false;
            true
        });
        if !first {
            return;
        }
        self.repository.shutdown().await;
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let domain = self.this.clone();
        let account_id = self.account_id.clone();
        let session_id = self.session_id;
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        RiseMusicDomainRegistry::shared(cx).update(cx, |registry, cx| registry.remove(&domain, cx));
                        RiseMusicPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                    });
                    Ok::<(), anyhow::Error>(())
                }
                .boxed_local()
            })
            .await;
    }

    fn is_live(&self) -> bool {
        self.state.with(|state| state.is_active && !state.is_shutdown)
    }
}

pub struct RiseMusicDomainRegistry {
    active: Option<Arc<RiseMusicDomain>>,
}

impl RiseMusicDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseMusicDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseMusicDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RiseMusicDomain>, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        if !std::ptr::eq(Arc::as_ptr(active), domain.as_ptr()) {
            return;
        }
        self.active = None;
        cx.notify();
    }

    pub fn matches(&self, account_id: &str, account_epoch: &str, session_id: Uuid) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        active.account_id == account_id && active.account_epoch == account_epoch && active.session_id == session_id
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseMusicDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseMusicEditError::StaleAccount.into())
        })
    }
}

pub enum RiseMusicActionExecutor {}

impl RiseMusicActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseMusicDomain>>> {
        RiseMusicDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }
}
