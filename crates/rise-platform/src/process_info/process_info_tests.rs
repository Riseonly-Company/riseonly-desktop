use std::time::Duration;

use crate::host_os::HostOs;
use crate::process_info::*;

#[test]
fn raw_values_round_trip_with_the_ios_numbers() {
    for (state, raw) in ThermalState::ALL.into_iter().zip(0..) {
        assert_eq!(state.raw_value(), raw, "{state:?} keeps the iOS raw value");
        assert_eq!(ThermalState::from_raw_value(raw), Some(state));
    }
    assert_eq!(ThermalState::from_raw_value(4), None);
    assert_eq!(ThermalState::from_raw_value(-1), None);
}

#[test]
fn names_match_what_the_bug_report_sends() {
    let names: Vec<&str> = ThermalState::ALL.iter().map(|state| state.name()).collect();
    assert_eq!(names, ["nominal", "fair", "serious", "critical"]);
}

#[test]
fn states_order_from_calm_to_worst() {
    assert!(ThermalState::Nominal < ThermalState::Fair);
    assert!(ThermalState::Fair < ThermalState::Serious);
    assert!(ThermalState::Serious < ThermalState::Critical);
}

#[test]
fn a_missing_thermal_answer_reads_nominal_with_its_reason() {
    for host in HostOs::ALL {
        let reason = unavailable_by_default(ProcessReading::ThermalState, host);
        let reading = ThermalReading::from_result(Err(reason.clone()));
        assert_eq!(reading.state, ThermalState::Nominal);
        assert_eq!(reading.unavailable, Some(reason));
    }
}

#[test]
fn a_system_answer_carries_no_reason() {
    let reading = ThermalReading::from_result(Ok(ThermalState::Serious));
    assert_eq!(reading.state, ThermalState::Serious);
    assert_eq!(reading.unavailable, None);
}

#[test]
fn only_macos_treats_a_missing_thermal_arm_as_a_gap() {
    for what in [ProcessReading::ThermalState, ProcessReading::ThermalStateWatch] {
        assert_eq!(
            unavailable_by_default(what, HostOs::MacOs),
            ProcessInfoUnavailable::BranchNotWritten {
                what,
                host: HostOs::MacOs
            }
        );
        for host in [HostOs::Linux, HostOs::Windows] {
            assert_eq!(
                unavailable_by_default(what, host),
                ProcessInfoUnavailable::NoThermalSignal(host)
            );
        }
    }
}

#[test]
fn memory_and_uptime_gaps_are_named_on_every_host() {
    for host in HostOs::ALL {
        for what in [ProcessReading::PhysicalMemory, ProcessReading::SystemUptime] {
            assert_eq!(
                unavailable_by_default(what, host),
                ProcessInfoUnavailable::BranchNotWritten { what, host }
            );
        }
    }
}

#[test]
fn every_reason_reads_as_a_sentence() {
    let reason = ProcessInfoUnavailable::BranchNotWritten {
        what: ProcessReading::PhysicalMemory,
        host: HostOs::Linux,
    };
    assert_eq!(reason.to_string(), "physical memory is not written for Linux");
    assert_eq!(
        ProcessInfoUnavailable::NoThermalSignal(HostOs::Windows).to_string(),
        "Windows publishes no thermal state to applications"
    );
}

#[test]
fn megabytes_truncate_like_the_ios_division() {
    assert_eq!(physical_memory_mb(17_179_869_184), 16_384);
    assert_eq!(physical_memory_mb(1_048_575), 0);
}

#[test]
fn uptime_accepts_a_finite_interval_and_refuses_the_rest() {
    assert_eq!(uptime_from_seconds(1.5), Ok(Duration::from_millis(1500)));
    for bad in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            uptime_from_seconds(bad),
            Err(ProcessInfoUnavailable::SystemRefused {
                what: ProcessReading::SystemUptime,
                ..
            })
        ));
    }
}

#[test]
fn the_processor_count_is_never_zero() {
    assert!(active_processor_count() >= 1);
}

#[test]
fn a_later_duplicate_environment_key_wins() {
    let environment = environment_from([("SAI_API_URL", "a"), ("HOME", "/h"), ("SAI_API_URL", "b")]);
    assert_eq!(environment.get("SAI_API_URL").map(String::as_str), Some("b"));
    assert_eq!(environment.len(), 2);
}

#[test]
fn the_launch_arguments_start_with_the_executable() {
    assert!(!arguments().is_empty(), "argv[0] is always present");
}


#[test]
fn meminfo_mem_total_is_read_in_kibibytes() {
    let meminfo = "MemTotal:       16314628 kB\nMemFree:         1234567 kB\n";
    assert_eq!(physical_memory_from_meminfo(meminfo), Ok(16_314_628 * 1024));
    let later = "MemFree:  1 kB\nMemTotal: 2048 kB\n";
    assert_eq!(physical_memory_from_meminfo(later), Ok(2048 * 1024));
}

#[test]
fn meminfo_without_a_usable_mem_total_is_refused() {
    for bad in [
        "",
        "MemFree: 1 kB\n",
        "MemTotal: lots kB\n",
        "MemTotal: 0 kB\n",
        "MemTotal: 12 MB\n",
        "MemTotal: 18446744073709551615 kB\n",
    ] {
        assert!(
            matches!(
                physical_memory_from_meminfo(bad),
                Err(ProcessInfoUnavailable::SystemRefused {
                    what: ProcessReading::PhysicalMemory,
                    ..
                })
            ),
            "{bad:?} must be refused"
        );
    }
}

#[test]
fn proc_uptime_first_field_is_the_uptime() {
    assert_eq!(
        system_uptime_from_proc_uptime("350735.47 234388.90\n"),
        Ok(Duration::from_secs_f64(350735.47))
    );
    for bad in ["", "abc 1.0", "-5.0 1.0"] {
        assert!(matches!(
            system_uptime_from_proc_uptime(bad),
            Err(ProcessInfoUnavailable::SystemRefused {
                what: ProcessReading::SystemUptime,
                ..
            })
        ));
    }
}

#[test]
fn tracer_pid_zero_means_no_debugger() {
    let status = "Name:\triseonly\nState:\tS (sleeping)\nTracerPid:\t0\nUid:\t1000\n";
    assert_eq!(debugger_attached_from_proc_status(status), Ok(false));
}

#[test]
fn a_tracer_pid_means_a_debugger() {
    let status = "Name:\triseonly\nTracerPid:\t4242\n";
    assert_eq!(debugger_attached_from_proc_status(status), Ok(true));
}

#[test]
fn status_without_a_usable_tracer_pid_is_refused() {
    for status in ["Name:\triseonly\n", "TracerPid:\t\n", "TracerPid:\tgdb\n"] {
        assert!(
            matches!(
                debugger_attached_from_proc_status(status),
                Err(ProcessInfoUnavailable::SystemRefused {
                    what: ProcessReading::DebuggerAttached,
                    ..
                })
            ),
            "{status:?}"
        );
    }
}

#[test]
fn the_debugger_check_answers_without_panicking() {
    let _ = is_debugger_attached();
}

#[test]
fn cpu_seconds_from_timevals_adds_user_and_system() {
    assert_eq!(cpu_seconds_from_timevals(0, 0, 0, 0), 0.0);
    assert_eq!(cpu_seconds_from_timevals(2, 250_000, 1, 500_000), 3.75);
    assert_eq!(cpu_seconds_from_timevals(1, 750_000, 0, 750_000), 2.5);
}

#[test]
fn cpu_seconds_from_filetimes_counts_100_nanosecond_ticks() {
    assert_eq!(cpu_seconds_from_filetimes(0, 0, 0, 0), 0.0);
    assert_eq!(cpu_seconds_from_filetimes(0, 10_000_000, 0, 5_000_000), 1.5);
    assert_eq!(cpu_seconds_from_filetimes(1, 0, 0, 0), 429.496_729_6);
}

#[test]
fn a_missing_cpu_reading_names_the_host_and_the_reading() {
    for host in [HostOs::MacOs, HostOs::Linux, HostOs::Windows] {
        assert_eq!(
            unavailable_by_default(ProcessReading::ProcessCpuSeconds, host),
            ProcessInfoUnavailable::BranchNotWritten {
                what: ProcessReading::ProcessCpuSeconds,
                host,
            }
        );
    }
}
