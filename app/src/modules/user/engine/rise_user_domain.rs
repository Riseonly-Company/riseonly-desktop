use std::sync::{Arc, Weak};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_canon::rise_request;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_sync::RiseSyncPushHandler;
use rise_engine::{CancellationError, RiseEngine};
use rise_theme::{RiseThemeAppearance, RiseThemePlatform};
use tokio::task::AbortHandle;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_screen_refresh::{SaiActiveScreen, SaiNavigationBridge};
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::widgets::navigations::main_tab_bar_settings::{RiseTabBarPlatform, RiseTabBarSettingsChange};
use crate::core::widgets::overlay::achievement_earned_store::AchievementEarnedStore;
use crate::modules::user::engine::core::rise_user_block_wire_models::{RiseBlockScope, RiseUserBlockRealtime};
use crate::modules::user::engine::core::rise_user_rpc::RiseUserRPC;
use crate::modules::user::engine::rise_meaningful_activity::RiseMeaningfulActivity;
use crate::modules::user::engine::rise_user_achievements_presentation::RiseUserAchievementsPresentation;
use crate::modules::user::engine::rise_user_achievements_realtime::RiseUserAchievementsRealtime;
use crate::modules::user::engine::rise_user_achievements_repository::RiseUserAchievementsRepository;
use crate::modules::user::engine::rise_user_block_presentation::RiseUserBlockPresentation;
use crate::modules::user::engine::rise_user_block_repository::RiseUserBlockRepository;
use crate::modules::user::engine::rise_user_engine_models::RiseUserSettingsError;
use crate::modules::user::engine::rise_user_locale_presentation::RiseUserLocalePresentation;
use crate::modules::user::engine::rise_user_locale_repository::RiseUserLocaleRepository;
use crate::modules::user::engine::rise_user_presentation::RiseUserPresentation;
use crate::modules::user::engine::rise_user_profile_photo_upload::RiseProfilePhotoDraft;
use crate::modules::user::engine::rise_user_profile_photos_presentation::{
    RiseProfilePhotoAlbum, RiseUserProfilePhotosPresentation,
};
use crate::modules::user::engine::rise_user_profile_photos_repository::RiseUserProfilePhotosRepository;
use crate::modules::user::engine::rise_user_settings_repository::RiseUserSettingsRepository;
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;
use crate::modules::user::engine::rise_user_tab_bar_settings_repository::RiseUserTabBarSettingsRepository;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use crate::modules::user::stores::user::user_services::user_services_hooks::UserServicesHooks;

/// the running quick reaction drain, done resolves when it ends and abort cancels
struct RiseUserQuickReactionRefresh {
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

struct RiseUserDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
    achievements_refresh_task: Option<AbortHandle>,
    quick_reaction_refresh_task: Option<RiseUserQuickReactionRefresh>,
    quick_reaction_refresh_requested: bool,
}

pub struct RiseUserDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub settings: Arc<RiseUserSettingsRepository>,
    pub tab_bar: Arc<RiseUserTabBarSettingsRepository>,
    pub locale: Arc<RiseUserLocaleRepository>,
    pub achievements: Arc<RiseUserAchievementsRepository>,
    pub profile_photos: Arc<RiseUserProfilePhotosRepository>,
    pub blocks: Arc<RiseUserBlockRepository>,
    this: Weak<RiseUserDomain>,
    state: RiseIsolated<RiseUserDomainState>,
}

impl RiseUserDomain {
    pub const ACHIEVEMENTS_RECONNECT_SCREEN_IDS: [&'static str; 3] = ["Profile", "UserPage", "Achievements"];

    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let session_id = Uuid::new_v4();
        let settings = RiseUserSettingsRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        );
        let tab_bar = Arc::new(RiseUserTabBarSettingsRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        ));
        let locale = RiseUserLocaleRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        );
        let achievements = Arc::new(RiseUserAchievementsRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        ));
        let profile_photos = RiseUserProfilePhotosRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session_id,
        );
        let blocks = Arc::new(RiseUserBlockRepository::new(
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
            settings,
            tab_bar,
            locale,
            achievements,
            profile_photos,
            blocks,
            this: this.clone(),
            state: RiseIsolated::new(RiseUserDomainState {
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
                achievements_refresh_task: None,
                quick_reaction_refresh_task: None,
                quick_reaction_refresh_requested: false,
            }),
        })
    }

    /// installs reducers here and never in activate
    /// session open snapshots the push claim set right after bootstrap
    pub async fn prepare(&self, context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared) = self.state.with(|state| (state.is_shutdown, state.is_prepared));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if is_prepared {
            return Ok(());
        }

        let achievements = self.achievements.clone();
        context
            .register_push_handler(RiseSyncPushHandler::new(
                RiseUserAchievementsRealtime::EVENT_TYPE,
                move |envelope| {
                    let achievements = achievements.clone();
                    async move {
                        let Some(grant) = RiseUserAchievementsRealtime::decode(&envelope) else {
                            return Ok(None);
                        };
                        achievements.apply_grant(grant.clone()).await;
                        if let Some(main) = RiseEngineMainHop::installed() {
                            let _ = main
                                .run(move |cx: AsyncApp| {
                                    async move {
                                        cx.update(|cx| {
                                            AchievementEarnedStore::shared(cx)
                                                .update(cx, |store, cx| store.enqueue(grant, cx));
                                        });
                                        Ok(())
                                    }
                                    .boxed_local()
                                })
                                .await;
                        }
                        Ok(None)
                    }
                },
            ))
            .await?;

        let blocks = self.blocks.clone();
        context
            .register_push_handler(RiseSyncPushHandler::new(
                RiseUserBlockRealtime::EVENT_TYPE,
                move |envelope| {
                    let blocks = blocks.clone();
                    async move {
                        let Some(change) = RiseUserBlockRealtime::decode(&envelope) else {
                            return Ok(None);
                        };
                        blocks.apply_remote_change(change).await;
                        if let Some(main) = RiseEngineMainHop::installed() {
                            let _ = main
                                .run(move |cx: AsyncApp| {
                                    async move {
                                        cx.update(|cx| UserServicesHooks::refresh_listener_volumes(cx));
                                        Ok(())
                                    }
                                    .boxed_local()
                                })
                                .await;
                        }
                        Ok(None)
                    }
                },
            ))
            .await?;

        self.state.with(|state| state.is_prepared = true);
        Ok(())
    }

    pub async fn activate(&self) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared, is_active) =
            self.state.with(|state| (state.is_shutdown, state.is_prepared, state.is_active));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if !is_prepared {
            return Err(RiseUserSettingsError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }
        let is_current_account =
            SaiAccountCacheScope::is_current_account_epoch(Some(&self.account_id), Some(&self.account_epoch));
        if !is_current_account {
            return Err(RiseUserSettingsError::StaleAccount.into());
        }

        let seed = self.settings.activate().await;
        let tab_bar_seed = self.tab_bar.activate().await;
        let locale_seed = self.locale.activate().await;
        let achievements_seed = self.achievements.activate().await;
        let blocks_seed = self.blocks.activate().await;
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
                    RiseUserPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        presentation.publish(&seed, cx);
                    });
                    RiseUserTabBarPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        if let Some(tab_bar_seed) = tab_bar_seed.as_ref() {
                            presentation.publish(tab_bar_seed, &account_id, &account_epoch, session_id, cx);
                        }
                    });
                    RiseUserLocalePresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        presentation.publish(locale_seed, &account_id, &account_epoch, session_id, cx);
                    });
                    RiseUserAchievementsPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        presentation.publish(achievements_seed, &account_id, &account_epoch, session_id, cx);
                    });
                    RiseUserProfilePhotosPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                    });
                    RiseUserBlockPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        presentation.publish(blocks_seed, &account_id, &account_epoch, session_id, cx);
                    });
                    RiseUserDomainRegistry::shared(cx).update(cx, |registry, cx| registry.install(domain, cx));
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
        self.settings.load(false).await;
        self.tab_bar.load(false);
        self.locale.load(false).await;
        self.locale.observe_device_signals().await;
    }

    pub async fn socket_connected(&self) {
        if !self.is_live() {
            return;
        }
        self.refresh_quick_reaction();
        if let Some(screen) = Self::current_screen().await
            && Self::ACHIEVEMENTS_RECONNECT_SCREEN_IDS.contains(&screen.id.as_str())
        {
            self.start_achievements_refresh(true);
        }
        self.settings.load(true).await;
        self.tab_bar.load(true);
        self.locale.flush_unsent().await;
        self.locale.load(true).await;
        self.locale.observe_device_signals().await;
    }

    pub fn refresh_quick_reaction(&self) {
        let Some(this) = self.this.upgrade() else {
            return;
        };
        let weak = Arc::downgrade(&this);
        self.state.with(|state| {
            if !state.is_active || state.is_shutdown {
                return;
            }
            state.quick_reaction_refresh_requested = true;
            if state.quick_reaction_refresh_task.is_some() {
                return;
            }
            let handle = RiseAccountRuntime::current().spawn(async move {
                if let Some(domain) = weak.upgrade() {
                    domain.drain_quick_reaction_refreshes().await;
                }
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            state.quick_reaction_refresh_task = Some(RiseUserQuickReactionRefresh { abort, done });
        });
    }

    async fn drain_quick_reaction_refreshes(&self) {
        loop {
            let next = self.state.with(|state| {
                if !state.quick_reaction_refresh_requested || !state.is_active || state.is_shutdown {
                    state.quick_reaction_refresh_task = None;
                    return false;
                }
                state.quick_reaction_refresh_requested = false;
                true
            });
            if !next {
                break;
            }
            let Some(revision) = self.quick_reaction_refresh_revision().await else {
                continue;
            };
            let Ok(response) = RiseUserRPC::new(self.engine.clone()).get_my_profile().await else {
                continue;
            };
            if !self.is_live() {
                continue;
            }
            let Some(profile) = response.profile else {
                continue;
            };
            if profile.id != self.account_id {
                continue;
            }
            let reaction = profile
                .quick_reaction
                .unwrap_or_else(|| ProfileModel::DEFAULT_QUICK_REACTION.to_owned());
            if !self.apply_refreshed_quick_reaction(reaction, revision).await {
                self.state.with(|state| state.quick_reaction_refresh_requested = true);
            }
        }
    }

    /// store revision, none if this domain isnt current anymore or a local change is still on the wire
    async fn quick_reaction_refresh_revision(&self) -> Option<u64> {
        let main = RiseEngineMainHop::installed()?;
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        main.run(move |cx: AsyncApp| {
            async move {
                Ok(cx.update(|cx| {
                    if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch))
                        || !RiseUserDomainRegistry::shared(cx).read(cx).matches(
                            &account_id,
                            &account_epoch,
                            session_id,
                        )
                    {
                        return None;
                    }
                    let store = UserServicesStore::shared(cx).read(cx);
                    if store.quick_reaction_mutation_pending {
                        return None;
                    }
                    Some(store.quick_reaction_refresh_revision())
                }))
            }
            .boxed_local()
        })
        .await
        .ok()
        .flatten()
    }

    async fn apply_refreshed_quick_reaction(&self, reaction: String, revision: u64) -> bool {
        let Some(main) = RiseEngineMainHop::installed() else {
            return true;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        main.run(move |cx: AsyncApp| {
            async move {
                Ok(cx.update(|cx| {
                    if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch))
                        || !RiseUserDomainRegistry::shared(cx).read(cx).matches(
                            &account_id,
                            &account_epoch,
                            session_id,
                        )
                    {
                        return true;
                    }
                    UserServicesStore::shared(cx)
                        .update(cx, |store, cx| store.apply_refreshed_quick_reaction(&reaction, revision, cx))
                }))
            }
            .boxed_local()
        })
        .await
        .unwrap_or(true)
    }

    pub async fn load_locale_settings(&self, force: bool) {
        if !self.is_live() {
            return;
        }
        self.locale.load(force).await;
    }

    pub async fn update_locale_settings(
        &self,
        request: rise_request::user::UpdateLocaleSettings,
    ) -> anyhow::Result<()> {
        self.validate_active()?;
        self.locale.update(request).await
    }

    pub async fn load_theme_settings(&self, force: bool) -> anyhow::Result<()> {
        self.validate_active()?;
        self.settings.load(force).await;
        Ok(())
    }

    pub async fn set_theme_appearance(
        &self,
        appearance: Option<RiseThemeAppearance>,
        platform: RiseThemePlatform,
    ) -> anyhow::Result<()> {
        self.validate_active()?;
        self.settings.set_appearance(appearance, platform).await
    }

    pub async fn load_tab_bar_settings(&self, force: bool) {
        if !self.is_live() {
            return;
        }
        self.tab_bar.load(force);
    }

    /// online says if the socket was up at change time, only such a change is sent
    pub async fn update_tab_bar_settings(
        &self,
        change: RiseTabBarSettingsChange,
        platform: RiseTabBarPlatform,
        sequence: u64,
        online: bool,
    ) -> anyhow::Result<()> {
        self.validate_active()?;
        self.tab_bar.update(change, platform, sequence, online).await;
        Ok(())
    }

    pub async fn delete_my_account(&self) -> anyhow::Result<i64> {
        self.validate_active()?;
        self.settings.delete_my_account().await
    }

    pub async fn load_achievements(&self, force: bool) {
        if !self.is_live() {
            return;
        }
        self.achievements.load(force).await;
    }

    pub async fn load_user_achievements(&self, user_id: &str, force: bool) {
        if !self.is_live() {
            return;
        }
        self.achievements.load_showcase(user_id, force).await;
    }

    pub async fn set_achievement_hidden(&self, achievement_id: i64, hidden: bool) -> anyhow::Result<()> {
        self.validate_active()?;
        self.achievements.set_hidden(achievement_id, hidden).await
    }

    pub async fn mark_achievements_seen(&self, ids: &[i64]) {
        if !self.is_live() {
            return;
        }
        self.achievements.mark_seen(ids).await;
    }

    pub async fn seed_profile_photos(&self, user_id: &str) -> Option<RiseProfilePhotoAlbum> {
        if !self.is_live() {
            return None;
        }
        self.profile_photos.seed_from_cache(user_id).await
    }

    pub async fn load_profile_photos(&self, user_id: &str, force: bool) {
        if !self.is_live() {
            return;
        }
        self.profile_photos
            .load(user_id, user_id == self.account_id, force)
            .await;
    }

    pub async fn load_more_profile_photos(&self, user_id: &str) {
        if !self.is_live() {
            return;
        }
        self.profile_photos.load_more(user_id).await;
    }

    pub async fn add_profile_photos(&self, drafts: Vec<RiseProfilePhotoDraft>) {
        if !self.is_live() {
            return;
        }
        self.profile_photos.add_photos(drafts);
    }

    pub async fn set_main_profile_photo(&self, photo_id: &str) -> anyhow::Result<String> {
        self.validate_active()?;
        self.profile_photos.set_main(&self.account_id, photo_id).await
    }

    pub async fn delete_profile_photo(&self, photo_id: &str) -> anyhow::Result<String> {
        self.validate_active()?;
        self.profile_photos.delete(&self.account_id, photo_id).await
    }

    pub async fn invalidate_profile_photos(&self, user_id: &str) {
        if self.state.with(|state| state.is_shutdown) {
            return;
        }
        self.profile_photos.invalidate(user_id);
    }

    pub async fn load_blocked_users(&self, force: bool) {
        if !self.is_live() {
            return;
        }
        self.blocks.load(force).await;
    }

    pub async fn load_more_blocked_users(&self) {
        if !self.is_live() {
            return;
        }
        self.blocks.load_more().await;
    }

    pub async fn trim_blocked_users(&self) {
        if self.state.with(|state| state.is_shutdown) {
            return;
        }
        self.blocks.trim_unobserved().await;
    }

    pub async fn set_block(&self, target_user_id: &str, scope: Option<RiseBlockScope>) -> anyhow::Result<()> {
        self.validate_active()?;
        self.blocks.set_block(target_user_id, scope).await
    }

    pub async fn note_block_scope(&self, target_user_id: &str, scope: Option<RiseBlockScope>) {
        if !self.is_live() {
            return;
        }
        self.blocks.note_block_scope(target_user_id, scope).await;
    }

    fn start_achievements_refresh(&self, force: bool) {
        let repository = self.achievements.clone();
        self.state.with(|state| {
            let handle = RiseAccountRuntime::current().spawn(async move {
                repository.load(force).await;
            });
            state.achievements_refresh_task = Some(handle.abort_handle());
        });
    }

    pub async fn shutdown(&self) {
        let stopped = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.is_shutdown = true;
            state.is_active = false;
            state.quick_reaction_refresh_requested = false;
            let quick_reaction_task = state.quick_reaction_refresh_task.take();
            if let Some(task) = quick_reaction_task.as_ref() {
                task.abort.abort();
            }
            if let Some(achievements_refresh_task) = state.achievements_refresh_task.take() {
                achievements_refresh_task.abort();
            }
            Some(quick_reaction_task.map(|task| task.done))
        });
        let Some(quick_reaction_done) = stopped else {
            return;
        };
        if let Some(done) = quick_reaction_done {
            done.await;
        }
        self.settings.shutdown().await;
        self.tab_bar.shutdown().await;
        self.locale.shutdown().await;
        self.achievements.shutdown().await;
        self.profile_photos.shutdown().await;
        self.blocks.shutdown().await;
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
                        RiseUserDomainRegistry::shared(cx).update(cx, |registry, cx| registry.remove(&domain, cx));
                        RiseUserPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                        RiseUserTabBarPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                        RiseUserLocalePresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                        RiseUserAchievementsPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                        RiseUserProfilePhotosPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                        RiseUserBlockPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    fn validate_active(&self) -> anyhow::Result<()> {
        if self.is_live() {
            Ok(())
        } else {
            Err(RiseUserSettingsError::MissingActiveSession.into())
        }
    }

    fn is_live(&self) -> bool {
        self.state.with(|state| state.is_active && !state.is_shutdown)
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
}

pub struct RiseUserDomainRegistry {
    active: Option<Arc<RiseUserDomain>>,
}

impl RiseUserDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseUserDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseUserDomain>, cx: &mut Context<Self>) {
        RiseMeaningfulActivity::shared(cx).update(cx, |activity, cx| activity.activate(&domain, cx));
        self.active = Some(domain);
        cx.notify();
    }

    /// only the domain that installed itself may remove itself
    pub fn remove(&mut self, domain: &Weak<RiseUserDomain>, cx: &mut Context<Self>) {
        let Some(active) = self.active.clone() else {
            return;
        };
        if !std::ptr::eq(Arc::as_ptr(&active), domain.as_ptr()) {
            return;
        }
        RiseMeaningfulActivity::shared(cx).update(cx, |activity, _| activity.deactivate(&active));
        self.active = None;
        cx.notify();
    }

    pub fn matches(&self, account_id: &str, account_epoch: &str, session_id: Uuid) -> bool {
        let Some(active) = &self.active else {
            return false;
        };
        active.account_id == account_id && active.account_epoch == account_epoch && active.session_id == session_id
    }

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseUserDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx).update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseUserSettingsError::MissingActiveSession.into())
        })
    }
}

pub enum RiseUserActionExecutor {}

impl RiseUserActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseUserDomain>>> {
        RiseUserDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }
}
