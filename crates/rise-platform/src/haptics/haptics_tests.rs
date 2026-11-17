use crate::gpui_shim::PlatformSupport;
use crate::haptics::*;
use crate::host_os::HostOs;

const STYLES: [ImpactStyle; 5] = [
    ImpactStyle::Light,
    ImpactStyle::Medium,
    ImpactStyle::Heavy,
    ImpactStyle::Soft,
    ImpactStyle::Rigid,
];

const KINDS: [NotificationKind; 3] = [
    NotificationKind::Success,
    NotificationKind::Warning,
    NotificationKind::Error,
];

#[test]
fn a_selection_change_is_an_alignment() {
    assert_eq!(pattern_for(Feedback::Selection), HapticPattern::Alignment);
}

#[test]
fn every_impact_style_is_the_generic_bump() {
    for style in STYLES {
        assert_eq!(pattern_for(Feedback::Impact(style)), HapticPattern::Generic);
    }
}

#[test]
fn every_notification_kind_is_a_level_change() {
    for kind in KINDS {
        assert_eq!(
            pattern_for(Feedback::Notification(kind)),
            HapticPattern::LevelChange
        );
    }
}

#[test]
fn the_plain_impact_generator_is_medium_as_on_ios() {
    assert_eq!(ImpactFeedbackGenerator::default().style(), ImpactStyle::Medium);
    assert_eq!(
        ImpactFeedbackGenerator::new(ImpactStyle::Rigid).style(),
        ImpactStyle::Rigid
    );
}

#[test]
fn linux_and_windows_name_the_missing_system_api() {
    for host in [HostOs::Linux, HostOs::Windows] {
        assert_eq!(
            unavailable_by_default(host),
            HapticsUnavailable::NoSystemHaptics(host)
        );
    }
}

#[test]
fn an_unwritten_macos_layer_says_so_instead_of_no_haptics() {
    assert_eq!(
        unavailable_by_default(HostOs::MacOs),
        HapticsUnavailable::BranchNotWritten(HostOs::MacOs)
    );
}

#[test]
fn every_reason_names_its_host() {
    for host in HostOs::ALL {
        let text = unavailable_by_default(host).to_string();
        assert!(text.contains(&format!("{host:?}")), "{text}");
    }
}

#[test]
fn the_answer_turns_into_platform_support() {
    assert_eq!(support_of(Ok(())), PlatformSupport::Performed);
    assert_eq!(
        support_of(Err(HapticsUnavailable::NoSystemHaptics(HostOs::Linux))),
        PlatformSupport::Unsupported
    );
}

#[test]
fn a_layer_without_overrides_refuses_with_the_host_default() {
    struct Bare;
    impl HapticPerformer for Bare {}
    let expected = Err(unavailable_by_default(HostOs::current()));
    assert_eq!(Bare.available(), expected);
    assert_eq!(Bare.perform(HapticPattern::Generic), expected);
}

#[test]
fn every_call_agrees_with_the_host_availability() {
    let expected = support_of(availability());
    assert_eq!(prepare(), expected);
    assert_eq!(selection_changed(), expected);
    assert_eq!(SelectionFeedbackGenerator::new().selection_changed(), expected);
    for style in STYLES {
        let generator = ImpactFeedbackGenerator::new(style);
        assert_eq!(generator.prepare(), expected);
        assert_eq!(generator.impact_occurred(), expected);
        assert_eq!(generator.impact_occurred_with_intensity(0.7), expected);
    }
    for kind in KINDS {
        assert_eq!(NotificationFeedbackGenerator::new().notification_occurred(kind), expected);
    }
}

#[test]
fn only_macos_plays_haptics() {
    let plays = availability().is_ok();
    assert_eq!(plays, HostOs::current() == HostOs::MacOs);
}
