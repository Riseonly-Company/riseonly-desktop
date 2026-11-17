use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::{CancellationError, RiseEngine};

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiActiveScreen, SaiNavigationBridge};
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_domain::{RisePostActionExecutor, RisePostDomain};
use crate::modules::post::engine::rise_post_engine_models::{RisePostPagePresentation, RisePostSearchKind};
use crate::modules::search::engine::rise_search_engine_models::{
    RiseGlobalSearchCategory, RiseGlobalSearchScope, RiseSearchPresentationSnapshot,
};
use crate::modules::search::engine::rise_search_presentation::RiseSearchPresentation;
use crate::modules::search::engine::rise_search_repository::{
    RiseSearchRepository, RiseSearchRepositoryError, RiseSearchRepositoryLoadIntent,
};

#[derive(Clone, Copy)]
struct RiseSearchOwnerVersion {
    generation: u64,
    access: u64,
}

#[derive(Default)]
struct RiseSearchDomainState {
    recent_scopes: HashMap<RiseGlobalSearchScope, u64>,
    scope_owners: HashMap<Uuid, RiseGlobalSearchScope>,
    owner_versions: HashMap<Uuid, RiseSearchOwnerVersion>,
    owner_clock: u64,
    scope_clock: u64,
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
}

pub struct RiseSearchDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub repository: Arc<RiseSearchRepository>,
    this: Weak<RiseSearchDomain>,
    state: RiseIsolated<RiseSearchDomainState>,
}

impl RiseSearchDomain {
    pub const RECONNECT_REFRESH_SCREEN_IDS: [&'static str; 2] = ["GlobalSearch", "HashtagSearch"];

    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let session_id = Uuid::new_v4();
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let repository = Arc::new(RiseSearchRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        ));
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id,
            engine,
            repository,
            this: this.clone(),
            state: RiseIsolated::new(RiseSearchDomainState::default()),
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
            return Err(RiseSearchRepositoryError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }
        if !self.is_current_account() {
            return Err(RiseSearchRepositoryError::StaleAccount.into());
        }

        let seed = self.repository.activate().await;
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
                    RiseSearchPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        let _ = presentation.publish(seed, cx);
                    });
                    RiseSearchDomainRegistry::shared(cx).update(cx, |registry, cx| registry.install(domain, cx));
                });
                Ok(())
            }
            .boxed_local()
        })
        .await
    }

    pub async fn did_activate_ownership(&self) {}

    pub async fn socket_connected(&self) {
        if !self.state.with(|state| state.is_active && !state.is_shutdown) {
            return;
        }
        let Some(screen) = Self::current_screen().await else {
            return;
        };
        if !Self::RECONNECT_REFRESH_SCREEN_IDS.iter().any(|id| *id == screen.id) {
            return;
        }
        let scopes: Vec<RiseGlobalSearchScope> = self.state.with(|state| {
            let active: HashSet<&RiseGlobalSearchScope> = state.scope_owners.values().collect();
            let mut scopes: Vec<RiseGlobalSearchScope> = active.into_iter().cloned().collect();
            scopes.sort_by(|lhs, rhs| {
                let lhs = state.recent_scopes.get(lhs).copied().unwrap_or(0);
                let rhs = state.recent_scopes.get(rhs).copied().unwrap_or(0);
                rhs.cmp(&lhs)
            });
            scopes.truncate(6);
            scopes
        });
        for scope in scopes {
            self.load(&scope, RiseSearchRepositoryLoadIntent::Refresh).await;
        }
    }

    pub async fn observe_global_search(
        &self,
        query: &str,
        category: RiseGlobalSearchCategory,
        owner_id: Uuid,
        generation: u64,
    ) -> anyhow::Result<()> {
        self.validate_active().await?;
        let scope = RiseGlobalSearchScope { query: query.to_owned(), category };
        if scope.normalized_query().is_empty() {
            return Ok(());
        }
        let accepted = self.state.with(|state| {
            if !Self::advance_owner_version(state, owner_id, generation) {
                return false;
            }
            state.scope_owners.insert(owner_id, scope.clone());
            Self::trim_owner_versions(state);
            true
        });
        if !accepted {
            return Ok(());
        }
        self.load(&scope, RiseSearchRepositoryLoadIntent::Activate).await;
        Ok(())
    }

    pub async fn stop_observing_global_search(
        &self,
        query: &str,
        category: RiseGlobalSearchCategory,
        owner_id: Uuid,
        generation: u64,
    ) {
        let scope = RiseGlobalSearchScope { query: query.to_owned(), category };
        self.state.with(|state| {
            if !Self::advance_owner_version(state, owner_id, generation) {
                return;
            }
            if state.scope_owners.get(&owner_id) == Some(&scope) {
                state.scope_owners.remove(&owner_id);
            }
            Self::trim_owner_versions(state);
        });
    }

    pub async fn activate_global_search(&self, query: &str, category: RiseGlobalSearchCategory) -> anyhow::Result<()> {
        self.load_validated(query, category, RiseSearchRepositoryLoadIntent::Activate).await
    }

    pub async fn refresh_global_search(&self, query: &str, category: RiseGlobalSearchCategory) -> anyhow::Result<()> {
        self.load_validated(query, category, RiseSearchRepositoryLoadIntent::Refresh).await
    }

    pub async fn load_more_global_search(&self, query: &str, category: RiseGlobalSearchCategory) -> anyhow::Result<()> {
        self.load_validated(query, category, RiseSearchRepositoryLoadIntent::LoadMore).await
    }

    pub async fn shutdown(&self) {
        let first_shutdown = self.state.with(|state| {
            if state.is_shutdown {
                return false;
            }
            state.is_shutdown = true;
            state.is_active = false;
            state.recent_scopes.clear();
            state.scope_owners.clear();
            state.owner_versions.clear();
            true
        });
        if !first_shutdown {
            return;
        }
        self.repository.shutdown().await;
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let domain = self.this.clone();
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        RiseSearchDomainRegistry::shared(cx).update(cx, |registry, cx| registry.remove(&domain, cx));
                        RiseSearchPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, &account_epoch, session_id, cx);
                        });
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    async fn load_validated(
        &self,
        query: &str,
        category: RiseGlobalSearchCategory,
        intent: RiseSearchRepositoryLoadIntent,
    ) -> anyhow::Result<()> {
        self.validate_active().await?;
        let scope = RiseGlobalSearchScope { query: query.to_owned(), category };
        if scope.normalized_query().is_empty() {
            return Ok(());
        }
        self.load(&scope, intent).await;
        Ok(())
    }

    async fn load(&self, scope: &RiseGlobalSearchScope, intent: RiseSearchRepositoryLoadIntent) {
        self.state.with(|state| Self::touch_scope(state, scope));
        if let Some(post_kind) = scope.category.post_kind() {
            let loading = self.repository.begin_post_load(scope, intent).await;
            self.publish(loading).await;
            let Some(post_domain) = Self::require_post_domain().await else {
                return;
            };
            let _ = match intent {
                RiseSearchRepositoryLoadIntent::Activate => {
                    post_domain.activate_post_search(&scope.query, post_kind).await
                }
                RiseSearchRepositoryLoadIntent::Refresh => {
                    post_domain.refresh_post_search(&scope.query, post_kind).await
                }
                RiseSearchRepositoryLoadIntent::LoadMore => {
                    post_domain.load_more_post_search(&scope.query, post_kind).await
                }
            };
            let Some(page) = Self::search_post_page(&scope.query, post_kind).await else {
                return;
            };
            let final_snapshot = self.repository.finish_post_load(scope, page).await;
            self.publish(final_snapshot).await;
            return;
        }

        let mut ticket = self.repository.begin_load(scope, intent).await;
        let snapshot = std::mem::replace(&mut ticket.snapshot, RiseSearchPresentationSnapshot::empty());
        self.publish(snapshot).await;
        if ticket.generation.is_none() || self.state.with(|state| state.is_shutdown) {
            return;
        }
        let final_snapshot = self.repository.finish_load(&ticket).await;
        self.publish(final_snapshot).await;
    }

    async fn require_post_domain() -> Option<Arc<RisePostDomain>> {
        let main = RiseEngineMainHop::installed()?;
        main.run(move |cx: AsyncApp| {
            async move {
                let task = cx.update(|cx| RisePostActionExecutor::require_domain(cx));
                task.await
            }
            .boxed_local()
        })
        .await
        .ok()
    }

    async fn search_post_page(query: &str, kind: RisePostSearchKind) -> Option<RisePostPagePresentation> {
        let main = RiseEngineMainHop::installed()?;
        let query = query.to_owned();
        main.run(move |cx: AsyncApp| {
            async move {
                Ok(cx.update(|cx| {
                    RisePostContentPresentation::shared(cx).read(cx).search_post_page(&query, kind, cx).into_owned()
                }))
            }
            .boxed_local()
        })
        .await
        .ok()
    }

    async fn current_screen() -> Option<SaiActiveScreen> {
        let main = RiseEngineMainHop::installed()?;
        main.run(move |cx: AsyncApp| {
            async move { Ok(cx.update(|cx| SaiNavigationBridge::current_screen(cx))) }.boxed_local()
        })
        .await
        .ok()
        .flatten()
    }

    /// stores the new owner generation, an equal or older one changes nothing
    fn advance_owner_version(state: &mut RiseSearchDomainState, owner_id: Uuid, generation: u64) -> bool {
        let stored = state.owner_versions.get(&owner_id).map_or(0, |version| version.generation);
        if generation <= stored {
            return false;
        }
        state.owner_clock = state.owner_clock.wrapping_add(1);
        state.owner_versions.insert(owner_id, RiseSearchOwnerVersion { generation, access: state.owner_clock });
        true
    }

    fn touch_scope(state: &mut RiseSearchDomainState, scope: &RiseGlobalSearchScope) {
        state.scope_clock = state.scope_clock.wrapping_add(1);
        state.recent_scopes.insert(scope.clone(), state.scope_clock);
        if state.recent_scopes.len() > 12 {
            let oldest = state.recent_scopes.iter().min_by_key(|(_, clock)| **clock).map(|(scope, _)| scope.clone());
            if let Some(oldest) = oldest {
                state.recent_scopes.remove(&oldest);
            }
        }
    }

    fn trim_owner_versions(state: &mut RiseSearchDomainState) {
        while state.owner_versions.len() > 256 {
            let removable_owner_id = state
                .owner_versions
                .iter()
                .filter(|(owner_id, _)| !state.scope_owners.contains_key(*owner_id))
                .min_by_key(|(_, version)| version.access)
                .map(|(owner_id, _)| *owner_id);
            let Some(owner_id) = removable_owner_id else {
                break;
            };
            state.owner_versions.remove(&owner_id);
        }
    }

    async fn validate_active(&self) -> anyhow::Result<()> {
        if !self.state.with(|state| !state.is_shutdown && state.is_active) {
            return Err(RiseSearchRepositoryError::MissingActiveSession.into());
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return Err(RiseSearchRepositoryError::StaleAccount.into());
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let matches = main
            .run(move |cx: AsyncApp| {
                async move {
                    Ok(cx.update(|cx| {
                        RiseSearchDomainRegistry::shared(cx).read(cx).matches(&account_id, &account_epoch, session_id)
                            && SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch))
                    }))
                }
                .boxed_local()
            })
            .await
            .unwrap_or(false);
        if !matches {
            return Err(RiseSearchRepositoryError::StaleAccount.into());
        }
        Ok(())
    }

    fn is_current_account(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(Some(&self.account_id), Some(&self.account_epoch))
    }

    async fn publish(&self, snapshot: RiseSearchPresentationSnapshot) {
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        if !RiseSearchDomainRegistry::shared(cx).read(cx).matches(
                            &account_id,
                            &account_epoch,
                            session_id,
                        ) {
                            return;
                        }
                        RiseSearchPresentation::shared(cx).update(cx, |presentation, cx| {
                            let _ = presentation.publish(snapshot, cx);
                        });
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }
}

pub struct RiseSearchDomainRegistry {
    active: Option<Arc<RiseSearchDomain>>,
}

impl RiseSearchDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseSearchDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseSearchDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RiseSearchDomain>, cx: &mut Context<Self>) {
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

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseSearchDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseSearchRepositoryError::MissingActiveSession.into())
        })
    }
}

pub enum RiseSearchActionExecutor {}

impl RiseSearchActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseSearchDomain>>> {
        RiseSearchDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }
}
