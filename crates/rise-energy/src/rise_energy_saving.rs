use std::collections::BTreeSet;
use std::sync::{PoisonError, RwLock};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, AsyncApp, Context, Entity, EventEmitter, Global, WeakEntity};
use rise_canon::RiseGuideSignal;
use rise_platform::power::{
    BatteryState, PowerReading, PowerWatch, battery_percent_from_level, power_snapshot, watch,
};
use rise_platform::user_defaults::UserDefaults;
use rise_platform::user_defaults_keys::ENERGY_SAVING;
use serde::{Deserialize, Serialize};

use crate::guide_signal_bus_hooks::GuideSignalBusHooks;

/// what power saving takes away when it is on, each one has a real consumer behind it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiseEnergySavingFeature {
    AnimatedStickers,
    AnimatedEmoji,
    AnimatedGifs,
    LoopAnimations,
    ChatWallpaperMotion,
    InterfaceEffects,
    MediaPrefetch,
}

impl RiseEnergySavingFeature {
    pub const ALL: [Self; 7] = [
        Self::AnimatedStickers,
        Self::AnimatedEmoji,
        Self::AnimatedGifs,
        Self::LoopAnimations,
        Self::ChatWallpaperMotion,
        Self::InterfaceEffects,
        Self::MediaPrefetch,
    ];

    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::AnimatedStickers => "animatedStickers",
            Self::AnimatedEmoji => "animatedEmoji",
            Self::AnimatedGifs => "animatedGifs",
            Self::LoopAnimations => "loopAnimations",
            Self::ChatWallpaperMotion => "chatWallpaperMotion",
            Self::InterfaceEffects => "interfaceEffects",
            Self::MediaPrefetch => "mediaPrefetch",
        }
    }

    pub fn from_raw_value(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|feature| feature.raw_value() == raw)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiseEnergySavingSettings {
    pub threshold_percent: i64,
    pub reduced_features: BTreeSet<RiseEnergySavingFeature>,
}

impl Default for RiseEnergySavingSettings {
    fn default() -> Self {
        Self {
            threshold_percent: RiseEnergySavingPolicy::DEFAULT_THRESHOLD_PERCENT,
            reduced_features: RiseEnergySavingFeature::ALL.into_iter().collect(),
        }
    }
}

pub struct RiseEnergySavingPolicy;

impl RiseEnergySavingPolicy {
    pub const OFF_THRESHOLD_PERCENT: i64 = 0;
    pub const ALWAYS_THRESHOLD_PERCENT: i64 = 100;
    pub const DEFAULT_THRESHOLD_PERCENT: i64 = 20;
    pub const THRESHOLD_BREAKPOINTS: [i64; 6] = [0, 10, 20, 30, 50, 100];

    /// nearest stop, a tie takes the lower one
    pub fn normalized_threshold(raw: i64) -> i64 {
        Self::THRESHOLD_BREAKPOINTS
            .into_iter()
            .min_by_key(|stop| stop.abs_diff(raw))
            .unwrap_or(Self::DEFAULT_THRESHOLD_PERCENT)
    }

    pub fn threshold_index(percent: i64) -> usize {
        let normalized = Self::normalized_threshold(percent);
        Self::THRESHOLD_BREAKPOINTS
            .iter()
            .position(|stop| *stop == normalized)
            .unwrap_or(0)
    }

    /// true when power saving is active, system low power mode counts
    /// unless the user set the app own switch to Off, that choice wins over the machine
    pub fn is_active(
        threshold_percent: i64,
        battery_percent: Option<i64>,
        system_low_power_mode: bool,
    ) -> bool {
        if threshold_percent <= Self::OFF_THRESHOLD_PERCENT {
            return false;
        }
        if system_low_power_mode {
            return true;
        }
        if threshold_percent >= Self::ALWAYS_THRESHOLD_PERCENT {
            return true;
        }
        let Some(battery_percent) = battery_percent else {
            return false;
        };
        battery_percent <= threshold_percent
    }

    pub fn battery_percent(level: f32) -> Option<i64> {
        battery_percent_from_level(level).map(i64::from)
    }
}

/// active verdict readable off the main thread, only the store writes it on the main thread
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseEnergySavingSnapshot {
    pub is_active: bool,
    pub reduced_features: BTreeSet<RiseEnergySavingFeature>,
}

impl RiseEnergySavingSnapshot {
    pub const NONE: Self = Self {
        is_active: false,
        reduced_features: BTreeSet::new(),
    };

    pub fn reduces(&self, feature: RiseEnergySavingFeature) -> bool {
        self.is_active && self.reduced_features.contains(&feature)
    }
}

pub const RISE_ENERGY_SAVING_DID_CHANGE: &str = "riseonly.energy-saving.did-change";

/// emitted by the store only when the snapshot changed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RiseEnergySavingDidChange;

static SNAPSHOT: RwLock<RiseEnergySavingSnapshot> = RwLock::new(RiseEnergySavingSnapshot::NONE);

struct RiseEnergySavingStoreShared(Entity<RiseEnergySavingStore>);

impl Global for RiseEnergySavingStoreShared {}

pub struct RiseEnergySavingStore {
    settings: RiseEnergySavingSettings,
    battery_percent: Option<i64>,
    on_external_power: bool,
    system_low_power_mode: bool,
    is_active: bool,
    power_watch: Option<PowerWatch>,
}

impl EventEmitter<RiseEnergySavingDidChange> for RiseEnergySavingStore {}

impl RiseEnergySavingStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        if let Some(shared) = cx.try_global::<RiseEnergySavingStoreShared>() {
            return shared.0.clone();
        }
        let entity = cx.new(Self::new);
        cx.set_global(RiseEnergySavingStoreShared(entity.clone()));
        entity
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let settings = Self::load();
        let reading = PowerReading::default();
        let battery_percent = reading.battery.percent.map(i64::from);
        let on_external_power = Self::is_on_external_power(&reading);
        let system_low_power_mode = reading.low_power_mode;
        let is_active = RiseEnergySavingPolicy::is_active(
            settings.threshold_percent,
            battery_percent.filter(|_| !on_external_power),
            system_low_power_mode,
        );
        Self::write_snapshot(RiseEnergySavingSnapshot {
            is_active,
            reduced_features: settings.reduced_features.clone(),
        });
        Self::start_power_watch(cx);
        Self {
            settings,
            battery_percent,
            on_external_power,
            system_low_power_mode,
            is_active,
            power_watch: None,
        }
    }

    pub fn snapshot() -> RiseEnergySavingSnapshot {
        SNAPSHOT.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// true if the feature is reduced right now, safe off the main thread
    pub fn snapshot_reduces(feature: RiseEnergySavingFeature) -> bool {
        SNAPSHOT.read().unwrap_or_else(PoisonError::into_inner).reduces(feature)
    }

    pub fn reduces(&self, feature: RiseEnergySavingFeature) -> bool {
        self.is_active && self.settings.reduced_features.contains(&feature)
    }

    pub fn is_reduced(&self, feature: RiseEnergySavingFeature) -> bool {
        self.settings.reduced_features.contains(&feature)
    }

    pub fn settings(&self) -> &RiseEnergySavingSettings {
        &self.settings
    }

    pub fn battery_percent(&self) -> Option<i64> {
        self.battery_percent
    }

    pub fn system_low_power_mode(&self) -> bool {
        self.system_low_power_mode
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn set_threshold(&mut self, percent: i64, cx: &mut Context<Self>) {
        let normalized = RiseEnergySavingPolicy::normalized_threshold(percent);
        if normalized == self.settings.threshold_percent {
            return;
        }
        self.settings.threshold_percent = normalized;
        self.persist_and_recompute(cx);
        GuideSignalBusHooks::emit(RiseGuideSignal::EnergyThresholdChanged);
    }

    pub fn set_reduced(
        &mut self,
        feature: RiseEnergySavingFeature,
        reduced: bool,
        cx: &mut Context<Self>,
    ) {
        if self.settings.reduced_features.contains(&feature) == reduced {
            return;
        }
        if reduced {
            self.settings.reduced_features.insert(feature);
        } else {
            self.settings.reduced_features.remove(&feature);
        }
        self.persist_and_recompute(cx);
        GuideSignalBusHooks::emit(RiseGuideSignal::EnergyFeatureToggled);
    }

    /// puts back settings noted earlier, so a lesson that tried power saving leaves it as found
    pub fn restore(&mut self, previous: RiseEnergySavingSettings, cx: &mut Context<Self>) {
        if previous == self.settings {
            return;
        }
        self.settings = previous;
        self.persist_and_recompute(cx);
    }

    pub fn reset_to_defaults(&mut self, cx: &mut Context<Self>) {
        let defaults = RiseEnergySavingSettings::default();
        if self.settings == defaults {
            return;
        }
        self.settings = defaults;
        self.persist_and_recompute(cx);
    }

    pub fn refresh_device_state(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let reading = cx.background_spawn(async move { power_snapshot().reading }).await;
            let _ = this.update(cx, |store, cx| store.apply_reading(reading, cx));
        })
        .detach();
    }

    fn start_power_watch(cx: &mut Context<Self>) {
        let (sender, mut receiver) = mpsc::unbounded::<PowerReading>();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (reading, power_watch) = cx
                .background_spawn(async move {
                    let reading = power_snapshot().reading;
                    let power_watch = watch(move |changed, _names| {
                        let _ = sender.unbounded_send(changed);
                    });
                    (reading, power_watch)
                })
                .await;
            let attached = this.update(cx, |store, cx| {
                store.power_watch = power_watch.ok();
                store.apply_reading(reading, cx);
            });
            if attached.is_err() {
                return;
            }
            while let Some(reading) = receiver.next().await {
                if this.update(cx, |store, cx| store.apply_reading(reading, cx)).is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// a laptop on the cable keeps its animations, the battery threshold counts only on battery
    fn is_on_external_power(reading: &PowerReading) -> bool {
        matches!(reading.battery.state, BatteryState::Charging | BatteryState::Full)
    }

    fn apply_reading(&mut self, reading: PowerReading, cx: &mut Context<Self>) {
        self.battery_percent = reading.battery.percent.map(i64::from);
        self.on_external_power = Self::is_on_external_power(&reading);
        self.system_low_power_mode = reading.low_power_mode;
        self.recompute(cx);
    }

    fn persist_and_recompute(&mut self, cx: &mut Context<Self>) {
        if let Ok(data) = serde_json::to_vec(&self.settings) {
            UserDefaults::standard().set(ENERGY_SAVING, data);
        }
        self.recompute(cx);
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        self.is_active = RiseEnergySavingPolicy::is_active(
            self.settings.threshold_percent,
            self.battery_percent.filter(|_| !self.on_external_power),
            self.system_low_power_mode,
        );
        cx.notify();
        let next = RiseEnergySavingSnapshot {
            is_active: self.is_active,
            reduced_features: self.settings.reduced_features.clone(),
        };
        if next == Self::snapshot() {
            return;
        }
        Self::write_snapshot(next);
        cx.emit(RiseEnergySavingDidChange);
    }

    fn write_snapshot(next: RiseEnergySavingSnapshot) {
        *SNAPSHOT.write().unwrap_or_else(PoisonError::into_inner) = next;
    }

    fn load() -> RiseEnergySavingSettings {
        let Some(data) = UserDefaults::standard().data(ENERGY_SAVING) else {
            return RiseEnergySavingSettings::default();
        };
        let Ok(stored) = serde_json::from_slice::<RiseEnergySavingSettings>(&data) else {
            return RiseEnergySavingSettings::default();
        };
        RiseEnergySavingSettings {
            threshold_percent: RiseEnergySavingPolicy::normalized_threshold(
                stored.threshold_percent,
            ),
            reduced_features: stored.reduced_features,
        }
    }
}
