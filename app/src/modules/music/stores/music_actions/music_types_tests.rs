use rise_music::ThermalVerdict as MusicEffectThermalVerdict;
use rise_platform::process_info::ThermalState;

use crate::modules::music::shared::utils::music::music_audio_effect_settings::{
    MusicAudioEffectKind, MusicAudioEffectKindExt, MusicAudioEffectSettings,
};
use crate::modules::music::shared::utils::music::music_effect_thermal_guard::MusicEffectThermalPolicy;
use crate::modules::music::stores::music_actions::music_types::{
    MusicListeningAudioEffect, MusicListeningAudioEffectWire, MusicListeningCommandPipelinePolicy,
    MusicListeningParticipantPermissions, MusicListeningSessionCommand,
    MusicListeningSessionStateVector,
};

fn effect(kind: &str, value_milli: i64) -> MusicListeningAudioEffect {
    MusicListeningAudioEffect {
        kind: kind.to_owned(),
        value_milli,
    }
}

#[test]
fn canonical_form_is_sorted_enabled_only_and_matches_the_server_encoding() {
    let mut settings = MusicAudioEffectSettings::off();
    settings.set_enabled(MusicAudioEffectKind::Spin, true);
    settings.set_value(MusicAudioEffectKind::Spin, 0.4);
    settings.set_enabled(MusicAudioEffectKind::BassBoost, true);
    settings.set_value(MusicAudioEffectKind::BassBoost, 1.0);
    settings.set_enabled(MusicAudioEffectKind::Slowed, true);
    settings.set_value(MusicAudioEffectKind::Slowed, 0.8);
    settings.set_value(MusicAudioEffectKind::Echo, 0.75);

    assert_eq!(settings.wire_canonical(), "bass_boost:1000,slowed:800,spin:400");
    assert_eq!(settings.wire_effects().len(), 3);
    assert_eq!(MusicAudioEffectSettings::off().wire_canonical(), "");
}

#[test]
fn adopting_a_session_keeps_remembered_values_for_effects_it_does_not_list() {
    let mut local = MusicAudioEffectSettings::off();
    local.set_enabled(MusicAudioEffectKind::Echo, true);
    local.set_value(MusicAudioEffectKind::Echo, 0.75);
    local.set_value(MusicAudioEffectKind::Spin, 0.9);

    let adopted = MusicAudioEffectSettings::from_wire_effects(&[effect("slowed", 700)], local);

    assert!(adopted.is_enabled(MusicAudioEffectKind::Slowed));
    assert!((adopted.value(MusicAudioEffectKind::Slowed) - 0.7).abs() < 0.0001);
    assert!(!adopted.is_enabled(MusicAudioEffectKind::Echo));
    assert!((adopted.value(MusicAudioEffectKind::Echo) - 0.75).abs() < 0.0001);
    assert!((adopted.value(MusicAudioEffectKind::Spin) - 0.9).abs() < 0.0001);
}

#[test]
fn canonical_round_trips_and_rejects_unknown_kinds() {
    let canonical = MusicListeningAudioEffectWire::canonical(&[
        effect("vinyl", 300),
        effect("reverb", 500),
        effect("nightcore", 1_250),
        effect("vinyl", 900),
    ]);

    assert_eq!(canonical, "nightcore:1250,vinyl:300");
    assert_eq!(MusicListeningAudioEffectWire::effects(&canonical).len(), 2);
}

#[test]
fn tempo_rate_prefers_nightcore_and_falls_back_to_neutral() {
    assert_eq!(MusicListeningAudioEffectWire::tempo_rate(""), 1.0);
    assert_eq!(MusicListeningAudioEffectWire::tempo_rate("spin:400"), 1.0);
    assert_eq!(MusicListeningAudioEffectWire::tempo_rate("slowed:800,spin:400"), 0.8);
    assert_eq!(MusicListeningAudioEffectWire::tempo_rate("nightcore:1250"), 1.25);
}

fn state_vector(
    is_playing: bool,
    audio_effect_enabled: bool,
    audio_effect_rate_milli: i64,
    audio_effects_canonical: &str,
) -> MusicListeningSessionStateVector {
    MusicListeningSessionStateVector {
        track_id: "track-a".to_owned(),
        position_ms: 5_000,
        is_playing,
        shuffle_enabled: false,
        shuffle_seed: 0,
        repeat_mode: "off".to_owned(),
        audio_effect_enabled,
        audio_effect_rate_milli,
        audio_effects_canonical: audio_effects_canonical.to_owned(),
    }
}

#[test]
fn pure_effect_change_declares_the_effects_command() {
    let base = state_vector(true, false, 1_000, "");
    let with_effect = state_vector(true, true, 800, "slowed:800");

    assert_eq!(
        MusicListeningCommandPipelinePolicy::resolved_command(
            MusicListeningSessionCommand::Heartbeat,
            Some(&base),
            &with_effect,
        ),
        MusicListeningSessionCommand::Effects
    );
    let paused_with_effect = state_vector(false, true, 800, "slowed:800");
    assert_eq!(
        MusicListeningCommandPipelinePolicy::resolved_command(
            MusicListeningSessionCommand::Heartbeat,
            Some(&base),
            &paused_with_effect,
        ),
        MusicListeningSessionCommand::Playback
    );
}

#[test]
fn effects_permission_is_not_implied_by_playback_control() {
    let playback_only = MusicListeningParticipantPermissions {
        can_control_playback: true,
        can_seek: true,
        can_change_tracks: true,
        can_apply_effects: false,
    };

    assert!(!playback_only.can_apply_effects);
    assert!(MusicListeningParticipantPermissions::HOST.can_apply_effects);
    assert!(!MusicListeningParticipantPermissions::NONE.can_apply_effects);
}

fn settings_with_everything_on() -> MusicAudioEffectSettings {
    let mut settings = MusicAudioEffectSettings::off();
    for kind in MusicAudioEffectKind::all_cases() {
        if *kind != MusicAudioEffectKind::Nightcore {
            settings.set_enabled(*kind, true);
        }
    }
    settings
}

#[test]
fn degradation_keeps_tempo_and_drops_only_the_expensive_nodes() {
    let degraded = settings_with_everything_on().thermally_degraded();

    assert!(degraded.is_enabled(MusicAudioEffectKind::Slowed));
    assert!(degraded.is_enabled(MusicAudioEffectKind::Pitch));
    assert!(degraded.is_enabled(MusicAudioEffectKind::BassBoost));
    assert!(degraded.is_enabled(MusicAudioEffectKind::NightMode));
    assert!(degraded.is_enabled(MusicAudioEffectKind::Vinyl));
    assert!(!degraded.is_enabled(MusicAudioEffectKind::Spin));
    assert!(!degraded.is_enabled(MusicAudioEffectKind::Echo));
    assert!(!degraded.is_enabled(MusicAudioEffectKind::Concert));
    assert!(!degraded.is_enabled(MusicAudioEffectKind::Distance));
    assert!(!degraded.is_enabled(MusicAudioEffectKind::Underwater));
}

#[test]
fn tempo_is_never_dropped_because_it_would_desync_the_listener() {
    for kind in MusicAudioEffectKind::all_cases() {
        if kind.affects_media_time() {
            assert!(kind.survives_thermal_pressure());
        }
    }
}

#[test]
fn own_effects_never_evict() {
    assert_eq!(
        MusicEffectThermalPolicy::verdict(
            ThermalState::Critical,
            10_000.0,
            false,
            MusicEffectThermalVerdict::Full,
        ),
        MusicEffectThermalVerdict::Full
    );
}

#[test]
fn sustained_serious_degrades_then_evicts() {
    assert_eq!(
        MusicEffectThermalPolicy::verdict(
            ThermalState::Serious,
            44.0,
            true,
            MusicEffectThermalVerdict::Full,
        ),
        MusicEffectThermalVerdict::Full
    );
    assert_eq!(
        MusicEffectThermalPolicy::verdict(
            ThermalState::Serious,
            45.0,
            true,
            MusicEffectThermalVerdict::Full,
        ),
        MusicEffectThermalVerdict::Degraded
    );
    assert_eq!(
        MusicEffectThermalPolicy::verdict(
            ThermalState::Serious,
            100.0,
            true,
            MusicEffectThermalVerdict::Degraded,
        ),
        MusicEffectThermalVerdict::Degraded
    );
    assert_eq!(
        MusicEffectThermalPolicy::verdict(
            ThermalState::Serious,
            135.0,
            true,
            MusicEffectThermalVerdict::Degraded,
        ),
        MusicEffectThermalVerdict::Evicted
    );
}

#[test]
fn critical_evicts_immediately_without_waiting_to_degrade_first() {
    assert_eq!(
        MusicEffectThermalPolicy::verdict(
            ThermalState::Critical,
            0.0,
            true,
            MusicEffectThermalVerdict::Full,
        ),
        MusicEffectThermalVerdict::Evicted
    );
}

#[test]
fn cooling_down_restores_the_full_chain_but_not_the_session() {
    assert_eq!(
        MusicEffectThermalPolicy::verdict(
            ThermalState::Fair,
            0.0,
            true,
            MusicEffectThermalVerdict::Degraded,
        ),
        MusicEffectThermalVerdict::Full
    );
    assert_eq!(
        MusicEffectThermalPolicy::verdict(
            ThermalState::Nominal,
            0.0,
            true,
            MusicEffectThermalVerdict::Evicted,
        ),
        MusicEffectThermalVerdict::Evicted
    );
}

#[test]
fn rejoin_needs_both_the_cooldown_and_a_cooled_device() {
    assert!(MusicEffectThermalPolicy::allows_rejoin(ThermalState::Critical, None));
    assert!(!MusicEffectThermalPolicy::allows_rejoin(ThermalState::Nominal, Some(60.0)));
    assert!(!MusicEffectThermalPolicy::allows_rejoin(ThermalState::Serious, Some(3_600.0)));
    assert!(MusicEffectThermalPolicy::allows_rejoin(ThermalState::Fair, Some(601.0)));
}
