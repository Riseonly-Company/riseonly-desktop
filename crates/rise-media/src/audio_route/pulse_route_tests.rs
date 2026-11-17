use std::time::Duration;

use super::pulse_route::*;

#[test]
fn sink_and_server_events_are_route_events() {
    assert!(is_route_event("Event 'new' on sink #57"));
    assert!(is_route_event("Event 'remove' on sink #57"));
    assert!(is_route_event("Event 'change' on server #-1"));
}

#[test]
fn other_events_are_not_route_events() {
    assert!(!is_route_event("Event 'new' on sink-input #12"));
    assert!(!is_route_event("Event 'change' on source #3"));
    assert!(!is_route_event("Event 'remove' on client #40"));
    assert!(!is_route_event(""));
}

#[test]
fn default_sink_is_read_from_info() {
    let info = "Server String: /run/user/1000/pulse/native\n\
                Default Sink: alsa_output.pci-0000_00_1f.3.analog-stereo\n\
                Default Source: alsa_input.pci-0000_00_1f.3.analog-stereo\n";
    assert_eq!(
        default_sink(info).as_deref(),
        Some("alsa_output.pci-0000_00_1f.3.analog-stereo")
    );
}

#[test]
fn missing_or_unset_default_sink_is_none() {
    assert_eq!(default_sink("Server Name: PulseAudio (on PipeWire 1.0.5)\n"), None);
    assert_eq!(default_sink("Default Sink: n/a\n"), None);
    assert_eq!(default_sink("Default Sink:   \n"), None);
}

#[test]
fn sink_names_take_the_second_column() {
    let listing = "57\talsa_output.usb-headset.analog-stereo\tPipeWire\ts16le 2ch 48000Hz\tRUNNING\n\
                   58\tbluez_output.AA_BB.1\tPipeWire\ts16le 2ch 48000Hz\tSUSPENDED\n\
                   \n";
    let names = sink_names(listing);
    assert_eq!(names.len(), 2);
    assert!(names.contains("alsa_output.usb-headset.analog-stereo"));
    assert!(names.contains("bluez_output.AA_BB.1"));
}

#[test]
fn a_line_without_tabs_is_no_sink() {
    assert!(sink_names("no sinks here\n").is_empty());
}

#[test]
fn restart_delay_doubles_and_caps() {
    assert_eq!(restart_delay(0), FIRST_RESTART_DELAY);
    assert_eq!(restart_delay(1), FIRST_RESTART_DELAY * 2);
    assert_eq!(restart_delay(3), FIRST_RESTART_DELAY * 8);
    assert_eq!(restart_delay(10), MAX_RESTART_DELAY);
    assert_eq!(restart_delay(u32::MAX), MAX_RESTART_DELAY);
}

#[test]
fn a_stable_run_resets_the_backoff() {
    assert_eq!(next_attempt(4, STABLE_RUN), 0);
    assert_eq!(next_attempt(4, Duration::from_millis(10)), 5);
    assert_eq!(next_attempt(u32::MAX, Duration::ZERO), u32::MAX);
}
