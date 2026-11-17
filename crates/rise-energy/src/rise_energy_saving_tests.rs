use std::collections::BTreeSet;

use crate::rise_energy_saving::{
    RiseEnergySavingFeature, RiseEnergySavingPolicy, RiseEnergySavingSettings,
    RiseEnergySavingSnapshot,
};

#[test]
fn normalized_threshold_snaps_to_nearest_stop_lower_on_tie() {
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(0), 0);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(5), 0);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(15), 10);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(25), 20);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(35), 30);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(40), 30);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(45), 50);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(75), 50);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(80), 100);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(-40), 0);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(500), 100);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(i64::MIN), 0);
    assert_eq!(RiseEnergySavingPolicy::normalized_threshold(i64::MAX), 100);
}

#[test]
fn threshold_index_follows_the_normalized_stop() {
    assert_eq!(RiseEnergySavingPolicy::threshold_index(0), 0);
    assert_eq!(RiseEnergySavingPolicy::threshold_index(20), 2);
    assert_eq!(RiseEnergySavingPolicy::threshold_index(45), 4);
    assert_eq!(RiseEnergySavingPolicy::threshold_index(100), 5);
}

#[test]
fn is_active_low_power_mode_wins_unless_switched_off() {
    assert!(!RiseEnergySavingPolicy::is_active(0, None, true));
    assert!(RiseEnergySavingPolicy::is_active(20, Some(90), true));
}

#[test]
fn is_active_off_and_always_ignore_the_battery() {
    assert!(!RiseEnergySavingPolicy::is_active(0, Some(1), false));
    assert!(RiseEnergySavingPolicy::is_active(100, None, false));
    assert!(RiseEnergySavingPolicy::is_active(100, Some(100), false));
}

#[test]
fn is_active_between_compares_battery_at_or_below() {
    assert!(RiseEnergySavingPolicy::is_active(20, Some(20), false));
    assert!(RiseEnergySavingPolicy::is_active(20, Some(5), false));
    assert!(!RiseEnergySavingPolicy::is_active(20, Some(21), false));
    assert!(!RiseEnergySavingPolicy::is_active(20, None, false));
}

#[test]
fn battery_percent_rounds_and_rejects_negative_level() {
    assert_eq!(RiseEnergySavingPolicy::battery_percent(-1.0), None);
    assert_eq!(RiseEnergySavingPolicy::battery_percent(0.0), Some(0));
    assert_eq!(RiseEnergySavingPolicy::battery_percent(0.206), Some(21));
    assert_eq!(RiseEnergySavingPolicy::battery_percent(1.0), Some(100));
}

#[test]
fn default_settings_reduce_every_feature_at_twenty() {
    let settings = RiseEnergySavingSettings::default();
    assert_eq!(settings.threshold_percent, RiseEnergySavingPolicy::DEFAULT_THRESHOLD_PERCENT);
    assert_eq!(settings.reduced_features.len(), RiseEnergySavingFeature::ALL.len());
}

#[test]
fn settings_json_matches_swift_codable_keys() {
    let settings = RiseEnergySavingSettings {
        threshold_percent: 30,
        reduced_features: BTreeSet::from([RiseEnergySavingFeature::MediaPrefetch]),
    };
    let encoded = serde_json::to_value(&settings).ok();
    let expected = serde_json::json!({
        "thresholdPercent": 30,
        "reducedFeatures": ["mediaPrefetch"],
    });
    assert_eq!(encoded, Some(expected));
}

#[test]
fn settings_json_from_ios_decodes() {
    let stored = br#"{"thresholdPercent":15,"reducedFeatures":["animatedGifs","loopAnimations"]}"#;
    let decoded = serde_json::from_slice::<RiseEnergySavingSettings>(stored).ok();
    let expected = RiseEnergySavingSettings {
        threshold_percent: 15,
        reduced_features: BTreeSet::from([
            RiseEnergySavingFeature::AnimatedGifs,
            RiseEnergySavingFeature::LoopAnimations,
        ]),
    };
    assert_eq!(decoded, Some(expected));
}

#[test]
fn feature_raw_values_round_trip() {
    for feature in RiseEnergySavingFeature::ALL {
        assert_eq!(RiseEnergySavingFeature::from_raw_value(feature.raw_value()), Some(feature));
        let encoded = serde_json::to_value(feature).ok();
        assert_eq!(encoded, Some(serde_json::Value::from(feature.raw_value())));
    }
}

#[test]
fn snapshot_reduces_only_when_active() {
    let reduced = BTreeSet::from([RiseEnergySavingFeature::AnimatedEmoji]);
    let active = RiseEnergySavingSnapshot {
        is_active: true,
        reduced_features: reduced.clone(),
    };
    let idle = RiseEnergySavingSnapshot {
        is_active: false,
        reduced_features: reduced,
    };
    assert!(active.reduces(RiseEnergySavingFeature::AnimatedEmoji));
    assert!(!active.reduces(RiseEnergySavingFeature::AnimatedGifs));
    assert!(!idle.reduces(RiseEnergySavingFeature::AnimatedEmoji));
    assert!(!RiseEnergySavingSnapshot::NONE.reduces(RiseEnergySavingFeature::MediaPrefetch));
}
