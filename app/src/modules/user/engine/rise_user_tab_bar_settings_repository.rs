use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt;
use gpui::AsyncApp;
use rise_canon::RiseTimingCanon;
use rise_canon::rise_view::user::TabBarSet;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine};
use tokio::task::JoinHandle;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::widgets::navigations::main_tab_bar_settings::{
    RiseTabBarPlatform, RiseTabBarSettingsChange, RiseTabBarSettingsSet,
};
use crate::modules::user::engine::core::rise_user_debug_trace::RiseUserDebugTrace;
use crate::modules::user::engine::core::rise_user_rpc::{RiseUserEndpoints, RiseUserRPC};
use crate::modules::user::engine::core::rise_user_tab_bar_settings_resource_repository::{
    RiseUserTabBarPersistedState, RiseUserTabBarSettingsResourceRepository,
};
use crate::modules::user::engine::core::rise_user_tab_bar_wire_models::TabBarSetSettings;
use crate::modules::user::engine::rise_user_domain::RiseUserDomainRegistry;
use crate::modules::user::engine::rise_user_engine_models::RiseUserSettingsError;
use crate::modules::user::engine::rise_user_tab_bar_presentation::RiseUserTabBarPresentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseUserTabBarSnapshot {
    pub settings: RiseTabBarSettingsSet,
    pub applied_sequence: u64,
    pub is_synced: bool,
}

#[derive(Clone, Copy)]
struct RiseUserTabBarSettingsRepositoryEdit {
    change: RiseTabBarSettingsChange,
    sequence: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RiseUserTabBarSettingsRepositoryWork {
    Refresh,
    Send(RiseTabBarPlatform),
}

struct RiseUserTabBarSettingsRepositoryState {
    server: Option<RiseTabBarSettingsSet>,
    local: Option<RiseTabBarSettingsSet>,
    edits: HashMap<RiseTabBarPlatform, RiseUserTabBarSettingsRepositoryEdit>,
    applied_sequence: u64,
    latest_sequence: HashMap<RiseTabBarPlatform, u64>,
    queue: Vec<RiseUserTabBarSettingsRepositoryWork>,
    worker: Option<JoinHandle<()>>,
    last_refresh_at_milliseconds: Option<i64>,
    is_shutdown: bool,
}

/// accounts bar for both phones and the desktop side bar
/// cached set shows at once, a read refreshes it behind
pub struct RiseUserTabBarSettingsRepository {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    rpc: RiseUserRPC,
    resource: RiseUserTabBarSettingsResourceRepository,
    refresh_interval_milliseconds: i64,
    state: RiseIsolated<RiseUserTabBarSettingsRepositoryState>,
}

impl RiseUserTabBarSettingsRepository {
    pub fn new(
        engine: Arc<RiseEngine>,
        account_id: String,
        account_epoch: String,
        session_id: Uuid,
    ) -> Self {
        Self::with_refresh_interval_milliseconds(
            engine,
            account_id,
            account_epoch,
            session_id,
            RiseTimingCanon::freshness_ms("user_settings"),
        )
    }

    pub fn with_refresh_interval_milliseconds(
        engine: Arc<RiseEngine>,
        account_id: String,
        account_epoch: String,
        session_id: Uuid,
        refresh_interval_milliseconds: i64,
    ) -> Self {
        assert!(refresh_interval_milliseconds >= 0);
        Self {
            rpc: RiseUserRPC::new(engine.clone()),
            resource: RiseUserTabBarSettingsResourceRepository::new(engine, &account_id),
            account_id,
            account_epoch,
            session_id,
            refresh_interval_milliseconds,
            state: RiseIsolated::new(RiseUserTabBarSettingsRepositoryState {
                server: None,
                local: None,
                edits: HashMap::new(),
                applied_sequence: 0,
                latest_sequence: HashMap::new(),
                queue: Vec::new(),
                worker: None,
                last_refresh_at_milliseconds: None,
                is_shutdown: false,
            }),
        }
    }

    /// none when nothing is cached for the account,
    /// screens keep their seed or defaults until the server answers
    pub async fn activate(&self) -> Option<RiseUserTabBarSnapshot> {
        if self.state.with(|s| s.is_shutdown) {
            return None;
        }
        let cached = self.resource.cached().await.ok().flatten()?;
        self.state.with(|s| {
            s.server = cached.server;
            s.local = cached.local;
            if s.server.is_none() && s.local.is_none() {
                return None;
            }
            Some(Self::make_snapshot(s))
        })
    }

    pub fn load(self: &Arc<Self>, force: bool) {
        let now = Self::now_milliseconds();
        let skip = self.state.with(|s| {
            s.is_shutdown
                || (!force
                    && s.last_refresh_at_milliseconds.is_some_and(|last| {
                        now - last < self.refresh_interval_milliseconds
                    }))
        });
        if skip {
            return;
        }
        self.enqueue(RiseUserTabBarSettingsRepositoryWork::Refresh);
    }

    /// send says if the socket was up when the change was made
    pub async fn update(
        self: &Arc<Self>,
        change: RiseTabBarSettingsChange,
        platform: RiseTabBarPlatform,
        sequence: u64,
        send: bool,
    ) {
        let taken = self.state.with(|s| {
            let latest = s.latest_sequence.get(&platform).copied().unwrap_or(0);
            if s.is_shutdown || sequence <= latest {
                return false;
            }
            s.latest_sequence.insert(platform, sequence);
            s.applied_sequence = s.applied_sequence.max(sequence);
            if send {
                s.edits.insert(platform, RiseUserTabBarSettingsRepositoryEdit { change, sequence });
                s.local = Some(Self::effective(s));
            } else {
                s.edits.remove(&platform);
                let mut offline = Self::effective(s);
                offline[platform] = change.settings();
                s.local = Some(offline);
            }
            true
        });
        if !taken {
            return;
        }
        self.persist().await;
        self.publish().await;
        if send {
            self.enqueue(RiseUserTabBarSettingsRepositoryWork::Send(platform));
        }
    }

    pub async fn shutdown(&self) {
        let task = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.is_shutdown = true;
            s.queue.clear();
            Some(s.worker.take())
        });
        let Some(task) = task else {
            return;
        };
        if let Some(task) = task.as_ref() {
            task.abort();
        }
        self.resource.shutdown().await;
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    fn enqueue(self: &Arc<Self>, work: RiseUserTabBarSettingsRepositoryWork) {
        let weak = Arc::downgrade(self);
        self.state.with(|s| {
            if !s.queue.contains(&work) {
                s.queue.push(work);
            }
            if s.worker.is_some() || s.is_shutdown {
                return;
            }
            s.worker = Some(RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = weak.upgrade() {
                    this.drain().await;
                }
            }));
        });
    }

    async fn drain(self: &Arc<Self>) {
        loop {
            let work = self.state.with(|s| {
                if s.is_shutdown || s.queue.is_empty() {
                    s.worker = None;
                    return None;
                }
                Some(s.queue.remove(0))
            });
            let Some(work) = work else {
                return;
            };
            if !self.is_current_session().await {
                self.state.with(|s| {
                    s.queue.clear();
                    s.worker = None;
                });
                return;
            }
            match work {
                RiseUserTabBarSettingsRepositoryWork::Refresh => self.refresh().await,
                RiseUserTabBarSettingsRepositoryWork::Send(platform) => self.send(platform).await,
            }
        }
    }

    async fn refresh(&self) {
        let generation = self.state.with(|s| s.applied_sequence);
        let span =
            RiseUserDebugTrace::begin_network(RiseUserEndpoints::get_tab_bar_settings().clone(), generation)
                .await;
        let answer = async {
            let response = self.rpc.get_tab_bar_settings().await?;
            let answer = Self::settings(response.settings.as_ref())?;
            self.ensure_running()?;
            anyhow::Ok(answer)
        }
        .await;
        match answer {
            Ok(answer) => {
                self.state.with(|s| Self::accept(s, answer));
                let committed = self.persist().await;
                let published = self.publish().await;
                RiseUserDebugTrace::finish_network(&span, committed, published, None).await;
            }
            Err(error) => {
                RiseUserDebugTrace::finish_network(&span, false, false, Some(&error)).await;
                self.give_up().await;
            }
        }
    }

    async fn send(self: &Arc<Self>, platform: RiseTabBarPlatform) {
        let Some(edit) = self.state.with(|s| s.edits.get(&platform).copied()) else {
            return;
        };
        let span = RiseUserDebugTrace::begin_network(
            RiseUserEndpoints::update_tab_bar_settings().clone(),
            edit.sequence,
        )
        .await;
        let answer = async {
            let response = self.rpc.update_tab_bar_settings(platform, edit.change).await?;
            let answer = Self::settings(response.settings.as_ref())?;
            self.ensure_running()?;
            anyhow::Ok(answer)
        }
        .await;
        match answer {
            Ok(answer) => {
                self.state.with(|s| {
                    Self::forget_edit(s, platform, edit.sequence);
                    Self::accept(s, answer);
                });
                let committed = self.persist().await;
                let published = self.publish().await;
                RiseUserDebugTrace::finish_network(&span, committed, published, None).await;
            }
            Err(error) => {
                let is_shutdown = self.state.with(|s| {
                    Self::forget_edit(s, platform, edit.sequence);
                    s.is_shutdown
                });
                let mut published = false;
                if !is_shutdown && !error.is::<CancellationError>() {
                    published = self.publish().await;
                    self.enqueue(RiseUserTabBarSettingsRepositoryWork::Refresh);
                }
                RiseUserDebugTrace::finish_network(&span, false, published, Some(&error)).await;
            }
        }
    }

    fn forget_edit(
        state: &mut RiseUserTabBarSettingsRepositoryState,
        platform: RiseTabBarPlatform,
        sequence: u64,
    ) {
        if state.edits.get(&platform).map(|edit| edit.sequence) == Some(sequence) {
            state.edits.remove(&platform);
        }
    }

    /// servers set wins,
    /// whatever this device kept is dropped and only changes still on their way stay on top
    fn accept(state: &mut RiseUserTabBarSettingsRepositoryState, answer: RiseTabBarSettingsSet) {
        state.server = Some(answer);
        state.local =
            if state.edits.is_empty() { None } else { Some(Self::overlaying_edits(state, answer)) };
        state.last_refresh_at_milliseconds = Some(Self::now_milliseconds());
    }

    fn effective(state: &RiseUserTabBarSettingsRepositoryState) -> RiseTabBarSettingsSet {
        let base = state.local.or(state.server).unwrap_or_else(RiseTabBarSettingsSet::defaults);
        Self::overlaying_edits(state, base)
    }

    fn overlaying_edits(
        state: &RiseUserTabBarSettingsRepositoryState,
        base: RiseTabBarSettingsSet,
    ) -> RiseTabBarSettingsSet {
        let mut set = base;
        for (platform, edit) in &state.edits {
            set[*platform] = edit.change.settings();
        }
        set
    }

    fn make_snapshot(state: &RiseUserTabBarSettingsRepositoryState) -> RiseUserTabBarSnapshot {
        RiseUserTabBarSnapshot {
            settings: Self::effective(state),
            applied_sequence: state.applied_sequence,
            is_synced: state.local.is_none() && state.edits.is_empty(),
        }
    }

    async fn persist(&self) -> bool {
        let persisted = self.state.with(|s| {
            (!s.is_shutdown).then(|| RiseUserTabBarPersistedState::new(s.server, s.local))
        });
        let Some(persisted) = persisted else {
            return false;
        };
        self.resource.store(&persisted).await.is_ok()
    }

    async fn publish(&self) -> bool {
        let Some(snapshot) = self.state.with(|s| (!s.is_shutdown).then(|| Self::make_snapshot(s)))
        else {
            return false;
        };
        let Some(main) = RiseEngineMainHop::installed() else {
            return false;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        main.run(move |cx: AsyncApp| {
            async move {
                anyhow::Ok(cx.update(|cx| {
                    RiseUserTabBarPresentation::shared(cx).update(cx, |presentation, cx| {
                        presentation.publish(&snapshot, &account_id, &account_epoch, session_id, cx)
                    })
                }))
            }
            .boxed_local()
        })
        .await
        .unwrap_or(false)
    }

    async fn give_up(&self) {
        if self.state.with(|s| s.is_shutdown) {
            return;
        }
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
                        RiseUserTabBarPresentation::shared(cx).update(cx, |presentation, cx| {
                            presentation.give_up(&account_id, &account_epoch, session_id, cx);
                        });
                    });
                    anyhow::Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    async fn is_current_session(&self) -> bool {
        let Some(main) = RiseEngineMainHop::installed() else {
            return false;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        main.run(move |cx: AsyncApp| {
            async move {
                anyhow::Ok(cx.update(|cx| {
                    SaiAccountCacheScope::is_current_account_epoch(
                        Some(&account_id),
                        Some(&account_epoch),
                    ) && RiseUserDomainRegistry::shared(cx).read(cx).matches(
                        &account_id,
                        &account_epoch,
                        session_id,
                    )
                }))
            }
            .boxed_local()
        })
        .await
        .unwrap_or(false)
    }

    fn ensure_running(&self) -> Result<(), CancellationError> {
        if self.state.with(|s| s.is_shutdown) { Err(CancellationError) } else { Ok(()) }
    }

    fn settings(set: Option<&TabBarSet>) -> anyhow::Result<RiseTabBarSettingsSet> {
        set.and_then(|set| set.settings())
            .ok_or_else(|| anyhow::Error::from(RiseUserSettingsError::InvalidResponse))
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
