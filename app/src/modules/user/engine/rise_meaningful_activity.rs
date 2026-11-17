use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::{RiseLimits, RiseWireRequestMetadata, rise_request, rise_response, rise_wire_methods};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_endpoints::EndpointKind as RiseEndpointKind;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::{CancellationError, RiseEngine};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::desktop_platform::rise_app_events::{
    RiseAppEvents, RiseNotificationName, RiseSubscription, UIApplication,
};
use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::modules::user::engine::core::rise_user_rpc::RiseUserEndpoint;
use crate::modules::user::engine::rise_user_domain::RiseUserDomain;

static MINIMUM_MS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("achievement_activity_min_ms"));
static REFRESH_MS: LazyLock<i64> =
    LazyLock::new(|| RiseLimits::service_value("achievement_activity_refresh_ms"));
static BEGIN: LazyLock<RiseUserEndpoint> = LazyLock::new(|| {
    RiseUserEndpoint::canonical(rise_wire_methods::user::BEGIN_MEANINGFUL_ACTIVITY, RiseEndpointKind::Mutation)
});
static CONFIRM: LazyLock<RiseUserEndpoint> = LazyLock::new(|| {
    RiseUserEndpoint::canonical(rise_wire_methods::user::CONFIRM_MEANINGFUL_ACTIVITY, RiseEndpointKind::Mutation)
});
static RECEIPT_LIFETIME: LazyLock<f64> = LazyLock::new(|| {
    RiseLimits::service_value("achievement_activity_challenge_ttl_ms") as f64 / 1000.0
});

pub type RiseMeaningfulActivitySource = Rc<dyn Fn(&App) -> bool>;

struct ReadingInterval {
    challenge_id: String,
    expires_uptime: f64,
    minimum_seconds: f64,
    elapsed: f64,
    active_since: Option<f64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ApplicationSignal {
    WillResignActive,
    DidBecomeActive,
}

enum AttemptFlow {
    Continue,
    Break,
}

/// one foreground task for the account,
/// shared by visible posts,
/// messages and playing music
pub struct RiseMeaningfulActivity {
    engine: Option<Arc<RiseEngine>>,
    session_id: Option<Uuid>,
    sources: HashMap<Uuid, RiseMeaningfulActivitySource>,
    task: Option<Task<()>>,
    generation: u64,
    reconcile_scheduled: bool,
    next_attempt_uptime: f64,
    tracking_complete_for_foreground: bool,
    reading: Option<ReadingInterval>,
    _observers: Vec<RiseSubscription>,
    _signals: Task<()>,
}

impl RiseMeaningfulActivity {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let (sender, mut inbox) = mpsc::unbounded::<ApplicationSignal>();
        let center = RiseAppEvents::default_center();
        let observe = |name: RiseNotificationName, signal: ApplicationSignal| {
            let sender = sender.clone();
            center.observe(name, move |_| {
                let _ = sender.unbounded_send(signal);
            })
        };
        let observers = vec![
            observe(UIApplication::WILL_RESIGN_ACTIVE_NOTIFICATION, ApplicationSignal::WillResignActive),
            observe(UIApplication::DID_BECOME_ACTIVE_NOTIFICATION, ApplicationSignal::DidBecomeActive),
        ];

        let signals = cx.spawn(async move |this, cx| {
            while let Some(signal) = inbox.next().await {
                let updated = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| match signal {
                    ApplicationSignal::WillResignActive => this.suspend(),
                    ApplicationSignal::DidBecomeActive => this.schedule(cx),
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        Self {
            engine: None,
            session_id: None,
            sources: HashMap::new(),
            task: None,
            generation: 0,
            reconcile_scheduled: false,
            next_attempt_uptime: 0.0,
            tracking_complete_for_foreground: false,
            reading: None,
            _observers: observers,
            _signals: signals,
        }
    }

    pub fn activate(&mut self, domain: &RiseUserDomain, cx: &mut Context<Self>) {
        if self.session_id == Some(domain.session_id) {
            return;
        }
        self.cancel();
        self.reading = None;
        self.tracking_complete_for_foreground = false;
        if self.session_id.is_some() {
            self.sources.clear();
        }
        self.engine = Some(domain.engine.clone());
        self.session_id = Some(domain.session_id);
        self.next_attempt_uptime = 0.0;
        self.schedule(cx);
    }

    pub fn deactivate(&mut self, domain: &RiseUserDomain) {
        if self.session_id != Some(domain.session_id) {
            return;
        }
        self.cancel();
        self.reading = None;
        self.tracking_complete_for_foreground = false;
        self.sources.clear();
        self.engine = None;
        self.session_id = None;
        self.next_attempt_uptime = 0.0;
    }

    pub fn set_source(
        &mut self,
        id: Uuid,
        active: bool,
        is_active: impl Fn(&App) -> bool + 'static,
        cx: &mut Context<Self>,
    ) {
        if active == self.sources.contains_key(&id) {
            return;
        }
        if active {
            self.sources.insert(id, Rc::new(is_active));
        } else {
            self.sources.remove(&id);
        }
        self.schedule(cx);
    }

    pub fn remove_source(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.sources.remove(&id).is_none() {
            return;
        }
        self.schedule(cx);
    }

    fn qualified(&self, cx: &App) -> bool {
        RiseAppLifecycle::scene_phase(cx) == ScenePhase::Active
            && self.sources.values().any(|is_active| is_active(cx))
    }

    pub fn suspend(&mut self) {
        self.cancel();
        self.reading = None;
        self.tracking_complete_for_foreground = false;
    }

    /// coalesces viewport changes in one loop so moving between visible posts doesnt restart the
    /// request or lose the interval
    pub fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.reconcile_scheduled {
            return;
        }
        self.reconcile_scheduled = true;
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                this.reconcile_scheduled = false;
                this.reconcile(cx);
            });
        })
        .detach();
    }

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.task = None;
    }

    fn reconcile(&mut self, cx: &mut Context<Self>) {
        if self.tracking_complete_for_foreground {
            return;
        }
        let engine = match self.engine.clone() {
            Some(engine) if self.qualified(cx) => engine,
            _ => {
                if let Some(reading) = self.reading.as_mut()
                    && let Some(since) = reading.active_since
                {
                    reading.elapsed += Self::uptime() - since;
                    reading.active_since = None;
                }
                self.cancel();
                return;
            }
        };
        if self.task.is_some() {
            return;
        }
        if let Some(reading) = self.reading.as_mut()
            && reading.active_since.is_none()
        {
            reading.active_since = Some(Self::uptime());
        }
        let app: &App = cx;
        self.sources.retain(|_, is_active| is_active(app));
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.task = Some(cx.spawn(async move |this, cx| {
            Self::run(this, generation, engine, cx).await;
        }));
    }

    async fn run(this: WeakEntity<Self>, generation: u64, engine: Arc<RiseEngine>, cx: &mut AsyncApp) {
        while Self::is_current(&this, generation, cx) {
            match Self::attempt(&this, generation, &engine, cx).await {
                Ok(AttemptFlow::Continue) => {}
                Ok(AttemptFlow::Break) => break,
                Err(_) => {
                    let keep_going = this
                        .update(cx, |this: &mut Self, _| {
                            if this.generation != generation {
                                return false;
                            }
                            this.reading = None;
                            this.next_attempt_uptime = Self::uptime() + *REFRESH_MS as f64 / 1000.0;
                            true
                        })
                        .unwrap_or(false);
                    if !keep_going {
                        break;
                    }
                }
            }
        }
        let _ = this.update(cx, |this: &mut Self, _| {
            if this.generation == generation
                && let Some(task) = this.task.take()
            {
                task.detach();
            }
        });
    }

    async fn attempt(
        this: &WeakEntity<Self>,
        generation: u64,
        engine: &Arc<RiseEngine>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<AttemptFlow> {
        let delay = this.update(cx, |this: &mut Self, _| (this.next_attempt_uptime - Self::uptime()).max(0.0))?;
        if delay > 0.0 {
            cx.background_executor().timer(Duration::from_secs_f64(delay)).await;
        }
        if !Self::is_current(this, generation, cx) {
            return Ok(AttemptFlow::Break);
        }
        let now = Self::uptime();
        let needs_receipt = this.update(cx, |this: &mut Self, _| {
            if this.reading.as_ref().map_or(0.0, |reading| reading.expires_uptime) <= now {
                this.reading = None;
            }
            this.reading.is_none()
        })?;
        if needs_receipt {
            let receipt: rise_response::user::BeginMeaningfulActivity =
                Self::call(engine, &BEGIN, rise_request::user::BeginMeaningfulActivity {}).await?;
            if !Self::is_current(this, generation, cx) {
                return Ok(AttemptFlow::Break);
            }
            if !receipt.enabled {
                this.update(cx, |this: &mut Self, _| {
                    this.next_attempt_uptime = Self::uptime() + *REFRESH_MS as f64 / 1000.0;
                    this.tracking_complete_for_foreground = true;
                })?;
                return Ok(AttemptFlow::Break);
            }
            if receipt.challenge_id.is_empty() {
                return Err(CancellationError.into());
            }
            let started = Self::uptime();
            let minimum_seconds = (*MINIMUM_MS).max(receipt.wait_ms) as f64 / 1000.0;
            this.update(cx, |this: &mut Self, _| {
                this.reading = Some(ReadingInterval {
                    challenge_id: receipt.challenge_id.clone(),
                    expires_uptime: now + *RECEIPT_LIFETIME,
                    minimum_seconds,
                    elapsed: 0.0,
                    active_since: Some(started),
                });
            })?;
        }

        let Some((challenge_id, remaining)) = this.update(cx, |this: &mut Self, _| {
            this.reading.as_ref().map(|reading| {
                let uptime = Self::uptime();
                let elapsed = reading.elapsed + (uptime - reading.active_since.unwrap_or(uptime)).max(0.0);
                (reading.challenge_id.clone(), (reading.minimum_seconds - elapsed).max(0.0))
            })
        })?
        else {
            return Ok(AttemptFlow::Break);
        };
        cx.background_executor().timer(Duration::from_secs_f64(remaining)).await;
        if !Self::is_current(this, generation, cx) {
            return Ok(AttemptFlow::Break);
        }
        let confirmed: rise_response::user::ConfirmMeaningfulActivity = Self::call(
            engine,
            &CONFIRM,
            rise_request::user::ConfirmMeaningfulActivity::new(challenge_id),
        )
        .await?;
        if !Self::is_current(this, generation, cx) {
            return Ok(AttemptFlow::Break);
        }
        this.update(cx, |this: &mut Self, _| {
            this.reading = None;
            this.next_attempt_uptime =
                Self::uptime() + (*REFRESH_MS).max(confirmed.refresh_after_ms) as f64 / 1000.0;
        })?;
        Ok(AttemptFlow::Continue)
    }

    fn is_current(this: &WeakEntity<Self>, generation: u64, cx: &mut AsyncApp) -> bool {
        this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
            this.generation == generation && this.qualified(cx)
        })
        .unwrap_or(false)
    }

    async fn call<Request, Response>(
        engine: &Arc<RiseEngine>,
        endpoint: &RiseUserEndpoint,
        request: Request,
    ) -> anyhow::Result<Response>
    where
        Request: Serialize + Send + Sync + 'static,
        Response: DeserializeOwned + Send + 'static,
    {
        let engine = engine.clone();
        let descriptor = endpoint.descriptor();
        RiseAccountRuntime::current()
            .spawn(async move {
                engine
                    .call_descriptor_response::<Request, Response>(
                        &descriptor,
                        &request,
                        None,
                        RiseWireRequestMetadata::default(),
                    )
                    .await
            })
            .await?
    }

    /// monotonic uptime in seconds
    fn uptime() -> f64 {
        RiseClock::monotonic().as_secs_f64()
    }
}
