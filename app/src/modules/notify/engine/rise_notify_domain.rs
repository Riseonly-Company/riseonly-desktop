use std::sync::{Arc, Weak};

use futures::FutureExt;
use gpui::{App, AsyncApp, Context, Entity, Task};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::rise_sync::RiseSyncPushHandler;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine};
use tokio::task::JoinHandle;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_app_engine_session::RiseAppEngineBootstrapContext;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::notify::engine::core::rise_notify_unread_wire_models::RiseNotifyUnreadRealtime;
use crate::modules::notify::stores::notify_actions::notify_types::NotificationItem;
use crate::modules::notify::engine::core::rise_release_notes_wire_models::RiseReleaseNotesError;
use crate::modules::notify::engine::rise_announcements_presentation::RiseAnnouncementsPresentation;
use crate::modules::notify::engine::rise_announcements_repository::RiseAnnouncementsRepository;
use crate::modules::notify::engine::rise_notifications_presentation::RiseNotificationsPresentation;
use crate::modules::notify::engine::rise_notifications_repository::RiseNotificationsRepository;
use crate::modules::notify::engine::rise_notify_unread_presentation::RiseNotifyUnreadPresentation;
use crate::modules::notify::engine::rise_notify_unread_repository::RiseNotifyUnreadRepository;
use crate::modules::notify::engine::rise_release_notes_presentation::RiseReleaseNotesPresentation;
use crate::modules::notify::engine::rise_release_notes_repository::RiseReleaseNotesRepository;

struct RiseNotifyDomainState {
    is_prepared: bool,
    is_active: bool,
    is_shutdown: bool,
    refresh_task: Option<JoinHandle<()>>,
}

pub struct RiseNotifyDomain {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub engine: Arc<RiseEngine>,
    pub release_notes: Arc<RiseReleaseNotesRepository>,
    pub announcements: Arc<RiseAnnouncementsRepository>,
    pub unread: Arc<RiseNotifyUnreadRepository>,
    pub notifications: Arc<RiseNotificationsRepository>,
    this: Weak<RiseNotifyDomain>,
    state: RiseIsolated<RiseNotifyDomainState>,
}

impl RiseNotifyDomain {
    pub fn new(engine: Arc<RiseEngine>) -> Arc<Self> {
        let session = Uuid::new_v4();
        let account_id = engine.account.account_id.clone();
        let account_epoch = engine.account.epoch.clone();
        let release_notes = Arc::new(RiseReleaseNotesRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session,
        ));
        let announcements = Arc::new(RiseAnnouncementsRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session,
        ));
        let unread = Arc::new(RiseNotifyUnreadRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session,
        ));
        let notifications = RiseNotificationsRepository::new(
            engine.clone(),
            account_id.clone(),
            account_epoch.clone(),
            session,
            unread.clone(),
        );
        Arc::new_cyclic(|this| Self {
            account_id,
            account_epoch,
            session_id: session,
            engine,
            release_notes,
            announcements,
            unread,
            notifications,
            this: this.clone(),
            state: RiseIsolated::new(RiseNotifyDomainState {
                is_prepared: false,
                is_active: false,
                is_shutdown: false,
                refresh_task: None,
            }),
        })
    }

    /// reducer is installed here never in activate
    /// the runtime snapshots the push claim set once right after bootstrap returns
    pub async fn prepare(&self, context: &RiseAppEngineBootstrapContext) -> anyhow::Result<()> {
        let (is_shutdown, is_prepared) =
            self.state.with(|state| (state.is_shutdown, state.is_prepared));
        if is_shutdown {
            return Err(CancellationError.into());
        }
        if is_prepared {
            return Ok(());
        }
        let unread = self.unread.clone();
        let notifications = self.notifications.clone();
        context
            .register_push_handler(RiseSyncPushHandler::new(
                RiseNotifyUnreadRealtime::EVENT_TYPE,
                move |envelope| {
                    let unread = unread.clone();
                    let notifications = notifications.clone();
                    async move {
                        let count = RiseNotifyUnreadRealtime::decode(&envelope);
                        match count {
                            Some(count) => unread.apply_authoritative(count).await,
                            None => unread.adjust(1).await,
                        }
                        let created_id = RiseNotifyUnreadRealtime::decode_notification_id(&envelope);
                        notifications.note_created(count, created_id).await;
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
            return Err(RiseReleaseNotesError::MissingActiveSession.into());
        }
        if is_active {
            return Ok(());
        }

        let is_current_account = SaiAccountCacheScope::is_current_account_epoch(
            Some(&self.account_id),
            Some(&self.account_epoch),
        );
        if !is_current_account {
            return Err(RiseReleaseNotesError::StaleAccount.into());
        }

        let (release_seed, announcement_seed, unread_seed, feed_seed) = futures::join!(
            self.release_notes.activate(),
            self.announcements.activate(),
            self.unread.activate(),
            self.notifications.activate(),
        );
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
                    RiseNotifyUnreadPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        let _ = presentation.publish(
                            unread_seed,
                            &account_id,
                            &account_epoch,
                            session_id,
                            cx,
                        );
                    });
                    RiseNotificationsPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        let _ = presentation.publish(
                            feed_seed,
                            &account_id,
                            &account_epoch,
                            session_id,
                            cx,
                        );
                    });
                    RiseReleaseNotesPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        let _ = presentation.publish(
                            release_seed,
                            &account_id,
                            &account_epoch,
                            session_id,
                            cx,
                        );
                    });
                    RiseAnnouncementsPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.activate(&account_id, &account_epoch, session_id, cx);
                        let _ = presentation.publish(
                            announcement_seed,
                            &account_id,
                            &account_epoch,
                            session_id,
                            cx,
                        );
                    });
                    RiseNotifyDomainRegistry::shared(cx)
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

    /// a publish probably landed while the app was away so a reconnect is the one moment worth a
    /// forced round trip
    pub async fn socket_connected(&self) {
        if !self.is_live() {
            return;
        }
        self.start_refresh(true);
    }

    /// started never awaited
    /// socket_connected runs on the lifecycle queue and accept awaits that same queue
    /// so an awaited rpc here would block delivery of its own reply
    fn start_refresh(&self, force: bool) {
        let release_notes = self.release_notes.clone();
        let announcements = self.announcements.clone();
        let unread = self.unread.clone();
        let notifications = self.notifications.clone();
        self.state.with(|state| {
            if state.is_shutdown {
                return;
            }
            state.refresh_task = Some(RiseAccountRuntime::current().spawn(async move {
                futures::join!(
                    release_notes.load(force),
                    announcements.load(force),
                    announcements.retry_deferred_open(),
                    unread.load(force),
                    notifications.refresh_if_observed(),
                );
            }));
        });
    }

    pub async fn refresh_unread_badge(&self) {
        if !self.is_live() {
            return;
        }
        let unread = self.unread.clone();
        let notifications = self.notifications.clone();
        let announcements = self.announcements.clone();
        RiseAccountRuntime::current().spawn(async move {
            announcements.retry_deferred_open().await;
        });
        RiseAccountRuntime::current().spawn(async move {
            unread.load(true).await;
            notifications.refresh_if_observed().await;
        });
    }

    pub async fn refresh_announcements_after_call(&self) {
        if !self.is_live() {
            return;
        }
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let is_current_session = main
            .run(move |cx: AsyncApp| {
                async move {
                    Ok(cx.update(|cx| {
                        RiseNotifyDomainRegistry::shared(cx).read(cx).matches(
                            &account_id,
                            &account_epoch,
                            session_id,
                        ) && SaiAccountCacheScope::is_current_account_epoch(
                            Some(&account_id),
                            Some(&account_epoch),
                        )
                    }))
                }
                .boxed_local()
            })
            .await
            .unwrap_or(false);
        if !is_current_session || !self.is_live() {
            return;
        }
        self.announcements.load(true).await;
    }

    pub async fn load_notifications(&self, force: bool) {
        if !self.is_live() {
            return;
        }
        self.notifications.load(force).await;
    }

    pub async fn created_notification(&self, notification_id: i64) -> Option<NotificationItem> {
        if !self.is_live() {
            return None;
        }
        self.notifications.created_item(notification_id).await
    }

    pub async fn begin_observing_notifications(&self, observer_id: Uuid) {
        if !self.is_live() {
            return;
        }
        self.notifications.begin_observing(observer_id).await;
    }

    pub async fn end_observing_notifications(&self, observer_id: Uuid) {
        self.notifications.end_observing(observer_id).await;
    }

    pub async fn load_more_notifications(&self) {
        if !self.is_live() {
            return;
        }
        self.notifications.load_more().await;
    }

    pub async fn load_newer_notifications(&self) {
        if !self.is_live() {
            return;
        }
        self.notifications.load_newer().await;
    }

    pub async fn mark_notification_read(&self, id: &str) {
        if !self.is_live() {
            return;
        }
        self.notifications.mark_read(id).await;
    }

    pub async fn mark_all_notifications_read(&self) {
        if !self.is_live() {
            return;
        }
        self.notifications.mark_all_read().await;
    }

    pub async fn delete_notification(&self, id: &str) {
        if !self.is_live() {
            return;
        }
        self.notifications.delete(id).await;
    }

    pub async fn clear_unread_badge(&self) {
        if !self.is_live() {
            return;
        }
        self.unread.clear().await;
    }

    pub async fn sync_unread_badge(&self, count: i64) {
        if !self.is_live() {
            return;
        }
        self.unread.apply_authoritative(count).await;
    }

    pub async fn adjust_unread_badge(&self, by: i64) {
        if !self.is_live() {
            return;
        }
        self.unread.adjust(by).await;
    }

    pub async fn mark_release_shown(&self, release_id: i64) {
        if !self.is_live() {
            return;
        }
        self.release_notes.mark_shown(release_id).await;
    }

    pub async fn dismiss_release(&self, release_id: i64, seen_slides: i64) {
        if !self.is_live() {
            return;
        }
        self.release_notes.dismiss(release_id, seen_slides).await;
    }

    pub async fn open_release_action(&self, release_id: i64, seen_slides: i64) {
        if !self.is_live() {
            return;
        }
        self.release_notes.opened_action(release_id, seen_slides).await;
    }

    pub async fn mark_announcement_shown(&self, announcement_id: i64) {
        if !self.is_live() {
            return;
        }
        self.announcements.mark_shown(announcement_id).await;
    }

    pub async fn dismiss_announcement(&self, announcement_id: i64, seen_slides: i64) {
        if !self.is_live() {
            return;
        }
        self.announcements.dismiss(announcement_id, seen_slides).await;
    }

    pub async fn open_announcement_action(&self, announcement_id: i64, seen_slides: i64) {
        if !self.is_live() {
            return;
        }
        self.announcements.opened_action(announcement_id, seen_slides).await;
    }

    pub async fn open_announcement(&self, announcement_id: i64) {
        if !self.is_live() {
            return;
        }
        self.announcements.open(announcement_id).await;
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
        self.release_notes.shutdown().await;
        self.announcements.shutdown().await;
        self.unread.shutdown().await;
        self.notifications.shutdown().await;
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
                        RiseNotifyDomainRegistry::shared(cx)
                            .update(cx, |registry, cx| registry.remove(&domain, cx));
                        RiseNotifyUnreadPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                        RiseNotificationsPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                        RiseReleaseNotesPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.deactivate(&account_id, session_id, cx);
                        });
                        RiseAnnouncementsPresentation::shared(cx).update(cx, |presentation, cx| {
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

pub struct RiseNotifyDomainRegistry {
    active: Option<Arc<RiseNotifyDomain>>,
}

impl RiseNotifyDomainRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: None })
    }

    pub fn active(&self) -> Option<&Arc<RiseNotifyDomain>> {
        self.active.as_ref()
    }

    pub fn install(&mut self, domain: Arc<RiseNotifyDomain>, cx: &mut Context<Self>) {
        self.active = Some(domain);
        cx.notify();
    }

    pub fn remove(&mut self, domain: &Weak<RiseNotifyDomain>, cx: &mut Context<Self>) {
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

    pub fn require_domain(&mut self, cx: &mut Context<Self>) -> Task<anyhow::Result<Arc<RiseNotifyDomain>>> {
        if let Some(active) = &self.active {
            return Task::ready(Ok(active.clone()));
        }
        let prepare = RiseEngineAppRuntime::shared(cx)
            .update(cx, |runtime, cx| runtime.prepare_active_account(cx));
        cx.spawn(async move |registry, cx| {
            prepare.await?;
            let active = registry.read_with(cx, |registry, _| registry.active.clone())?;
            active.ok_or_else(|| RiseReleaseNotesError::MissingActiveSession.into())
        })
    }
}

pub enum RiseNotifyActionExecutor {}

impl RiseNotifyActionExecutor {
    pub fn require_domain(cx: &mut App) -> Task<anyhow::Result<Arc<RiseNotifyDomain>>> {
        RiseNotifyDomainRegistry::shared(cx).update(cx, |registry, cx| registry.require_domain(cx))
    }
}
