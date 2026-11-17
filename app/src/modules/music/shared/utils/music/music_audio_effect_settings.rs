use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;

use rise_music::{EFFECTS, EffectKind, EffectScale, EffectScaleId, EffectSpec, MILLI_PER_UNIT, SCALES};

use super::music_effect_offline_renderer::MusicEffectMix;
use crate::modules::music::stores::music_actions::music_types::MusicListeningAudioEffect;

pub type MusicAudioEffectKind = EffectKind;
pub type MusicEffectKind = EffectKind;
pub type MusicEffectSpec = EffectSpec;
pub type MusicEffectScale = EffectScale;
pub type MusicEffectScaleId = EffectScaleId;

/// canon tables indexed once so every derived property is a table hit
/// effective_value runs per slider drag and per spin timer tick
pub enum MusicEffectCanon {}

impl MusicEffectCanon {
    pub fn specs() -> &'static [MusicEffectSpec] {
        &EFFECTS
    }

    pub fn scales() -> &'static [MusicEffectScale] {
        &SCALES
    }

    pub fn scale(id: MusicEffectScaleId) -> &'static MusicEffectScale {
        Self::scales().get(id.index()).unwrap_or(&SCALES[0])
    }

    pub fn unit(milli: i64) -> f64 {
        milli as f64 / MILLI_PER_UNIT as f64
    }

    pub fn milli(value: f64) -> i64 {
        (value * MILLI_PER_UNIT as f64).round() as i64
    }

    /// canon quantises in whole milli, clamp then snap to nearest step then clamp
    /// halves go away from zero
    /// float units disagree on exact halves so the math happens in the wire unit
    pub fn quantize_milli(milli: i64, scale: &MusicEffectScale) -> i64 {
        let clamped = scale.max_milli.min(scale.min_milli.max(milli));
        let snapped = ((clamped + scale.step_milli / 2) / scale.step_milli) * scale.step_milli;
        scale.max_milli.min(scale.min_milli.max(snapped))
    }
}

/// derived properties for the canon effect kind enum
pub trait MusicAudioEffectKindExt: Sized + 'static {
    fn raw_value(self) -> &'static str;
    fn from_raw_value(raw_value: &str) -> Option<Self>;
    fn all_cases() -> &'static [Self];
    fn canon_spec(self) -> &'static MusicEffectSpec;
    fn canon_scale(self) -> &'static MusicEffectScale;
    fn affects_media_time(self) -> bool;
    fn survives_thermal_pressure(self) -> bool;
    fn conflicting(self) -> Option<Self>;
    fn neutral_value(self) -> f64;
    fn default_value(self) -> f64;
    fn value_range(self) -> RangeInclusive<f64>;
    fn value_step(self) -> f64;
    fn quantize_milli(self, milli: i64) -> i64;
    fn quantize(self, value: f64) -> f64;
}

impl MusicAudioEffectKindExt for MusicAudioEffectKind {
    fn raw_value(self) -> &'static str {
        self.wire()
    }

    fn from_raw_value(raw_value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.wire() == raw_value)
    }

    fn all_cases() -> &'static [Self] {
        &Self::ALL
    }

    fn canon_spec(self) -> &'static MusicEffectSpec {
        MusicEffectCanon::specs()
            .get(self.index())
            .unwrap_or(&EFFECTS[0])
    }

    fn canon_scale(self) -> &'static MusicEffectScale {
        MusicEffectCanon::scale(self.canon_spec().scale)
    }

    fn affects_media_time(self) -> bool {
        self.canon_spec().affects_media_time
    }

    fn survives_thermal_pressure(self) -> bool {
        self.canon_spec().survives_thermal_pressure
    }

    fn conflicting(self) -> Option<Self> {
        self.canon_spec().conflicts_with
    }

    fn neutral_value(self) -> f64 {
        MusicEffectCanon::unit(self.canon_spec().neutral_milli)
    }

    fn default_value(self) -> f64 {
        MusicEffectCanon::unit(self.canon_spec().default_milli)
    }

    fn value_range(self) -> RangeInclusive<f64> {
        let scale = self.canon_scale();
        MusicEffectCanon::unit(scale.min_milli)..=MusicEffectCanon::unit(scale.max_milli)
    }

    fn value_step(self) -> f64 {
        MusicEffectCanon::unit(self.canon_scale().step_milli)
    }

    fn quantize_milli(self, milli: i64) -> i64 {
        MusicEffectCanon::quantize_milli(milli, self.canon_scale())
    }

    fn quantize(self, value: f64) -> f64 {
        MusicEffectCanon::unit(self.quantize_milli(MusicEffectCanon::milli(value)))
    }
}

/// everything the effect graph needs in one value, playback and exporter build the same chain from it
/// values keeps the last slider position even while off
/// effective_value is what the dsp reads, neutral while off
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicAudioEffectSettings {
    enabled: BTreeSet<&'static str>,
    values: BTreeMap<&'static str, f64>,
}

impl MusicAudioEffectSettings {
    /// settings with every effect off
    pub fn off() -> Self {
        Self::default()
    }

    pub fn is_any_enabled(&self) -> bool {
        !self.enabled.is_empty()
    }

    pub fn enabled_kinds(&self) -> Vec<MusicAudioEffectKind> {
        MusicAudioEffectKind::all_cases()
            .iter()
            .copied()
            .filter(|kind| self.enabled.contains(kind.raw_value()))
            .collect()
    }

    pub fn is_enabled(&self, kind: MusicAudioEffectKind) -> bool {
        self.enabled.contains(kind.raw_value())
    }

    /// remembered slider position, what the ui shows and what persists
    pub fn value(&self, kind: MusicAudioEffectKind) -> f64 {
        kind.quantize(
            self.values
                .get(kind.raw_value())
                .copied()
                .unwrap_or_else(|| kind.default_value()),
        )
    }

    /// what the dsp reads, an effect that is off contributes nothing
    pub fn effective_value(&self, kind: MusicAudioEffectKind) -> f64 {
        if self.is_enabled(kind) {
            self.value(kind)
        } else {
            kind.neutral_value()
        }
    }

    pub fn set_enabled(&mut self, kind: MusicAudioEffectKind, is_on: bool) {
        if is_on {
            self.enabled.insert(kind.raw_value());
            if let Some(conflicting) = kind.conflicting() {
                self.enabled.remove(conflicting.raw_value());
            }
        } else {
            self.enabled.remove(kind.raw_value());
        }
    }

    pub fn set_value(&mut self, kind: MusicAudioEffectKind, value: f64) {
        self.values.insert(kind.raw_value(), kind.quantize(value));
    }

    pub fn slowed_enabled(&self) -> bool {
        self.is_enabled(MusicAudioEffectKind::Slowed)
    }

    pub fn set_slowed_enabled(&mut self, new_value: bool) {
        self.set_enabled(MusicAudioEffectKind::Slowed, new_value);
    }

    pub fn rate(&self) -> f64 {
        self.value(MusicAudioEffectKind::Slowed)
    }

    pub fn set_rate(&mut self, new_value: f64) {
        self.set_value(MusicAudioEffectKind::Slowed, new_value);
    }

    /// only the tempo shifts media time, other effects are purely tonal
    pub fn playback_rate(&self) -> f64 {
        if self.is_enabled(MusicAudioEffectKind::Nightcore) {
            return self.effective_value(MusicAudioEffectKind::Nightcore);
        }
        self.effective_value(MusicAudioEffectKind::Slowed)
    }

    /// enabled effects only, sorted by kind
    /// server stores this as a canonical string and compares it so the ordering is part of the contract
    pub fn wire_effects(&self) -> Vec<MusicListeningAudioEffect> {
        let mut kinds: Vec<MusicAudioEffectKind> = MusicAudioEffectKind::all_cases()
            .iter()
            .copied()
            .filter(|kind| self.is_enabled(*kind))
            .collect();
        kinds.sort_by(|left, right| left.raw_value().cmp(right.raw_value()));
        kinds
            .into_iter()
            .map(|kind| MusicListeningAudioEffect {
                kind: kind.raw_value().to_owned(),
                value_milli: MusicEffectCanon::milli(self.value(kind)),
            })
            .collect()
    }

    pub fn wire_canonical(&self) -> String {
        self.wire_effects()
            .iter()
            .map(|effect| {
                format!(
                    "{}{}{}",
                    effect.kind,
                    rise_music::WIRE_PAIR_SEPARATOR,
                    effect.value_milli
                )
            })
            .collect::<Vec<_>>()
            .join(rise_music::WIRE_ENTRY_SEPARATOR)
    }

    /// what the graph renders while the device is too hot, expensive effects drop out
    /// user settings are untouched so the original chain returns once it cools
    pub fn thermally_degraded(&self) -> Self {
        let mut degraded = self.clone();
        for kind in MusicAudioEffectKind::all_cases() {
            if !kind.survives_thermal_pressure() {
                degraded.set_enabled(*kind, false);
            }
        }
        degraded
    }

    /// adopting a sessions effects keeps this devices remembered slider
    /// positions for everything the session doesnt list
    /// so leaving a session and switching an effect back on restores your own value
    pub fn from_wire_effects(
        wire_effects: &[MusicListeningAudioEffect],
        remembered_values: Self,
    ) -> Self {
        let mut settings = remembered_values;
        settings.enabled.clear();
        for effect in wire_effects {
            let Some(kind) = MusicAudioEffectKind::from_raw_value(&effect.kind) else {
                continue;
            };
            settings.enabled.insert(kind.raw_value());
            settings.values.insert(
                kind.raw_value(),
                MusicEffectCanon::unit(kind.quantize_milli(effect.value_milli)),
            );
        }
        settings
    }

    pub fn from_wire_effects_only(wire_effects: &[MusicListeningAudioEffect]) -> Self {
        Self::from_wire_effects(wire_effects, Self::off())
    }

    /// default settings
    pub fn new() -> Self {
        Self::default()
    }
}

/// reverb voicings the chain switches between, rebuilt by the desktop reverb in the graph file
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AVAudioUnitReverbPreset {
    MediumChamber,
    MediumHall,
    LargeHall,
    Cathedral,
}

/// maps the user facing sliders onto the actual unit parameters
pub enum MusicAudioEffectDSP {}

impl MusicAudioEffectDSP {
    /// one reverb serves every spatial effect, the wettest request wins so combinations stay predictable
    pub fn reverb_wet_dry_mix(settings: &MusicAudioEffectSettings) -> f32 {
        let mut mix: f32 = 0.0;
        if settings.is_enabled(MusicAudioEffectKind::Slowed) {
            mix = mix.max(MusicEffectMix::REVERB_WET_DRY_MIX);
        }
        if settings.is_enabled(MusicAudioEffectKind::Distance) {
            mix = mix.max(30.0 + 45.0 * settings.effective_value(MusicAudioEffectKind::Distance) as f32);
        }
        if settings.is_enabled(MusicAudioEffectKind::Underwater) {
            mix = mix.max(25.0 + 35.0 * settings.effective_value(MusicAudioEffectKind::Underwater) as f32);
        }
        if settings.is_enabled(MusicAudioEffectKind::Concert) {
            mix = mix.max(35.0 + 40.0 * settings.effective_value(MusicAudioEffectKind::Concert) as f32);
        }
        mix.min(100.0)
    }

    pub fn reverb_preset(settings: &MusicAudioEffectSettings) -> AVAudioUnitReverbPreset {
        if settings.is_enabled(MusicAudioEffectKind::Concert) {
            return AVAudioUnitReverbPreset::Cathedral;
        }
        if settings.is_enabled(MusicAudioEffectKind::Underwater) {
            return AVAudioUnitReverbPreset::MediumChamber;
        }
        AVAudioUnitReverbPreset::LargeHall
    }

    /// walls eat treble long before bass so distance is mostly a low
    /// pass that closes as the listener walks away
    /// underwater is the same idea taken further
    pub fn low_pass_hz(settings: &MusicAudioEffectSettings) -> f32 {
        let mut cutoff: f32 = 20_000.0;
        if settings.is_enabled(MusicAudioEffectKind::Distance) {
            cutoff = cutoff
                .min(6_500.0 - 5_600.0 * settings.effective_value(MusicAudioEffectKind::Distance) as f32);
        }
        if settings.is_enabled(MusicAudioEffectKind::Underwater) {
            cutoff = cutoff
                .min(1_400.0 - 1_100.0 * settings.effective_value(MusicAudioEffectKind::Underwater) as f32);
        }
        if settings.is_enabled(MusicAudioEffectKind::Vinyl) {
            cutoff = cutoff
                .min(12_000.0 - 5_000.0 * settings.effective_value(MusicAudioEffectKind::Vinyl) as f32);
        }
        cutoff.max(200.0)
    }

    pub fn is_low_pass_active(settings: &MusicAudioEffectSettings) -> bool {
        settings.is_enabled(MusicAudioEffectKind::Distance)
            || settings.is_enabled(MusicAudioEffectKind::Underwater)
            || settings.is_enabled(MusicAudioEffectKind::Vinyl)
    }

    /// a touch of low shelf keeps a muffled signal from sounding thin and telephone like
    pub fn low_shelf_gain_db(settings: &MusicAudioEffectSettings) -> f32 {
        let mut gain: f32 = 0.0;
        if settings.is_enabled(MusicAudioEffectKind::Distance) {
            gain += 1.5 + 3.5 * settings.effective_value(MusicAudioEffectKind::Distance) as f32;
        }
        if settings.is_enabled(MusicAudioEffectKind::BassBoost) {
            gain += 2.0 + 10.0 * settings.effective_value(MusicAudioEffectKind::BassBoost) as f32;
        }
        gain.min(18.0)
    }

    pub fn is_low_shelf_active(settings: &MusicAudioEffectSettings) -> bool {
        settings.is_enabled(MusicAudioEffectKind::Distance)
            || settings.is_enabled(MusicAudioEffectKind::BassBoost)
    }

    /// vinyl and lo-fi lose the deepest bottom which makes a record sound like a record
    pub fn high_pass_hz(settings: &MusicAudioEffectSettings) -> f32 {
        if !settings.is_enabled(MusicAudioEffectKind::Vinyl) {
            return 20.0;
        }
        40.0 + 160.0 * settings.effective_value(MusicAudioEffectKind::Vinyl) as f32
    }

    pub fn is_high_pass_active(settings: &MusicAudioEffectSettings) -> bool {
        settings.is_enabled(MusicAudioEffectKind::Vinyl)
    }

    pub fn direct_gain(settings: &MusicAudioEffectSettings) -> f32 {
        let mut gain: f32 = 1.0;
        if settings.is_enabled(MusicAudioEffectKind::Distance) {
            gain *= 1.0 - 0.25 * settings.effective_value(MusicAudioEffectKind::Distance) as f32;
        }
        if settings.is_enabled(MusicAudioEffectKind::Underwater) {
            gain *= 1.0 - 0.2 * settings.effective_value(MusicAudioEffectKind::Underwater) as f32;
        }
        if settings.is_enabled(MusicAudioEffectKind::BassBoost) {
            gain *= 1.0 - 0.18 * settings.effective_value(MusicAudioEffectKind::BassBoost) as f32;
        }
        gain.max(0.2)
    }

    /// independent of tempo, varispeed already drags pitch with speed
    /// this changes the voice without changing timing
    pub fn pitch_cents(settings: &MusicAudioEffectSettings) -> f32 {
        if !settings.is_enabled(MusicAudioEffectKind::Pitch) {
            return 0.0;
        }
        ((settings.effective_value(MusicAudioEffectKind::Pitch) - 0.5) * 2.0 * 1_200.0) as f32
    }

    pub fn delay_wet_dry_mix(settings: &MusicAudioEffectSettings) -> f32 {
        if !settings.is_enabled(MusicAudioEffectKind::Echo) {
            return 0.0;
        }
        10.0 + 45.0 * settings.effective_value(MusicAudioEffectKind::Echo) as f32
    }

    pub fn delay_feedback(settings: &MusicAudioEffectSettings) -> f32 {
        if !settings.is_enabled(MusicAudioEffectKind::Echo) {
            return 0.0;
        }
        15.0 + 55.0 * settings.effective_value(MusicAudioEffectKind::Echo) as f32
    }

    pub const DELAY_TIME_SECONDS: f64 = 0.32;

    /// night mode pulls quiet parts up so headphones stay
    /// listenable at low volume without loud parts jumping
    pub fn compression_threshold_db(settings: &MusicAudioEffectSettings) -> f32 {
        if !settings.is_enabled(MusicAudioEffectKind::NightMode) {
            return 0.0;
        }
        -12.0 - 26.0 * settings.effective_value(MusicAudioEffectKind::NightMode) as f32
    }

    pub fn compression_makeup_db(settings: &MusicAudioEffectSettings) -> f32 {
        if !settings.is_enabled(MusicAudioEffectKind::NightMode) {
            return 0.0;
        }
        3.0 + 7.0 * settings.effective_value(MusicAudioEffectKind::NightMode) as f32
    }

    pub fn spin_hz(settings: &MusicAudioEffectSettings) -> f64 {
        if !settings.is_enabled(MusicAudioEffectKind::Spin) {
            return 0.0;
        }
        0.04 + 0.16 * settings.effective_value(MusicAudioEffectKind::Spin)
    }

    pub fn ambience_gain(settings: &MusicAudioEffectSettings, kind: MusicAudioEffectKind) -> f32 {
        if !settings.is_enabled(kind) {
            return 0.0;
        }
        let intensity = settings.effective_value(kind) as f32;
        match kind {
            MusicAudioEffectKind::Vinyl => 0.02 + 0.16 * intensity,
            _ => 0.0,
        }
    }
}
