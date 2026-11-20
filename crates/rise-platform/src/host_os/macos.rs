use super::HostOs;

pub(super) const CURRENT: HostOs = HostOs::MacOs;

pub(super) fn hide_console(_command: &mut std::process::Command) {}
