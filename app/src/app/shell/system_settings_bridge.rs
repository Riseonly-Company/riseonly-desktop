use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Global, Task};
use rise_core::rise_app_events::{RiseAppEvents, RiseNotificationName, UIAccessibility, UIApplication};
use rise_platform::accessibility::{self, AccessibilityWatch};
use rise_platform::memory_pressure::{self, MemoryPressureWatch};
use rise_platform::power;
use rise_platform::process_info::{self, THERMAL_STATE_DID_CHANGE, ThermalStateWatch};
use rise_platform::system_activity::ActivityWatch;
use rise_platform::system_clock::{self, SystemClockChange};

use crate::riseonly_app::{CURRENT_LOCALE_DID_CHANGE, NS_SYSTEM_TIME_ZONE_DID_CHANGE};

const THERMAL_STATE_DID_CHANGE_NOTIFICATION: RiseNotificationName =
    RiseNotificationName::new(THERMAL_STATE_DID_CHANGE);

/// one system change going from a seam thread to the main thread
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SystemSignal {
    ReduceMotion(bool),
    ThermalStateDidChange,
    DidReceiveMemoryWarning,
    PowerDidChange(&'static str),
    SystemClock(SystemClockChange),
}

struct SystemSettingsBridge {
    _reduce_motion: Option<AccessibilityWatch>,
    _thermal_state: Option<ThermalStateWatch>,
    _memory_pressure: Option<MemoryPressureWatch>,
    _system_clock: Option<ActivityWatch>,
    _deliveries: Task<()>,
}

impl Global for SystemSettingsBridge {}

pub fn install(cx: &mut App) {
    let (sender, mut signals) = mpsc::unbounded::<SystemSignal>();

    let motion_sender = sender.clone();
    let reduce_motion = match accessibility::watch_reduce_motion(move |reduce| {
        let _ = motion_sender.unbounded_send(SystemSignal::ReduceMotion(reduce));
    }) {
        Ok(watched) => {
            cx.set_reduce_motion(watched.current);
            Some(watched.watch)
        }
        Err(reason) => {
            tracing::warn!(target: "riseonly", %reason, "reduce motion is not observed; the start value stays");
            cx.set_reduce_motion(accessibility::reduce_motion());
            None
        }
    };

    let thermal_sender = sender.clone();
    let thermal_state = match process_info::watch_thermal_state(move |_state| {
        let _ = thermal_sender.unbounded_send(SystemSignal::ThermalStateDidChange);
    }) {
        Ok(watch) => Some(watch),
        Err(reason) => {
            tracing::warn!(target: "riseonly", %reason, "thermal state is not observed; no change will be posted");
            None
        }
    };

    let power_sender = sender.clone();
    let power_watch = cx.background_spawn(async move {
        power::watch(move |_reading, names| {
            for &name in names {
                let _ = power_sender.unbounded_send(SystemSignal::PowerDidChange(name));
            }
        })
    });

    let clock_sender = sender.clone();
    let system_clock = match system_clock::watch(move |change| {
        let _ = clock_sender.unbounded_send(SystemSignal::SystemClock(change));
    }) {
        Ok(watch) => Some(watch),
        Err(reason) => {
            tracing::warn!(target: "riseonly", %reason, "system clock changes are not observed; activation still re-reads");
            None
        }
    };

    let memory_pressure = match memory_pressure::watch(move |_level| {
        let _ = sender.unbounded_send(SystemSignal::DidReceiveMemoryWarning);
    }) {
        Ok(watch) => Some(watch),
        Err(reason) => {
            tracing::warn!(target: "riseonly", %reason, "memory pressure is not observed; caches purge only by their budgets");
            None
        }
    };

    let deliveries = cx.spawn(async move |cx| {
        let _power = match power_watch.await {
            Ok(watch) => Some(watch),
            Err(reason) => {
                tracing::warn!(target: "riseonly", %reason, "power changes are not observed; readers keep the last snapshot");
                None
            }
        };
        while let Some(signal) = signals.next().await {
            cx.update(|cx| deliver(signal, cx));
        }
    });

    cx.set_global(SystemSettingsBridge {
        _reduce_motion: reduce_motion,
        _thermal_state: thermal_state,
        _memory_pressure: memory_pressure,
        _system_clock: system_clock,
        _deliveries: deliveries,
    });
}

/// runs on the main thread
fn deliver(signal: SystemSignal, cx: &mut App) {
    match signal {
        SystemSignal::ReduceMotion(reduce) => {
            cx.set_reduce_motion(reduce);
            RiseAppEvents::default_center().post(UIAccessibility::REDUCE_MOTION_STATUS_DID_CHANGE_NOTIFICATION);
        }
        SystemSignal::ThermalStateDidChange => {
            RiseAppEvents::default_center().post(THERMAL_STATE_DID_CHANGE_NOTIFICATION);
        }
        SystemSignal::DidReceiveMemoryWarning => {
            RiseAppEvents::default_center().post(UIApplication::DID_RECEIVE_MEMORY_WARNING_NOTIFICATION);
        }
        SystemSignal::PowerDidChange(name) => {
            RiseAppEvents::default_center().post(RiseNotificationName::new(name));
        }
        SystemSignal::SystemClock(SystemClockChange::TimeZone) => {
            RiseAppEvents::default_center().post(NS_SYSTEM_TIME_ZONE_DID_CHANGE);
        }
        SystemSignal::SystemClock(SystemClockChange::Locale) => {
            RiseAppEvents::default_center().post(CURRENT_LOCALE_DID_CHANGE);
        }
    }
}
