use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::RiseGuideTrigger;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine};
use tokio::task::JoinHandle;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::guides::engine::rise_guides_models::{RiseGuide, RiseGuideAck, RiseGuidesError};
use crate::modules::guides::engine::rise_guides_presentation::RiseGuidesPresentation;
use crate::modules::guides::engine::rise_guides_repository::RiseGuidesRepository;

struct RiseGuidesDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
    refresh_task: Option<JoinHandle<()>>,
}

pub struct RiseGuidesDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub guides: Arc<RiseGuidesRepository>,
    this: Weak<RiseGuidesDomain>,
    state: RiseIsolated<RiseGuidesDomainState>,
}

impl RiseGuidesDomain {
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let session = Uuid::new_v4();
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let guides = Arc::new(RiseGuidesRepository::new(
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
            guides,
            this: this.clone(),
            state: RiseIsolated::new(RiseGuidesDomainState {
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
                refresh_task: None,
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
        let (is_shutdown, is_prepared, is_active) =
            self.state.with(|state| (state.is_shutdown, state.is_prepared, state.is_active));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if !is_prepared {
            return Err(RiseGuidesError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }

        let is_current_account = SaiAccountCacheScope::is_current_account_epoch(
            Some(&self.account_id),
            Some(&self.account_epoch),
        );
        if !is_current_account {
            return Err(RiseGuidesError::StaleAccount.into());
        }

        let seed = self.guides.activate().await;
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
                    RiseGuidesPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        let _ = presentation.publish(seed, &account_id, &account_epoch, session_id, cx);
                    });
                    RiseGuidesDomainRegistry::shared(cx)
                        .update(cx, |registry, cx| registry.install(domain, cx));
                });
                Ok(())
            }
            .boxed_local()
        })
        .await
    }

    pub async fn did_activate_ownership(&self) {
        if !self.is_live() {
            return;
        }
        self.start_refresh(false);
    }

    /// guides ship and get pulled mid day so a reconnect is worth a forced round trip
    pub async fn socket_connected(&self) {
        if !self.is_live() {
            return;
        }
        self.start_refresh(true);
    }

    /// started never awaited, socket_connected runs on the lifecycle queue and the runtime
    /// awaits it before delivering a frame
    /// await an rpc here and it blocks its own reply
    fn start_refresh(&self, force: bool) {
        let guides = self.guides.clone();
        self.state.with(|state| {
            state.refresh_task = Some(RiseAccountRuntime::current().spawn(async move {
                guides.load(force).await;
            }));
        });
    }

    /// revalidate on demand like pull to refresh or a feature offering its guide from an empty state
    /// started never awaited, same reason as start_refresh
    pub async fn refresh(&self, force: bool) {
        if !self.is_live() {
            return;
        }
        self.start_refresh(force);
    }

    pub async fn guide(&self, id: i64) -> Option<RiseGuide> {
        if !self.is_live() {
            return None;
        }
        self.guides.guide(id).await
    }

    pub async fn guide_by_key(&self, key: &str) -> Option<RiseGuide> {
        if !self.is_live() {
            return None;
        }
        self.guides.guide_by_key(key).await
    }

    pub async fn acknowledge(
        &self,
        guide_id: i64,
        guide_revision: i64,
        ack: RiseGuideAck,
        reached_step: i64,
        origin: RiseGuideTrigger,
    ) {
        if !self.is_live() {
            return;
        }
        self.guides
            .acknowledge(guide_id, guide_revision, ack, reached_step, origin)
            .await;
    }

    pub async fn set_auto_start(&self, enabled: bool) -> bool {
        if !self.is_live() {
            return enabled;
        }
        self.guides.set_auto_start(enabled).await
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
        self.guides.shutdown().await;
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
                        RiseGuidesDomainRegistry::shared(cx)
                            .update(cx, |registry, cx| registry.remove(&domain, cx));
                        RiseGuidesPresentation::shared(cx).update(cx, |presentation, cx| {
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

pub struct RiseGuidesDomainRegistry {
    active: Option<Arc<RiseGuidesDomain>>,
}

impl RiseGuidesDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseGuidesDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseGuidesDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RiseGuidesDomain>, cx: &mut Context<Self>) {
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

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseGuidesDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx)
            .update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseGuidesError::MissingActiveSession.into())
        })
    }
}

pub enum RiseGuidesActionExecutor {}

impl RiseGuidesActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseGuidesDomain>>> {
        RiseGuidesDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }
}
