use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::{CancellationError, RiseEngine};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::moderation::engine::moderation_domain_hooks::ModerationDomainHooks;
use crate::modules::moderation::engine::rise_moderation_presentation::RiseModerationPresentation;
use crate::modules::moderation::engine::rise_moderation_repository::RiseModerationRepository;
use crate::modules::moderation::engine::rise_moderation_rpc::RiseModerationError;

struct RiseModerationDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
}

/// account scoped root for report-service
/// reports, the queue and its cases appeals and the callers own sanctions
/// moderator roster and permission editor still use the legacy transport
pub struct RiseModerationDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub repository: Arc<RiseModerationRepository>,
    this: Weak<RiseModerationDomain>,
    state: RiseIsolated<RiseModerationDomainState>,
}

impl RiseModerationDomain {
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let repository = Arc::new(RiseModerationRepository::new(engine.clone(), account_id.clone()));
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id: Uuid::new_v4(),
            engine,
            repository,
            this: this.clone(),
            state: RiseIsolated::new(RiseModerationDomainState {
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
        let (is_shutdown, is_prepared, is_active) =
            self.state.with(|state| (state.is_shutdown, state.is_prepared, state.is_active));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if !is_prepared {
            return Err(RiseModerationError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }

        let is_current_account =
            SaiAccountCacheScope::is_current_account_epoch(Some(&self.account_id), Some(&self.account_epoch));
        if !is_current_account {
            return Err(RiseModerationError::StaleAccount.into());
        }

        let blocks = self.repository.cached_my_blocks().await;
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
                    RiseModerationPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        if let Some(blocks) = blocks {
                            presentation.seed_blocks(blocks.value, blocks.source, cx);
                        }
                    });
                    RiseModerationDomainRegistry::shared(cx)
                        .update(cx, |registry, cx| registry.install(domain, cx));
                });
                Ok(())
            }
            .boxed_local()
        })
        .await
    }

    pub async fn did_activate_ownership(&self) {}

    /// a reconnect is when a verdict most likely landed unseen
    /// nothing is awaited here, the lifecycle queue mustnt wait on a reply that has to pass through it
    pub async fn socket_connected(&self) {
        if !self.state.with(|state| state.is_active && !state.is_shutdown) {
            return;
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        main.post(|cx| {
            RiseModerationPresentation::shared(cx)
                .update(cx, |presentation, cx| presentation.mark_stale_after_reconnect(cx));
            ModerationDomainHooks::refresh_loaded_surfaces(cx);
        });
    }

    pub async fn shutdown(&self) {
        let was_live = self.state.with(|state| {
            if state.is_shutdown {
                return false;
            }
            state.is_shutdown = true;
            state.is_active = false;
            true
        });
        if !was_live {
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
                        RiseModerationDomainRegistry::shared(cx)
                            .update(cx, |registry, cx| registry.remove(&domain, cx));
                        RiseModerationPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }
}

pub struct RiseModerationDomainRegistry {
    active: Option<Arc<RiseModerationDomain>>,
}

impl RiseModerationDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseModerationDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseModerationDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    /// only the domain that installed itself may take itself out
    pub fn remove(&mut self, domain: &Weak<RiseModerationDomain>, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        if !std::ptr::eq(Arc::as_ptr(active), domain.as_ptr()) {
            return;
        }
        self.active = None;
        cx.notify();
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseModerationDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseModerationError::MissingActiveSession.into())
        })
    }
}
