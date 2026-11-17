use crate::device_info::*;
use crate::host_os::HostOs;

fn model(identifier: Option<&str>, manufacturer: Option<&str>, name: Option<&str>) -> HardwareModel {
    HardwareModel {
        identifier: identifier.map(str::to_owned),
        manufacturer: manufacturer.map(str::to_owned),
        name: name.map(str::to_owned),
    }
}

fn version(name: Option<&str>, version: &str) -> SystemVersion {
    SystemVersion {
        name: name.map(str::to_owned),
        version: version.to_owned(),
    }
}

#[test]
fn mac_identifiers_name_their_family() {
    let cases = [
        ("MacBookPro18,3", "MacBook Pro"),
        ("MacBookAir10,1", "MacBook Air"),
        ("MacBook10,1", "MacBook"),
        ("iMacPro1,1", "iMac Pro"),
        ("iMac21,1", "iMac"),
        ("Macmini9,1", "Mac mini"),
        ("MacPro7,1", "Mac Pro"),
        ("Mac14,2", "Mac"),
        ("VirtualMac2,1", "Mac"),
        ("", "Mac"),
    ];
    for (identifier, family) in cases {
        assert_eq!(mac_family(identifier), family, "{identifier}");
    }
}

#[test]
fn mac_identifiers_name_their_model_like_the_ios_model_map() {
    let cases = [
        ("Mac14,2", "MacBook Air (M2, 2022)"),
        ("Mac15,3", "MacBook Pro (14-inch, M3, Nov 2023)"),
        ("Mac16,10", "Mac mini (2024)"),
        ("Mac13,2", "Mac Studio (2022)"),
        ("MacBookAir10,1", "MacBook Air (M1, 2020)"),
        ("iMac21,1", "iMac (24-inch, M1, 2021)"),
    ];
    for (identifier, name) in cases {
        assert_eq!(mac_marketing_name(identifier), name, "{identifier}");
    }
}

#[test]
fn a_mac_missing_from_the_table_falls_back_to_its_family() {
    assert_eq!(mac_marketing_name("MacBookPro11,1"), "MacBook Pro");
    assert_eq!(mac_marketing_name("Mac99,1"), "Mac");
    assert_eq!(mac_marketing_name("VirtualMac2,1"), "Mac");
    assert_eq!(display_name(HostOs::MacOs, &model(Some("Mac99,1"), None, None)), "Apple Mac");
}

#[test]
fn the_mac_model_table_has_no_duplicate_identifiers() {
    let mut seen = std::collections::HashSet::new();
    for (identifier, _) in MAC_MODEL_MAP {
        assert!(seen.insert(*identifier), "{identifier} twice");
    }
}

#[test]
fn a_mac_reads_as_apple_hardware_like_ios_display_name() {
    let name = display_name(HostOs::MacOs, &model(Some("MacBookPro18,3"), None, None));
    assert_eq!(name, "Apple MacBook Pro (14-inch, 2021)");
    assert_eq!(display_name(HostOs::MacOs, &HardwareModel::default()), "Apple Mac");
    assert_eq!(with_apple_prefix("Apple Vision"), "Apple Vision");
    assert_eq!(with_apple_prefix("apple tv"), "apple tv");
}

#[test]
fn placeholders_and_padding_never_reach_the_server() {
    assert_eq!(clean_firmware_text("  To Be Filled By O.E.M.\n"), None);
    assert_eq!(clean_firmware_text("System Product Name"), None);
    assert_eq!(clean_firmware_text("Default string"), None);
    assert_eq!(clean_firmware_text("\0\0"), None);
    assert_eq!(clean_firmware_text(""), None);
    assert_eq!(
        clean_firmware_text(" ThinkPad X1 Carbon Gen 9\0"),
        Some("ThinkPad X1 Carbon Gen 9".to_owned())
    );
}

#[test]
fn a_pc_joins_maker_and_model_without_repeating_the_maker() {
    let joined = model(Some("XPS 13 9310"), Some("Dell Inc."), None);
    assert_eq!(display_name(HostOs::Linux, &joined), "Dell Inc. XPS 13 9310");

    let repeated = model(Some("HP EliteBook 840 G8"), Some("HP"), None);
    assert_eq!(display_name(HostOs::Windows, &repeated), "HP EliteBook 840 G8");

    let lenovo = model(Some("20XW004GUS"), Some("LENOVO"), Some("ThinkPad X1 Carbon Gen 9"));
    assert_eq!(
        display_name(HostOs::Linux, &lenovo),
        "LENOVO ThinkPad X1 Carbon Gen 9",
        "the human model name wins over the machine type"
    );
}

#[test]
fn a_pc_with_blank_firmware_still_has_a_name() {
    let blank = model(Some("System Product Name"), Some("System manufacturer"), None);
    assert_eq!(display_name(HostOs::Linux, &blank), "Linux PC");
    assert_eq!(display_name(HostOs::Windows, &blank), "Windows PC");

    let maker_only = model(None, Some("ASUS"), None);
    assert_eq!(display_name(HostOs::Windows, &maker_only), "ASUS Windows PC");
}

#[test]
fn versions_follow_the_ios_spelling() {
    assert_eq!(dotted_version(15, 1, 0), "15.1");
    assert_eq!(dotted_version(15, 1, 2), "15.1.2");
    assert_eq!(dotted_version(26, 0, 0), "26.0");
    assert_eq!(dotted_version(10, 0, 22631), "10.0.22631");
}

#[test]
fn a_full_answer_keeps_no_reasons() {
    let info = DeviceInfo::resolve(
        HostOs::MacOs,
        Ok(model(Some("Mac15,3"), None, None)),
        Ok(version(None, "15.1")),
    );
    assert_eq!(info.machine_identifier.as_deref(), Some("Mac15,3"));
    assert_eq!(info.display_name, "Apple MacBook Pro (14-inch, M3, Nov 2023)");
    assert_eq!(info.os_name, "macOS");
    assert_eq!(info.system_version, "15.1");
    assert_eq!(info.system_label(), "macOS 15.1");
    assert!(info.unavailable.is_empty());
}

#[test]
fn a_distribution_name_replaces_the_host_name() {
    let info = DeviceInfo::resolve(
        HostOs::Linux,
        Ok(HardwareModel::default()),
        Ok(version(Some("Ubuntu"), "24.04")),
    );
    assert_eq!(info.system_label(), "Ubuntu 24.04");

    let unnamed = DeviceInfo::resolve(
        HostOs::Linux,
        Ok(HardwareModel::default()),
        Ok(version(Some("  "), "")),
    );
    assert_eq!(unnamed.os_name, "Linux");
    assert_eq!(unnamed.system_version, UNKNOWN_VERSION);
}

#[test]
fn every_host_answers_even_when_both_readings_fail() {
    for host in HostOs::ALL {
        let model_gap = DeviceInfoUnavailable::BranchNotWritten {
            what: DeviceReading::HardwareModel,
            host,
        };
        let version_gap = DeviceInfoUnavailable::SystemRefused {
            what: DeviceReading::SystemVersion,
            detail: "test".to_owned(),
        };
        let info = DeviceInfo::resolve(host, Err(model_gap.clone()), Err(version_gap.clone()));
        assert!(!info.display_name.is_empty(), "{host:?}");
        assert_eq!(info.os_name, os_name(host));
        assert_eq!(info.system_version, UNKNOWN_VERSION);
        assert_eq!(info.machine_identifier, None);
        assert_eq!(info.unavailable, vec![model_gap, version_gap]);
    }
}

#[test]
fn this_machine_answers_both_readings_on_macos() {
    let info = DeviceInfo::current();
    assert!(!info.display_name.is_empty());
    if HostOs::current() == HostOs::MacOs {
        assert!(info.unavailable.is_empty(), "{:?}", info.unavailable);
        assert!(info.display_name.starts_with("Apple "));
        assert!(info.machine_identifier.is_some());
        assert_ne!(info.system_version, UNKNOWN_VERSION);
    }
}
