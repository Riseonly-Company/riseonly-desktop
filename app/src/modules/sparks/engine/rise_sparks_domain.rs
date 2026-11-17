use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_sync::RiseSyncPushHandler;
use rise_engine::{CancellationError, RiseEngine};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::sparks::engine::rise_sparks_presentation::RiseSparksPresentation;
use crate::modules::sparks::engine::rise_sparks_purchases::RiseSparksPurchases;
use crate::modules::sparks::engine::rise_sparks_repository::{RiseSparksRepository, RiseSparksRepositorySnapshot};

struct RiseSparksDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
}

pub struct RiseSparksDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub repository: Arc<RiseSparksRepository>,
    pub purchases: Arc<RiseSparksPurchases>,
    this: Weak<RiseSparksDomain>,
    state: RiseIsolated<RiseSparksDomainState>,
}

impl RiseSparksDomain {
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let session_id = Uuid::new_v4();
        let repository = RiseSparksRepository::new(engine.clone(), &account_id, &account_epoch, session_id);
        let purchases = Arc::new(RiseSparksPurchases::new(engine, &account_id));
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id,
            repository,
            purchases,
            this: this.clone(),
            state: RiseIsolated::new(RiseSparksDomainState {
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
            }),
        })
    }

    pub async fn prepare(&self, context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        if self.state.with(|state| state.is_shutdown) {
            return Err(CancellationError.into());
        }
        let repository = self.repository.clone();
        context
            .register_push_handler(RiseSyncPushHandler::new(
                rise_events::name::SPARKS_BALANCE_CHANGED,
                move |push| {
                    let repository = repository.clone();
                    async move {
                        repository.reduce_balance_event(push.data.object_value().cloned().unwrap_or_default()).await;
                        Ok(None)
                    }
                },
            ))
            .await?;
        self.state.with(|state| state.is_prepared = true);
        Ok(())
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
        let is_current =
            SaiAccountCacheScope::is_current_account_epoch(Some(&self.account_id), Some(&self.account_epoch));
        if !is_current {
            return Err(CancellationError.into());
        }
        self.state.with(|state| state.is_active = true);
        let Some(domain) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        Self::on_main(move |cx| {
            RiseSparksPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.activate(&account_id, &account_epoch, session_id, cx);
            });
            RiseSparksDomainRegistry::shared(cx).update(cx, |registry, cx| registry.install(domain, cx));
        })
        .await;
        self.repository
            .set_publisher(Arc::new(|snapshot: RiseSparksRepositorySnapshot| {
                async move { Self::publish(snapshot).await }.boxed()
            }))
            .await;
        let seed = self.repository.activate().await;
        Self::publish(seed).await;
        let repository = self.repository.clone();
        self.purchases
            .start(Arc::new(move |response| {
                let repository = repository.clone();
                async move {
                    let Some(spendable) = response.spendable_after else {
                        repository.refresh_balance(true).await;
                        return;
                    };
                    repository.apply_spendable_after(spendable, Date::now().ms()).await;
                    repository.invalidate_history().await;
                }
                .boxed()
            }))
            .await;
        Ok(())
    }

    pub async fn did_activate_ownership(&self) {
        self.socket_connected().await;
    }

    /// balance is cheap and every spark button screen reads it, history waits for its own screen
    /// a paid pack whose check the transport lost is retried here
    pub async fn socket_connected(&self) {
        let is_live = self.state.with(|state| state.is_active && !state.is_shutdown);
        if !is_live {
            return;
        }
        self.repository.refresh_balance(true).await;
        self.purchases.settle_unfinished().await;
    }

    pub async fn shutdown(&self) {
        let was_shutdown = self.state.with(|state| {
            let was_shutdown = state.is_shutdown;
            state.is_shutdown = true;
            state.is_active = false;
            was_shutdown
        });
        if was_shutdown {
            return;
        }
        self.purchases.shutdown().await;
        self.repository.shutdown().await;
        let domain = self.this.clone();
        let account_id = self.account_id.clone();
        let session_id = self.session_id;
        Self::on_main(move |cx| {
            RiseSparksDomainRegistry::shared(cx).update(cx, |registry, cx| registry.remove(&domain, cx));
            RiseSparksPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.deactivate(&account_id, session_id, cx);
            });
        })
        .await;
    }

    async fn publish(snapshot: RiseSparksRepositorySnapshot) {
        Self::on_main(move |cx| {
            RiseSparksPresentation::shared(cx).update(cx, |presentation, cx| {
                let _ = presentation.publish(snapshot, cx);
            });
        })
        .await;
    }

    /// runs the work on the main thread
    async fn on_main(work: impl FnOnce(&mut App) + Send + 'static) {
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(work);
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }
}

pub struct RiseSparksDomainRegistry {
    active: Option<Arc<RiseSparksDomain>>,
}

impl RiseSparksDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseSparksDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseSparksDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RiseSparksDomain>, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        if !std::ptr::eq(Arc::as_ptr(active), domain.as_ptr()) {
            return;
        }
        self.active = None;
        cx.notify();
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseSparksDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| CancellationError.into())
        })
    }
}
