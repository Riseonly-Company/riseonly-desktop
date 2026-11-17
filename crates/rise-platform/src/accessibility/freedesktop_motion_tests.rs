use crate::accessibility::freedesktop_motion::*;

#[test]
fn reduced_motion_of_the_appearance_namespace() {
    let source = MotionSource::Appearance;
    assert_eq!(source.reduce_from(&SettingValue::Unsigned(0)), Some(false));
    assert_eq!(source.reduce_from(&SettingValue::Unsigned(1)), Some(true));
    assert_eq!(source.reduce_from(&SettingValue::Signed(1)), Some(true));
    assert_eq!(source.reduce_from(&SettingValue::Unsigned(2)), None);
    assert_eq!(source.reduce_from(&SettingValue::Signed(-1)), None);
    assert_eq!(source.reduce_from(&SettingValue::Bool(true)), None);
}

#[test]
fn gnome_animations_off_is_reduce() {
    let source = MotionSource::Gnome;
    assert_eq!(source.reduce_from(&SettingValue::Bool(false)), Some(true));
    assert_eq!(source.reduce_from(&SettingValue::Bool(true)), Some(false));
    assert_eq!(source.reduce_from(&SettingValue::Unsigned(0)), None);
}

#[test]
fn kde_instant_animations_are_reduce() {
    let source = MotionSource::Kde;
    assert_eq!(source.reduce_from(&SettingValue::Double(0.0)), Some(true));
    assert_eq!(source.reduce_from(&SettingValue::Double(0.5)), Some(false));
    assert_eq!(source.reduce_from(&SettingValue::Text("0".to_owned())), Some(true));
    assert_eq!(source.reduce_from(&SettingValue::Text("1.5".to_owned())), Some(false));
    assert_eq!(source.reduce_from(&SettingValue::Unsigned(0)), Some(true));
    assert_eq!(source.reduce_from(&SettingValue::Text("fast".to_owned())), None);
    assert_eq!(source.reduce_from(&SettingValue::Double(-1.0)), None);
    assert_eq!(source.reduce_from(&SettingValue::Double(f64::NAN)), None);
    assert_eq!(source.reduce_from(&SettingValue::Other), None);
}

#[test]
fn a_signal_is_matched_by_namespace_and_key() {
    assert_eq!(
        MotionSource::of(APPEARANCE_NAMESPACE, REDUCED_MOTION_KEY),
        Some(MotionSource::Appearance)
    );
    assert_eq!(
        MotionSource::of(GNOME_INTERFACE_NAMESPACE, ENABLE_ANIMATIONS_KEY),
        Some(MotionSource::Gnome)
    );
    assert_eq!(
        MotionSource::of(KDE_GLOBALS_NAMESPACE, ANIMATION_DURATION_FACTOR_KEY),
        Some(MotionSource::Kde)
    );
    assert_eq!(MotionSource::of(APPEARANCE_NAMESPACE, "color-scheme"), None);
    assert_eq!(MotionSource::of(GNOME_INTERFACE_NAMESPACE, REDUCED_MOTION_KEY), None);
}

#[test]
fn nothing_answered_is_no_value() {
    assert_eq!(MotionSettings::default().resolved(), None);
}

#[test]
fn the_appearance_key_wins_over_the_desktop_keys() {
    let mut settings = MotionSettings::default();
    settings.record(MotionSource::Gnome, &SettingValue::Bool(false));
    settings.record(MotionSource::Kde, &SettingValue::Double(1.0));
    assert_eq!(settings.resolved(), Some(true));
    settings.record(MotionSource::Appearance, &SettingValue::Unsigned(0));
    assert_eq!(settings.resolved(), Some(false));
}

#[test]
fn gnome_wins_over_kde() {
    let mut settings = MotionSettings::default();
    settings.record(MotionSource::Kde, &SettingValue::Double(0.0));
    assert_eq!(settings.resolved(), Some(true));
    settings.record(MotionSource::Gnome, &SettingValue::Bool(true));
    assert_eq!(settings.resolved(), Some(false));
}

#[test]
fn an_undocumented_value_clears_its_key() {
    let mut settings = MotionSettings::default();
    settings.record(MotionSource::Appearance, &SettingValue::Unsigned(1));
    settings.record(MotionSource::Gnome, &SettingValue::Bool(true));
    settings.record(MotionSource::Appearance, &SettingValue::Unsigned(7));
    assert_eq!(settings.resolved(), Some(false));
}

#[test]
fn a_change_of_another_setting_delivers_nothing() {
    let mut settings = MotionSettings::default();
    settings.record(MotionSource::Gnome, &SettingValue::Bool(false));
    assert_eq!(
        settings.apply_change(APPEARANCE_NAMESPACE, "color-scheme", &SettingValue::Unsigned(1)),
        None
    );
    assert_eq!(settings.resolved(), Some(true));
}

#[test]
fn a_motion_change_delivers_the_resolved_value() {
    let mut settings = MotionSettings::default();
    assert_eq!(
        settings.apply_change(
            GNOME_INTERFACE_NAMESPACE,
            ENABLE_ANIMATIONS_KEY,
            &SettingValue::Bool(false)
        ),
        Some(true)
    );
    assert_eq!(
        settings.apply_change(
            APPEARANCE_NAMESPACE,
            REDUCED_MOTION_KEY,
            &SettingValue::Unsigned(0)
        ),
        Some(false)
    );
    assert_eq!(
        settings.apply_change(
            GNOME_INTERFACE_NAMESPACE,
            ENABLE_ANIMATIONS_KEY,
            &SettingValue::Bool(true)
        ),
        Some(false)
    );
}

#[test]
fn the_known_value_survives_its_byte() {
    for known in [
        KnownMotion::Unknown,
        KnownMotion::Known(false),
        KnownMotion::Known(true),
    ] {
        assert_eq!(KnownMotion::from_code(known.code()), known);
    }
    assert_eq!(KnownMotion::from_code(0).value(), None);
    assert_eq!(KnownMotion::Known(true).value(), Some(true));
    assert_eq!(KnownMotion::from_code(200), KnownMotion::Unknown);
}
