use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS,
    REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::HSTRING;

use super::autostart::*;
use crate::gpui_shim::PlatformSupport;

fn root_handle(root: RegistryRoot) -> HKEY {
    match root {
        RegistryRoot::CurrentUser => HKEY_CURRENT_USER,
        RegistryRoot::LocalMachine => HKEY_LOCAL_MACHINE,
    }
}

fn refused(status: WIN32_ERROR) -> AutostartError {
    AutostartError::Refused(format!("registry error {}", status.0))
}

struct RunKey(HKEY);

impl RunKey {
    fn open(access: REG_SAM_FLAGS) -> Result<Self, AutostartError> {
        let subkey = HSTRING::from(WINDOWS_RUN_SUBKEY);
        let mut handle = HKEY::default();

        let status = unsafe {
            RegOpenKeyExW(
                root_handle(WINDOWS_RUN_ROOT),
                &subkey,
                None,
                access,
                &raw mut handle,
            )
        };

        if status != ERROR_SUCCESS {
            return Err(refused(status));
        }

        Ok(Self(handle))
    }
}

impl Drop for RunKey {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn wide_bytes(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

pub(super) fn launched_at_login() -> bool {
    launched_at_login_by_argument()
}

pub(super) fn availability(_config: &AutostartConfig) -> PlatformSupport {
    match RunKey::open(KEY_QUERY_VALUE) {
        Ok(_) => PlatformSupport::Performed,
        Err(_) => PlatformSupport::Unsupported,
    }
}

pub(super) fn state(config: &AutostartConfig) -> Result<AutostartState, AutostartError> {
    let key = RunKey::open(KEY_QUERY_VALUE)?;
    let name = HSTRING::from(config.id.as_str());
    let mut size: u32 = 0;

    let status =
        unsafe { RegQueryValueExW(key.0, &name, None, None, None, Some(&raw mut size)) };

    match status {
        ERROR_SUCCESS => Ok(AutostartState::Enabled),
        ERROR_FILE_NOT_FOUND => Ok(AutostartState::Disabled),
        other => Err(refused(other)),
    }
}

pub(super) fn set_enabled(
    config: &AutostartConfig,
    enabled: bool,
) -> Result<PlatformSupport, AutostartError> {
    let action = transition(state(config)?, enabled);
    if action == AutostartTransition::AlreadySettled {
        return Ok(PlatformSupport::Performed);
    }

    let key = RunKey::open(KEY_QUERY_VALUE | KEY_SET_VALUE)?;
    let name = HSTRING::from(config.id.as_str());

    let status = if action == AutostartTransition::Register {
        let value = wide_bytes(&windows_run_command(&config.executable, &config.arguments));
        unsafe { RegSetValueExW(key.0, &name, None, REG_SZ, Some(value.as_slice())) }
    } else {
        unsafe { RegDeleteValueW(key.0, &name) }
    };

    match status {
        ERROR_SUCCESS => Ok(PlatformSupport::Performed),
        other => Err(refused(other)),
    }
}
