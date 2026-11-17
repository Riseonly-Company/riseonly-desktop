use std::time::Duration;

use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{App, Context, Entity, Task};
use rise_canon::rise_key::report_diagnostics;
use rise_canon::rise_request::report::SubmitDiagnosticReport;
use rise_canon::{RiseDiagnosticReportKind, RiseOverheatEffect};
use rise_core::foundation::foundation_date::RiseClock;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_music::{
    THERMAL_DEGRADE_AFTER_MS, THERMAL_EVICT_AFTER_DEGRADED_MS, THERMAL_REJOIN_COOLDOWN_MS, ThermalVerdict,
};
use rise_platform::process_info::{THERMAL_STATE_DID_CHANGE, ThermalState, thermal_state};
#[cfg(debug_assertions)]
use rise_platform::user_defaults::UserDefaults;
#[cfg(debug_assertions)]
use rise_platform::user_defaults_keys::RISE_THERMAL_OVERRIDE;

use crate::core::diagnostics::rise_diagnostics_rpc::{
    RiseDiagnosticsLocale, RiseDiagnosticsRPC, SubmitDiagnosticReportDiagnostics,
};
use crate::core::lib::device_info::DeviceInfo;
use crate::core::rise_engine_bridge::rise_engine_app_runtime::RiseEngineAppRuntime;
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;
use crate::core::desktop_platform::rise_app_events::{RiseAppEvents, RiseNotificationName, RiseSubscription};
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::music::shared::utils::music::music_audio_effect_settings::MusicAudioEffectSettings;
use crate::modules::music::shared::utils::music::music_player_state::MusicPlayerState;

const THERMAL_STATE_DID_CHANGE_NOTIFICATION: RiseNotificationName = RiseNotificationName::new(THERMAL_STATE_DID_CHANGE);

/// what to do with an effect chain on a too hot device
/// full renders all, degraded only cheap effects
/// evicted leaves the session and stops playback
pub type MusicEffectThermalVerdict = ThermalVerdict;

/// pure policy so the escalation ladder is testable without a hot device
/// serious lasts way longer than critical so sustained serious escalates on its own
/// no thermal signal reads nominal
pub enum MusicEffectThermalPolicy {}

impl MusicEffectThermalPolicy {
    pub fn degrade_after_seconds() -> f64 {
        THERMAL_DEGRADE_AFTER_MS as f64 / 1_000.0
    }

    pub fn evict_after_degraded_seconds() -> f64 {
        THERMAL_EVICT_AFTER_DEGRADED_MS as f64 / 1_000.0
    }

    pub fn rejoin_cooldown_seconds() -> f64 {
        THERMAL_REJOIN_COOLDOWN_MS as f64 / 1_000.0
    }

    pub fn verdict(
        thermal_state: ThermalState,
        serious_sustained_seconds: f64,
        carries_foreign_effects: bool,
        current: MusicEffectThermalVerdict,
    ) -> MusicEffectThermalVerdict {
        if !carries_foreign_effects {
            return ThermalVerdict::Full;
        }
        match thermal_state {
            ThermalState::Critical => ThermalVerdict::Evicted,
            ThermalState::Serious => match current {
                ThermalVerdict::Evicted => ThermalVerdict::Evicted,
                ThermalVerdict::Degraded => {
                    if serious_sustained_seconds
                        >= Self::degrade_after_seconds() + Self::evict_after_degraded_seconds()
                    {
                        ThermalVerdict::Evicted
                    } else {
                        ThermalVerdict::Degraded
                    }
                }
                ThermalVerdict::Full => {
                    if serious_sustained_seconds >= Self::degrade_after_seconds() {
                        ThermalVerdict::Degraded
                    } else {
                        ThermalVerdict::Full
                    }
                }
            },
            ThermalState::Nominal | ThermalState::Fair => {
                if current == ThermalVerdict::Evicted {
                    ThermalVerdict::Evicted
                } else {
                    ThermalVerdict::Full
                }
            }
        }
    }

    pub fn allows_rejoin(thermal_state: ThermalState, seconds_since_eviction: Option<f64>) -> bool {
        let Some(seconds_since_eviction) = seconds_since_eviction else {
            return true;
        };
        if seconds_since_eviction < Self::rejoin_cooldown_seconds() {
            return false;
        }
        match thermal_state {
            ThermalState::Nominal | ThermalState::Fair => true,
            ThermalState::Serious | ThermalState::Critical => false,
        }
    }
}

/// watches thermal state while a session plays someone elses effects
/// mutes the expensive half when hot, evicts the listener if it keeps climbing
pub struct MusicEffectThermalGuard {
    verdict: MusicEffectThermalVerdict,
    thermal_observer: Option<RiseSubscription>,
    thermal_listener: Option<Task<()>>,
    evaluation_task: Option<Task<()>>,
    serious_since: Option<Duration>,
    evicted_at: Option<Duration>,
    is_monitoring: bool,
}

impl MusicEffectThermalGuard {
    const POLL_INTERVAL: Duration = Duration::from_secs(5);

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self {
            verdict: ThermalVerdict::Full,
            thermal_observer: None,
            thermal_listener: None,
            evaluation_task: None,
            serious_since: None,
            evicted_at: None,
            is_monitoring: false,
        })
    }

    /// shared guard for readers with only &App, none before the first shared call
    pub fn try_read(cx: &App) -> Option<&Self> {
        rise_ui::shared::try_read::<Self>(cx)
    }

    pub fn verdict(&self) -> MusicEffectThermalVerdict {
        self.verdict
    }

    /// device thermal state, debug builds take a defaults override so a cool host can fake it
    fn current_thermal_state() -> ThermalState {
        #[cfg(debug_assertions)]
        {
            let overridden = match UserDefaults::standard().string(RISE_THERMAL_OVERRIDE).as_deref() {
                Some("nominal") => Some(ThermalState::Nominal),
                Some("fair") => Some(ThermalState::Fair),
                Some("serious") => Some(ThermalState::Serious),
                Some("critical") => Some(ThermalState::Critical),
                _ => None,
            };
            if let Some(state) = overridden {
                return state;
            }
        }
        thermal_state()
    }

    pub fn rendered(&self, settings: &MusicAudioEffectSettings) -> MusicAudioEffectSettings {
        if self.verdict == ThermalVerdict::Degraded {
            settings.thermally_degraded()
        } else {
            settings.clone()
        }
    }

    pub fn allows_listening_session_join(&self) -> bool {
        MusicEffectThermalPolicy::allows_rejoin(
            Self::current_thermal_state(),
            self.evicted_at
                .map(|evicted_at| RiseClock::monotonic().saturating_sub(evicted_at).as_secs_f64()),
        )
    }

    pub fn refresh_monitoring(&mut self, carries_foreign_effects: bool, cx: &mut Context<Self>) {
        if !carries_foreign_effects {
            self.stop_monitoring(cx);
            return;
        }
        if self.verdict == ThermalVerdict::Evicted {
            self.verdict = ThermalVerdict::Full;
            self.serious_since = None;
            cx.notify();
        }
        self.start_monitoring_if_needed(cx);
        self.evaluate_with(carries_foreign_effects, cx);
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        self.stop_monitoring(cx);
        self.evicted_at = None;
    }

    fn start_monitoring_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.is_monitoring {
            return;
        }
        self.is_monitoring = true;
        let (sender, mut inbox) = mpsc::unbounded::<()>();
        self.thermal_observer = Some(RiseAppEvents::default_center().observe(
            THERMAL_STATE_DID_CHANGE_NOTIFICATION,
            move |_| {
                let _ = sender.unbounded_send(());
            },
        ));
        self.thermal_listener = Some(cx.spawn(async move |this, cx| {
            while inbox.next().await.is_some() {
                if this.update(cx, |guard, cx| guard.evaluate(cx)).is_err() {
                    break;
                }
            }
        }));
        self.evaluation_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Self::POLL_INTERVAL).await;
                if this.update(cx, |guard, cx| guard.evaluate(cx)).is_err() {
                    return;
                }
            }
        }));
    }

    fn stop_monitoring(&mut self, cx: &mut Context<Self>) {
        if !self.is_monitoring {
            return;
        }
        self.is_monitoring = false;
        self.thermal_observer = None;
        self.thermal_listener = None;
        self.evaluation_task = None;
        self.serious_since = None;
        if self.verdict == ThermalVerdict::Evicted {
            return;
        }
        self.apply_verdict(ThermalVerdict::Full, cx);
    }

    fn evaluate(&mut self, cx: &mut Context<Self>) {
        let carries_foreign_effects = MusicPlayerState::shared(cx)
            .read(cx)
            .is_carrying_foreign_audio_effects();
        self.evaluate_with(carries_foreign_effects, cx);
    }

    fn evaluate_with(&mut self, carries_foreign_effects: bool, cx: &mut Context<Self>) {
        let state = Self::current_thermal_state();
        let now = RiseClock::monotonic();
        match state {
            ThermalState::Serious | ThermalState::Critical => {
                if self.serious_since.is_none() {
                    self.serious_since = Some(now);
                }
            }
            ThermalState::Nominal | ThermalState::Fair => self.serious_since = None,
        }
        let sustained = self
            .serious_since
            .map(|since| now.saturating_sub(since).as_secs_f64())
            .unwrap_or(0.0);
        let next = MusicEffectThermalPolicy::verdict(state, sustained, carries_foreign_effects, self.verdict);
        self.apply_verdict(next, cx);
    }

    fn apply_verdict(&mut self, next: MusicEffectThermalVerdict, cx: &mut Context<Self>) {
        if next == self.verdict {
            return;
        }
        let previous = self.verdict;
        self.verdict = next;
        cx.notify();

        match next {
            ThermalVerdict::Full => {
                if previous != ThermalVerdict::Full {
                    cx.defer(|cx| {
                        MusicPlayerState::shared(cx).update(cx, |player, cx| player.reapply_audio_effect_rendering(cx));
                    });
                }
            }
            ThermalVerdict::Degraded => {
                cx.defer(|cx| {
                    let _ = Self::report_overheat_for_listening_session(cx);
                    MusicPlayerState::shared(cx).update(cx, |player, cx| player.reapply_audio_effect_rendering(cx));
                    Notifier::send(
                        tr("music_effects_overheat_warning_title"),
                        NotifierSendOptions {
                            content: Some(tr("music_effects_overheat_warning_body")),
                            r#type: NotifierType::Warning,
                            position: NotifierPosition::Top,
                            icon: NotifierIcon::System("thermometer.medium".to_owned()),
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                });
            }
            ThermalVerdict::Evicted => {
                self.evicted_at = Some(RiseClock::monotonic());
                cx.defer(|cx| {
                    let report_sent = Self::report_overheat_for_listening_session(cx);
                    MusicPlayerState::shared(cx)
                        .update(cx, |player, cx| player.evict_from_listening_session_for_overheating(report_sent, cx));
                    Notifier::send(
                        tr("music_effects_overheat_title"),
                        NotifierSendOptions {
                            content: Some(tr("music_effects_overheat_body")),
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Top,
                            icon: NotifierIcon::System("thermometer.high".to_owned()),
                            duration: 6.5,
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                });
                self.stop_monitoring(cx);
            }
        }
    }

    /// looks up the session for the transition
    /// player is read in a deferred call, never inside its own update
    fn report_overheat_for_listening_session(cx: &mut App) -> Option<oneshot::Receiver<()>> {
        let session_id = MusicPlayerState::shared(cx).read(cx).listening_session()?.session_id.clone();
        Self::report_overheat(session_id, cx)
    }

    fn report_overheat(session_id: String, cx: &mut App) -> Option<oneshot::Receiver<()>> {
        let engine = RiseEngineAppRuntime::shared(cx).read(cx).engine()?;
        let thermal = if Self::current_thermal_state() == ThermalState::Critical {
            rise_music::ThermalState::Critical.slug()
        } else {
            rise_music::ThermalState::Serious.slug()
        };
        let mut body = serde_json::Map::new();
        body.insert(
            report_diagnostics::EFFECTS.to_owned(),
            serde_json::Value::Array(vec![serde_json::Value::String(RiseOverheatEffect::MusicEffects.wire().to_owned())]),
        );
        body.insert(report_diagnostics::camel::SESSION_ID.to_owned(), serde_json::Value::String(session_id));
        body.insert(report_diagnostics::camel::THERMAL_STATE.to_owned(), serde_json::Value::String(thermal.to_owned()));
        let payload = serde_json::to_string(&serde_json::Value::Object(body)).ok()?;
        let request = SubmitDiagnosticReport::diagnostic(
            RiseDiagnosticReportKind::Overheat.wire().to_owned(),
            payload,
            RiseReleaseNotesDeviceContext::app_version(),
            RiseReleaseNotesDeviceContext::build_number(),
            DeviceInfo::system_version(),
            DeviceInfo::display_name(),
            RiseDiagnosticsLocale::current(),
            Some((RiseClock::now().time_interval_since_1970() * 1_000.0) as i64),
            None,
        );
        let (settle, settled) = oneshot::channel::<()>();
        drop(RiseAccountRuntime::current().spawn(async move {
            let _ = RiseDiagnosticsRPC::new(engine).submit_diagnostic_report(&request).await;
            let _ = settle.send(());
        }));
        Some(settled)
    }
}
