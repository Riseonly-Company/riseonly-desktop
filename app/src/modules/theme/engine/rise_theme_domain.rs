use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine};
use tokio::task::JoinHandle;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::SaiNavigationBridge;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::{
    ChatWallpaperLook, ChatWallpaperStore,
};
use crate::modules::theme::engine::rise_theme_wallpaper_upload::RiseChatWallpaperDraft;
use crate::modules::theme::engine::rise_theme_wallpapers_presentation::RiseThemeWallpapersPresentation;
use crate::modules::theme::engine::rise_theme_wallpapers_repository::{
    RiseThemeWallpapersError, RiseThemeWallpapersRepository,
};

const REFRESHING_SCREENS: [&str; 3] = ["Chat", "ChatWallpapersSettings", "ChatWallpaperPicker"];

struct RiseThemeDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
    refresh_task: Option<JoinHandle<()>>,
}

pub struct RiseThemeDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub wallpapers: Arc<RiseThemeWallpapersRepository>,
    this: Weak<RiseThemeDomain>,
    state: RiseIsolated<RiseThemeDomainState>,
}

impl RiseThemeDomain {
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let session = Uuid::new_v4();
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let wallpapers = Arc::new(RiseThemeWallpapersRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session,
        ));
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id: session,
            engine,
            wallpapers,
            this: this.clone(),
            state: RiseIsolated::new(RiseThemeDomainState {
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
                refresh_task: None,
            }),
        })
    }

    /// no reducers since theme-service publishes no realtime events, hook kept for the lifecycle
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
        let (is_shutdown, is_prepared, is_active) =
            self.state.with(|state| (state.is_shutdown, state.is_prepared, state.is_active));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if !is_prepared {
            return Err(RiseThemeWallpapersError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }

        let is_current_account = SaiAccountCacheScope::is_current_account_epoch(
            Some(&self.account_id),
            Some(&self.account_epoch),
        );
        if !is_current_account {
            return Err(RiseThemeWallpapersError::StaleAccount.into());
        }

        let seed = self.wallpapers.activate().await;
        let activated = self.state.with(|state| {
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
        let Some(main) = RiseEngineMainHop::installed() else {
            return Ok(());
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        main.run(move |cx: AsyncApp| {
            async move {
                cx.update(|cx| {
                    let selected = seed.selected().cloned();
                    let preferences = seed.preferences.clone();
                    RiseThemeWallpapersPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        let _ = presentation.publish(seed, &account_id, &account_epoch, session_id, cx);
                    });
                    ChatWallpaperStore::shared(cx).update(cx, |store, cx| {
                        store.apply_image_wallpaper(
                            selected.as_ref().map(|row| row.id.as_str()),
                            selected.as_ref().map(|row| row.image_url.as_str()),
                            selected.as_ref().map(|row| row.look()),
                            &account_id,
                            &account_epoch,
                            cx,
                        );
                        store.apply_preferences(&preferences, &account_id, &account_epoch, cx);
                    });
                    RiseThemeDomainRegistry::shared(cx)
                        .update(cx, |registry, cx| registry.install(domain, cx));
                });
                Ok(())
            }
            .boxed_local()
        })
        .await
    }

    /// activation restores the pinned wallpaper and preferences from the account store
    /// the catalogue is requested by the wallpaper screen never by sign in
    pub async fn did_activate_ownership(&self) {}

    /// refreshes a visible wallpaper screen after changes missed on another device
    pub async fn socket_connected(&self) {
        if !self.is_live() {
            return;
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let screen = main
            .run(|cx: AsyncApp| {
                async move { Ok(cx.update(SaiNavigationBridge::current_screen)) }.boxed_local()
            })
            .await
            .ok()
            .flatten();
        let Some(screen) = screen else {
            return;
        };
        if !REFRESHING_SCREENS.contains(&screen.id.as_str()) {
            return;
        }
        self.start_refresh(false);
    }

    /// the visible chat keeps its cached wallpaper while the shared refresh completes
    fn start_refresh(&self, force: bool) {
        let wallpapers = self.wallpapers.clone();
        self.state.with(|state| {
            state.refresh_task = Some(RiseAccountRuntime::current().spawn(async move {
                wallpapers.load(force).await;
            }));
        });
    }

    pub async fn refresh(&self, force: bool) {
        if !self.is_live() {
            return;
        }
        self.wallpapers.load(force).await;
    }

    pub async fn add_wallpaper(&self, draft: RiseChatWallpaperDraft, look: ChatWallpaperLook) -> Option<String> {
        if !self.is_live() {
            return None;
        }
        self.wallpapers.add(draft, look).await
    }

    pub async fn select_wallpaper(&self, id: Option<String>) {
        if !self.is_live() {
            return;
        }
        self.wallpapers.select(id.as_deref()).await;
    }

    pub async fn delete_wallpaper(&self, id: String) {
        if !self.is_live() {
            return;
        }
        self.wallpapers.delete(&id).await;
    }

    pub async fn update_wallpaper_look(&self, id: String, look: ChatWallpaperLook) {
        if !self.is_live() {
            return;
        }
        self.wallpapers.update_look(&id, look).await;
    }

    pub async fn set_own_wallpapers_only(&self, own_only: bool) {
        if !self.is_live() {
            return;
        }
        self.wallpapers.set_own_only(own_only).await;
    }

    pub async fn set_shows_chat_wallpaper(&self, shows: bool, chat_id: String) {
        if !self.is_live() {
            return;
        }
        self.wallpapers.set_shows_chat_wallpaper(shows, &chat_id).await;
    }

    pub async fn clear_mutation_error(&self) {
        if !self.is_live() {
            return;
        }
        self.wallpapers.clear_mutation_error();
    }

    pub async fn shutdown(&self) {
        let refresh_task = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.is_shutdown = true;
            state.is_active = false;
            Some(state.refresh_task.take())
        });
        let Some(refresh_task) = refresh_task else {
            return;
        };
        if let Some(refresh_task) = refresh_task {
            refresh_task.abort();
        }
        self.wallpapers.shutdown().await;
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
                        RiseThemeDomainRegistry::shared(cx)
                            .update(cx, |registry, cx| registry.remove(&domain, cx));
                        RiseThemeWallpapersPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    fn is_live(&self) -> bool {
        self.state.with(|state| state.is_active && !state.is_shutdown)
    }
}

pub struct RiseThemeDomainRegistry {
    active: Option<Arc<RiseThemeDomain>>,
}

impl RiseThemeDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseThemeDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseThemeDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RiseThemeDomain>, cx: &mut Context<Self>) {
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
        active.account_id == account_id
            && active.account_epoch == account_epoch
            && active.session_id == session_id
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseThemeDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx)
            .update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseThemeWallpapersError::MissingActiveSession.into())
        })
    }
}

pub enum RiseThemeActionExecutor {}

impl RiseThemeActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseThemeDomain>>> {
        RiseThemeDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }
}
