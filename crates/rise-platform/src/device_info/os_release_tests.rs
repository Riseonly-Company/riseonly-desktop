use super::device_info::*;
use super::os_release::*;
use crate::host_os::HostOs;

const UBUNTU: &str = "PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n\
                      NAME=\"Ubuntu\"\n\
                      VERSION_ID=\"24.04\"\n\
                      VERSION=\"24.04.1 LTS (Noble Numbat)\"\n\
                      ID=ubuntu\n";

const ARCH: &str = "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\nBUILD_ID=rolling\n";

#[test]
fn ubuntu_names_its_release() {
    assert_eq!(
        system_version_from_os_release(UBUNTU),
        SystemVersion {
            name: Some("Ubuntu".to_owned()),
            version: "24.04".to_owned()
        }
    );
}

#[test]
fn a_rolling_distribution_answers_its_build_id() {
    let system = system_version_from_os_release(ARCH);
    assert_eq!(system.name.as_deref(), Some("Arch Linux"));
    assert_eq!(system.version, "rolling");
}

#[test]
fn no_version_reads_as_unknown_through_the_hub() {
    let info = DeviceInfo::resolve(
        HostOs::Linux,
        Ok(HardwareModel::default()),
        Ok(system_version_from_os_release("ID=custom\n")),
    );
    assert_eq!(info.os_name, "Linux");
    assert_eq!(info.system_version, UNKNOWN_VERSION);
}

#[test]
fn quotes_comments_and_escapes_follow_the_specification() {
    let text = "# NAME=\"Commented\"\n\
                \n\
                NAME='Single \\n kept'\n\
                VERSION_ID=\"1.0 \\\"beta\\\" \\$HOME\"\n\
                ID=plain\\ value\n\
                BROKEN LINE\n";
    assert_eq!(os_release_value(text, "NAME").as_deref(), Some("Single \\n kept"));
    assert_eq!(os_release_value(text, "VERSION_ID").as_deref(), Some("1.0 \"beta\" $HOME"));
    assert_eq!(os_release_value(text, "ID").as_deref(), Some("plain value"));
    assert_eq!(os_release_value(text, "MISSING"), None);
}

#[test]
fn the_last_assignment_wins_and_crlf_is_dropped() {
    assert_eq!(os_release_value("NAME=One\r\nNAME=\"Two\"\r\n", "NAME").as_deref(), Some("Two"));
}

#[test]
fn a_lenovo_shows_its_product_version() {
    let model = hardware_model_from_files(LinuxHardwareFiles {
        sys_vendor: Some("LENOVO\n".to_owned()),
        product_name: Some("20XW0055GE\n".to_owned()),
        product_version: Some("ThinkPad X1 Carbon Gen 9\n".to_owned()),
        devicetree_model: None,
    })
    .expect("a model");
    assert_eq!(display_name(HostOs::Linux, &model), "LENOVO ThinkPad X1 Carbon Gen 9");
    assert_eq!(model.identifier.as_deref(), Some("20XW0055GE\n"));
}

#[test]
fn other_vendors_ignore_product_version() {
    let model = hardware_model_from_files(LinuxHardwareFiles {
        sys_vendor: Some("Dell Inc.\n".to_owned()),
        product_name: Some("XPS 13 9310\n".to_owned()),
        product_version: Some("Not Specified\n".to_owned()),
        devicetree_model: None,
    })
    .expect("a model");
    assert_eq!(model.name, None);
    assert_eq!(display_name(HostOs::Linux, &model), "Dell Inc. XPS 13 9310");
}

#[test]
fn a_board_without_dmi_uses_the_devicetree_model() {
    let model = hardware_model_from_files(LinuxHardwareFiles {
        devicetree_model: Some("Raspberry Pi 5 Model B Rev 1.0\u{0}".to_owned()),
        ..LinuxHardwareFiles::default()
    })
    .expect("a model");
    assert_eq!(display_name(HostOs::Linux, &model), "Raspberry Pi 5 Model B Rev 1.0");
}

#[test]
fn nothing_read_is_no_model() {
    assert_eq!(hardware_model_from_files(LinuxHardwareFiles::default()), None);
}

#[test]
fn linux_details_name_the_distro_session_and_packaging() {
    let variables = |name: &str| match name {
        "XDG_SESSION_TYPE" => Some("wayland".to_owned()),
        "XDG_CURRENT_DESKTOP" => Some("ubuntu:GNOME".to_owned()),
        "APPIMAGE" => Some("/home/u/Riseonly.AppImage".to_owned()),
        _ => None,
    };
    let details = linux_os_details(Some(UBUNTU), Some("6.8.0-45-generic\n"), variables, false);
    assert_eq!(details.name.as_deref(), Some("Ubuntu 24.04.1 LTS"));
    assert_eq!(details.kernel.as_deref(), Some("6.8.0-45-generic"));
    assert_eq!(details.session.as_deref(), Some("Wayland"));
    assert_eq!(details.desktop.as_deref(), Some("ubuntu:GNOME"));
    assert_eq!(details.packaging.as_deref(), Some("AppImage"));
}

#[test]
fn linux_details_fall_back_to_display_variables_and_flatpak() {
    let variables = |name: &str| (name == "DISPLAY").then(|| ":0".to_owned());
    let details = linux_os_details(None, None, variables, true);
    assert_eq!(details.name, None);
    assert_eq!(details.session.as_deref(), Some("X11"));
    assert_eq!(details.packaging.as_deref(), Some("Flatpak"));
}

#[test]
fn reported_linux_os_carries_kernel_and_environment() {
    let device = DeviceInfo::resolve(HostOs::Linux, Err(DeviceInfoUnavailable::BranchNotWritten {
        what: DeviceReading::HardwareModel,
        host: HostOs::Linux,
    }), Ok(system_version_from_os_release(ARCH)));
    let details = OsDetails {
        kernel: Some("6.11.5-arch1-1".to_owned()),
        session: Some("Wayland".to_owned()),
        desktop: Some("KDE".to_owned()),
        ..OsDetails::default()
    };
    let reported = ReportedOs::resolve(HostOs::Linux, &device, &details, "x86_64");
    assert_eq!(reported.name, "Arch Linux rolling");
    assert_eq!(reported.build.as_deref(), Some("Linux 6.11.5-arch1-1"));
    assert_eq!(reported.arch, "x86_64");
    assert_eq!(reported.environment.as_deref(), Some("Wayland \u{00B7} KDE"));
}
