use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once, PoisonError};
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_canon::rise_maintenance::{
    DEFAULT_BODY_KEY, DEFAULT_MEDIA_REF, DEFAULT_TITLE_KEY, MediaKind, POLL_LADDER_MS,
    POLL_MINIMUM_INTERVAL_MS, is_builtin_media, may_probe_on_foreground,
};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_engine::{RiseEngine, RiseJSONValue, RiseWireMethodDescriptor, RiseWireTransport};
use rise_i18n::{AppLanguageCatalog, Locale, tr};

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::lib::sai::sai::Sai;
use crate::core::lib::sai::sai_auth_hooks::sai_auth_access_token;
use crate::core::maintenance::maintenance_auth_hooks::MaintenanceAuthHooks;
use crate::core::maintenance::maintenance_probe::{MaintenanceProbe, MaintenanceProbeReading};

/// whether the product is closed and everything the app needs meanwhile
/// not an engine domain, same for everyone and needed before an account
/// server picks the poll interval so clients dont rush the opening
pub struct MaintenanceGate {
    snapshot: MaintenanceSnapshot,

    server_snapshot: MaintenanceSnapshot,
    poll_task: Option<Task<()>>,
    consecutive_failures: u32,
    last_probe_at_ms: i64,
    is_probing: bool,
    server_clock_sample: Option<RiseServerClockSample>,
    server_clock_revision: u64,
}

static ENGINE_WALL: Once = Once::new();

impl MaintenanceGate {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            snapshot: MaintenanceSnapshot::OPEN,
            server_snapshot: MaintenanceSnapshot::OPEN,
            poll_task: None,
            consecutive_failures: 0,
            last_probe_at_ms: 0,
            is_probing: false,
            server_clock_sample: None,
            server_clock_revision: 0,
        }
    }

    pub fn snapshot(&self) -> &MaintenanceSnapshot {
        &self.snapshot
    }

    /// server time from the status probe, elapsed time survives a device clock change
    pub fn server_now(&self) -> Date {
        self.server_clock_sample
            .map_or_else(Date::now, |sample| sample.date(RiseClock::monotonic()))
    }

    /// atomic so engine tasks and the socket manager can read it without a main thread hop
    /// the value is almost always false
    pub fn is_closed() -> bool {
        MaintenanceFlag::is_closed()
    }

    /// starts the loop, a second call while its alive does nothing
    pub fn start(&mut self, cx: &mut Context<Self>) {
        let probing_immediately = rise_may_probe_on_foreground(Self::now_ms(), self.last_probe_at_ms);
        self.start_loop(probing_immediately, cx);
    }

    fn start_loop(&mut self, probing_immediately: bool, cx: &mut Context<Self>) {
        if self.poll_task.is_some() {
            return;
        }
        self.poll_task = Some(cx.spawn(async move |this, cx| {
            Self::r#loop(this, probing_immediately, cx).await;
        }));
    }

    /// respects the canon gap so a burst of app switches doesnt queue requests
    pub fn application_did_become_active(&mut self, cx: &mut Context<Self>) {
        self.start(cx);
        if !rise_may_probe_on_foreground(Self::now_ms(), self.last_probe_at_ms) {
            return;
        }
        self.probe_once(cx).detach();
    }

    /// manual retry, always runs because an explicit request beats the schedule
    pub fn refresh_now(&mut self, cx: &mut Context<Self>) {
        self.probe_once(cx).detach();
    }

    pub fn transport_did_drop(&mut self, cx: &mut Context<Self>) {
        if self.server_snapshot.is_closed {
            return;
        }
        let minimum_gap_ms = i64::try_from(POLL_MINIMUM_INTERVAL_MS).unwrap_or(i64::MAX);
        if !rise_may_probe_again(Self::now_ms(), self.last_probe_at_ms, minimum_gap_ms) {
            return;
        }
        self.probe_once(cx).detach();
    }

    async fn r#loop(this: WeakEntity<Self>, probing_immediately: bool, cx: &mut AsyncApp) {
        let mut should_probe = probing_immediately;
        loop {
            if should_probe {
                let Ok(probe) = this.update(cx, |gate, cx| gate.probe_once(cx)) else {
                    return;
                };
                probe.await;
            }
            should_probe = true;

            let Ok(delay_ms) = this.update(cx, |gate, _| gate.next_delay_ms()) else {
                return;
            };
            if delay_ms <= 0 {
                let _ = this.update(cx, |gate, _| gate.poll_task = None);
                return;
            }
            let delay = Duration::from_millis(u64::try_from(delay_ms).unwrap_or(u64::MAX));
            cx.background_executor().timer(delay).await;
        }
    }

    fn restart_if_parked(&mut self, cx: &mut Context<Self>) {
        self.start_loop(false, cx);
    }

    fn probe_once(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if self.is_probing {
            return Task::ready(());
        }
        self.is_probing = true;

        self.last_probe_at_ms = Self::now_ms();
        let languages = AppLanguageCatalog::device_preferences();
        let reading = MaintenanceProbe::read(&languages, cx);

        cx.spawn(async move |this, cx| {
            let reading = reading.await;
            let _ = this.update(cx, |gate, cx| {
                gate.is_probing = false;
                let Some(reading) = reading else {
                    gate.consecutive_failures = gate.consecutive_failures.wrapping_add(1);
                    return;
                };

                gate.consecutive_failures = 0;
                gate.apply(&reading, cx);
            });
        })
    }

    fn apply(&mut self, reading: &MaintenanceProbeReading, cx: &mut Context<Self>) {
        if reading.server_time_ms > 0 {
            let sample = RiseServerClockSample {
                server_time_ms: reading.server_time_ms,
                received_at: RiseClock::monotonic(),
            };
            self.server_clock_sample = Some(sample);
            RiseServerClock::set(sample);
            self.server_clock_revision = self.server_clock_revision.wrapping_add(1);
            cx.notify();
        }
        self.server_snapshot = MaintenanceSnapshot::from_reading(reading, Date::now());
        if self.server_snapshot.is_closed {
            self.restart_if_parked(cx);
        }
        self.publish(true, cx);
    }

    /// active account or its role changed
    /// a bypassing role gets through a closed product
    /// so one reading can be open for an admin and closed for the next account
    pub fn viewer_did_change(&mut self, connects_on_open: bool, cx: &mut Context<Self>) {
        self.publish(connects_on_open, cx);
    }

    fn engine_wall() {
        ENGINE_WALL.call_once(|| {
            let next = RiseEngine::outbound_guard();
            RiseEngine::set_outbound_guard(Some(Arc::new(
                move |descriptor: &RiseWireMethodDescriptor, request: &RiseJSONValue| {
                    if descriptor.transport == RiseWireTransport::Socket
                        && MaintenanceFlag::is_closed()
                    {
                        return Some(anyhow::Error::new(SaiWsSendError::Maintenance));
                    }
                    next.as_ref().and_then(|next| next(descriptor, request))
                },
            )));
        });
    }

    fn publish(&mut self, connects_on_open: bool, cx: &mut Context<Self>) {
        let mut next = self.server_snapshot.clone();
        if next.is_closed && MaintenanceAuthHooks::viewer_bypasses_maintenance(cx) {
            next.is_closed = false;
        }
        let was_closed = self.snapshot.is_closed;

        if next.is_closed {
            Self::engine_wall();
        }
        MaintenanceFlag::set(next.is_closed);
        if next == self.snapshot {
            return;
        }
        let is_closed = next.is_closed;
        let revision = next.revision;
        self.snapshot = next;
        cx.notify();

        if was_closed != is_closed {
            let message = if is_closed {
                format!("product CLOSED (revision {revision})")
            } else if self.server_snapshot.is_closed {
                "product CLOSED, role passes".to_owned()
            } else {
                "product OPEN".to_owned()
            };
            Sai::log("Maintenance", move || message, SaiLogKind::Network, cx);
        }

        if was_closed && !is_closed && connects_on_open {
            cx.defer(|cx| {
                let access_token = sai_auth_access_token(cx);
                SaiWs::connect_if_needed(access_token.as_deref(), cx);
            });
        }
        if !was_closed && is_closed {
            cx.defer(SaiWs::disconnect);
        }
    }

    fn next_delay_ms(&self) -> i64 {
        if !self.server_snapshot.is_closed {
            return 0;
        }

        if self.consecutive_failures == 0 && self.server_snapshot.server_retry_after_ms > 0 {
            return self.server_snapshot.server_retry_after_ms;
        }
        rise_ladder_delay_ms(self.consecutive_failures)
    }

    fn now_ms() -> i64 {
        Date::now().ms()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseServerClockSample {
    pub server_time_ms: i64,
    pub received_at: Duration,
}

impl RiseServerClockSample {
    pub fn date(&self, at: Duration) -> Date {
        let seconds = at.checked_sub(self.received_at).map_or(0.0, |elapsed| elapsed.as_secs_f64());
        Date::from_time_interval_since_1970(self.server_time_ms as f64 / 1_000.0 + seconds)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaintenanceSnapshot {
    pub is_closed: bool,
    pub revision: i64,
    pub title: String,
    pub body: String,
    pub media_kind: MediaKind,
    pub media_ref: String,
    pub ends_at: Option<Date>,
    pub server_retry_after_ms: i64,
}

impl MaintenanceSnapshot {
    pub const OPEN: MaintenanceSnapshot = MaintenanceSnapshot {
        is_closed: false,
        revision: 0,
        title: String::new(),
        body: String::new(),
        media_kind: MediaKind::None,
        media_ref: String::new(),
        ends_at: None,
        server_retry_after_ms: 0,
    };

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        is_closed: bool,
        revision: i64,
        title: String,
        body: String,
        media_kind: MediaKind,
        media_ref: String,
        ends_at: Option<Date>,
        server_retry_after_ms: i64,
    ) -> Self {
        Self {
            is_closed,
            revision,
            title,
            body,
            media_kind,
            media_ref,
            ends_at,
            server_retry_after_ms,
        }
    }

    pub fn from_reading(reading: &MaintenanceProbeReading, observed_at: Date) -> Self {
        let remaining_ms = Self::remaining_ms(reading.ends_at_ms, reading.server_time_ms);
        Self::new(
            reading.is_closed,
            reading.revision,
            if reading.title.is_empty() { tr(DEFAULT_TITLE_KEY) } else { reading.title.clone() },
            if reading.body.is_empty() { tr(DEFAULT_BODY_KEY) } else { reading.body.clone() },
            reading.media_kind,
            if reading.media_ref.is_empty() {
                DEFAULT_MEDIA_REF.to_owned()
            } else {
                reading.media_ref.clone()
            },
            remaining_ms.map(|ms| observed_at.adding_time_interval(ms as f64 / 1_000.0)),
            reading.retry_after_ms,
        )
    }

    /// ms left until the deadline on the server clock, none if no deadline or its passed
    pub fn remaining_ms(ends_at_ms: i64, server_time_ms: i64) -> Option<i64> {
        if ends_at_ms <= 0 || server_time_ms <= 0 {
            return None;
        }
        let remaining = ends_at_ms - server_time_ms;
        (remaining > 0).then_some(remaining)
    }

    pub fn reopening_text(&self) -> String {
        let Some(ends_at) = self.ends_at else {
            return tr("maintenance_eta_unknown");
        };
        let when = RiseFormat::date_time_label(
            &ends_at,
            &Date::now(),
            &Calendar::current(),
            Locale::app_locale().identifier(),
        );
        tr("maintenance_eta").replace("{when}", &when)
    }

    pub fn builtin_animation_name(&self) -> Option<String> {
        if self.media_kind != MediaKind::Builtin {
            return None;
        }
        is_builtin_media(&self.media_ref).then(|| self.media_ref.clone())
    }
}

/// atomic flag behind is_closed
/// the transport asks on every request from any thread so reading the entity would need main
pub enum MaintenanceFlag {}

static MAINTENANCE_FLAG_CLOSED: AtomicBool = AtomicBool::new(false);

impl MaintenanceFlag {
    pub fn is_closed() -> bool {
        MAINTENANCE_FLAG_CLOSED.load(Ordering::Acquire)
    }

    pub fn set(closed: bool) {
        MAINTENANCE_FLAG_CLOSED.store(closed, Ordering::Release);
    }
}

/// server time for code off the main thread like token expiry checks
/// uses the device clock until the first status probe answers
pub enum RiseServerClock {}

static RISE_SERVER_CLOCK_SAMPLE: Mutex<Option<RiseServerClockSample>> = Mutex::new(None);

impl RiseServerClock {
    pub fn now() -> Date {
        let sample = *RISE_SERVER_CLOCK_SAMPLE.lock().unwrap_or_else(PoisonError::into_inner);
        sample.map_or_else(Date::now, |sample| sample.date(RiseClock::monotonic()))
    }

    pub fn set(value: RiseServerClockSample) {
        *RISE_SERVER_CLOCK_SAMPLE.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
    }
}

/// canon ladder step in ms, counted from the first
/// failure so zero and one both give the first rung
pub fn rise_ladder_delay_ms(consecutive_failures: u32) -> i64 {
    let rung = consecutive_failures.max(1) - 1;
    let index = usize::try_from(rung)
        .unwrap_or(usize::MAX)
        .min(POLL_LADDER_MS.len().saturating_sub(1));
    let step = POLL_LADDER_MS.get(index).copied().unwrap_or(POLL_MINIMUM_INTERVAL_MS);
    i64::try_from(step).unwrap_or(i64::MAX)
}

/// true if a probe may run again
/// a clock that went backwards counts as allowed so it cant lock the check out forever
pub fn rise_may_probe_again(now_ms: i64, last_probe_at_ms: i64, minimum_gap_ms: i64) -> bool {
    if last_probe_at_ms <= 0 {
        return true;
    }
    let elapsed = now_ms - last_probe_at_ms;
    elapsed < 0 || elapsed >= minimum_gap_ms
}

/// gap for paths a person triggers like foreground or a screen appearing
pub fn rise_may_probe_on_foreground(now_ms: i64, last_probe_at_ms: i64) -> bool {
    may_probe_on_foreground(now_ms, last_probe_at_ms)
}
