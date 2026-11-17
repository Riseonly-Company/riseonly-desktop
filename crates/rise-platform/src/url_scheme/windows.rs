use std::path::{Path, PathBuf};

use gpui::{App, AppContext as _, Task};
use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{
    ASSOCF_INIT_IGNOREUNKNOWN, ASSOCF_IS_PROTOCOL, ASSOCSTR_COMMAND, AssocQueryStringW,
    SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

use super::association_entries::*;
use super::scheme_desktop_entry::is_build_tree_executable;
use super::url_scheme::*;

pub(super) struct NativeUrlScheme;

/// executable the os should start, a binary run from a build tree must not claim anything
pub(crate) fn installed_executable() -> Result<PathBuf, UrlSchemeUnavailable> {
    let executable = std::env::current_exe().map_err(|_| UrlSchemeUnavailable::NotInstalled)?;
    if is_build_tree_executable(&executable) {
        return Err(UrlSchemeUnavailable::NotInstalled);
    }
    Ok(executable)
}

fn text_before_terminator(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

fn association_command(scheme: &str) -> Option<String> {
    let scheme = HSTRING::from(scheme);
    let flags = ASSOCF_IS_PROTOCOL | ASSOCF_INIT_IGNOREUNKNOWN;
    let mut length = 0u32;
    let asked = unsafe {
        AssocQueryStringW(
            flags,
            ASSOCSTR_COMMAND,
            PCWSTR(scheme.as_ptr()),
            PCWSTR::null(),
            None,
            &mut length,
        )
    };
    if asked.is_err() || length <= 1 {
        return None;
    }
    let mut buffer = vec![0u16; length as usize];
    let answered = unsafe {
        AssocQueryStringW(
            flags,
            ASSOCSTR_COMMAND,
            PCWSTR(scheme.as_ptr()),
            PCWSTR::null(),
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )
    };
    answered.is_ok().then(|| text_before_terminator(&buffer))
}

fn write_value(entry: &AssociationValue) -> Result<(), WIN32_ERROR> {
    let subkey = HSTRING::from(entry.subkey.as_str());
    let mut handle = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &raw mut handle,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(status);
    }
    let name = entry.name.as_deref().map(HSTRING::from);
    let name = name.as_ref().map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
    let value: Vec<u8> = entry
        .value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let status = unsafe { RegSetValueExW(handle, name, None, REG_SZ, Some(value.as_slice())) };
    let _ = unsafe { RegCloseKey(handle) };
    if status == ERROR_SUCCESS { Ok(()) } else { Err(status) }
}

/// writes the values and tells the shell, a changed association needs that
pub(crate) fn write_association_values(values: &[AssociationValue]) -> anyhow::Result<()> {
    for entry in values {
        if let Err(status) = write_value(entry) {
            anyhow::bail!("registry error {} at {}", status.0, entry.subkey);
        }
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(())
}

/// default value of a key under HKCU, None when absent or not a string
pub(crate) fn read_default_value(subkey: &str) -> Option<String> {
    let subkey = HSTRING::from(subkey);
    let mut bytes = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &subkey,
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&raw mut bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &subkey,
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut bytes),
        )
    };
    (status == ERROR_SUCCESS).then(|| text_before_terminator(&buffer))
}

fn holds_scheme(scheme: &str, executable: &Path) -> bool {
    association_command(scheme).is_some_and(|command| command_starts_executable(&command, executable))
}

impl UrlSchemeRegistrar for NativeUrlScheme {
    fn is_default_handler(&self, scheme: &str) -> Result<bool, UrlSchemeUnavailable> {
        Ok(holds_scheme(scheme, &installed_executable()?))
    }

    fn register(
        &self,
        scheme: &str,
        cx: &App,
    ) -> Result<Task<anyhow::Result<()>>, UrlSchemeUnavailable> {
        let values = url_scheme_values(scheme, &installed_executable()?);
        Ok(cx.background_spawn(async move { write_association_values(&values) }))
    }
}
