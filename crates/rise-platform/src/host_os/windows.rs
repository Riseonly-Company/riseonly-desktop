use super::HostOs;

pub(super) const CURRENT: HostOs = HostOs::Windows;

pub(super) fn hide_console(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}
