use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::RiseIsolated;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_sync::RiseSyncPushHandler;
use rise_engine::{CancellationError, RiseEngine};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::gifts::engine::core::rise_gifts_rpc::RiseGiftsRPC;
use crate::modules::gifts::engine::rise_gift_appearance_presentation::RiseGiftAppearancePresentation;
use crate::modules::gifts::engine::rise_gift_appearance_repository::RiseGiftAppearanceRepository;
use crate::modules::gifts::stores::gift_appearance::gift_appearance_interactions::gift_appearance_interactions::GiftAppearanceInteractionsStore;
use crate::modules::gifts::stores::gifts::gifts_actions::gifts_actions::GiftsActionsStore;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::GiftsInteractionsStore;
use crate::modules::gifts::stores::gifts::gifts_services::gifts_services::GiftsServicesStore;

struct RiseGiftsDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
}

pub struct RiseGiftsDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub rpc: RiseGiftsRPC,
    pub appearances: Arc<RiseGiftAppearanceRepository>,
    this: Weak<RiseGiftsDomain>,
    state: RiseIsolated<RiseGiftsDomainState>,
}

impl RiseGiftsDomain {
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let rpc = RiseGiftsRPC::new(engine);
        let appearances =
            RiseGiftAppearanceRepository::new(rpc.clone(), account_id.clone(), account_epoch.clone());
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            rpc,
            appearances,
            this: this.clone(),
            state: RiseIsolated::new(RiseGiftsDomainState {
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
        context
            .register_push_handler(RiseSyncPushHandler::new(
                rise_events::name::GIFT_RECEIVED,
                |push| async move {
                    let payload = push.data;
                    if let Some(main) = RiseEngineMainHop::installed() {
                        let _ = main
                            .run(move |cx: AsyncApp| {
                                async move {
                                    cx.update(|cx| {
                                        GiftsInteractionsStore::shared(cx)
                                            .update(cx, |store, cx| store.handle_gift_received(payload, cx));
                                    });
                                    Ok(())
                                }
                                .boxed_local()
                            })
                            .await;
                    }
                    Ok(None)
                },
            ))
            .await?;
        context
            .register_push_handler(RiseSyncPushHandler::new(
                rise_events::name::USER_GIFTS_CHANGED,
                |push| async move {
                    let payload = push.data;
                    if let Some(main) = RiseEngineMainHop::installed() {
                        let _ = main
                            .run(move |cx: AsyncApp| {
                                async move {
                                    cx.update(|cx| {
                                        GiftsActionsStore::shared(cx)
                                            .update(cx, |store, cx| store.handle_gifts_changed(payload, cx));
                                    });
                                    Ok(())
                                }
                                .boxed_local()
                            })
                            .await;
                    }
                    Ok(None)
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
        let is_current = SaiAccountCacheScope::is_current_account_epoch(
            Some(&self.account_id),
            Some(&self.account_epoch),
        );
        if !is_current {
            return Err(CancellationError.into());
        }
        self.state.with(|state| state.is_active = true);
        let Some(domain) = self.this.upgrade() else {
            return Err(CancellationError.into());
        };
        if let Some(main) = RiseEngineMainHop::installed() {
            main.run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        RiseGiftsDomainRegistry::shared(cx)
                            .update(cx, |registry, cx| registry.install(domain, cx));
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await?;
        }
        let appearances = self.appearances.clone();
        drop(RiseAccountRuntime::current().spawn(async move {
            appearances.warm_own().await;
        }));
        Ok(())
    }

    pub async fn did_activate_ownership(&self) {}

    /// a gift may have arrived while the socket was down
    /// the next open of a tab reads it and an own tab already on screen reads it now
    pub async fn socket_connected(&self) {
        let is_live = self.state.with(|state| state.is_active && !state.is_shutdown);
        if !is_live {
            return;
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = main
            .run(|cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        GiftsActionsStore::shared(cx).update(cx, |store, cx| store.refresh_own_tab_if_loaded(cx));
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
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
        self.appearances.shutdown().await;
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let domain = self.this.clone();
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        RiseGiftsDomainRegistry::shared(cx).update(cx, |registry, cx| registry.remove(&domain, cx));
                        GiftsServicesStore::shared(cx).update(cx, |store, cx| store.reset(cx));
                        GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.drop_account_prompts(cx));
                        GiftAppearanceInteractionsStore::shared(cx).update(cx, |store, cx| store.reset(cx));
                        RiseGiftAppearancePresentation::shared(cx)
                            .update(cx, |presentation, cx| presentation.reset(cx));
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }
}

pub struct RiseGiftsDomainRegistry {
    active: Option<Arc<RiseGiftsDomain>>,
}

impl RiseGiftsDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseGiftsDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseGiftsDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RiseGiftsDomain>, cx: &mut Context<Self>) {
        let Some(active) = &self.active else {
            return;
        };
        if !std::ptr::eq(Arc::as_ptr(active), domain.as_ptr()) {
            return;
        }
        self.active = None;
        cx.notify();
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseGiftsDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx)
            .update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| CancellationError.into())
        })
    }
}
