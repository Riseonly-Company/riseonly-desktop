//! Launch at login.
//!
//! Whether the row should exist at all (`availability()`) is a different
//! question from whether the switch is on (`state()`). A settings screen that
//! conflates them draws a switch that silently does nothing.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Debug, Error)]
pub enum AutostartError {
    /// The mechanism exists on this OS but this process cannot use it.
    #[error("{0} is not usable in this process")]
    Unavailable(AutostartMechanism),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The OS was asked and said no.
    #[error("the system refused: {0}")]
    Refused(String),
}

/// How one OS is asked to start the app when the user logs in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AutostartMechanism {
    /// `SMAppService.mainAppService`, macOS 13 and later. It registers the
    /// *bundle*, not a path, so it does nothing for a loose binary — see
    /// [`is_bundled_macos_executable`].
    LoginItemService,
    /// A desktop entry under `$XDG_CONFIG_HOME/autostart`, per the freedesktop
    /// Desktop Application Autostart Specification.
    XdgDesktopEntry,
    /// A value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` —
    /// per-user, so writing it never needs elevation.
    CurrentUserRunKey,
}

impl AutostartMechanism {
    pub const fn for_host(host: HostOs) -> Self {
        match host {
            HostOs::MacOs => Self::LoginItemService,
            HostOs::Linux => Self::XdgDesktopEntry,
            HostOs::Windows => Self::CurrentUserRunKey,
        }
    }

    /// True where the OS registers an installed application rather than a path.
    pub const fn needs_application_bundle(self) -> bool {
        matches!(self, Self::LoginItemService)
    }
}

impl std::fmt::Display for AutostartMechanism {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::LoginItemService => "the macOS login-item service (SMAppService)",
            Self::XdgDesktopEntry => "an XDG autostart desktop entry",
            Self::CurrentUserRunKey => "the per-user Windows Run key",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AutostartState {
    Enabled,
    Disabled,
    /// macOS only. Registered, but switched off by the user in System Settings.
    /// Re-registering does not clear it; only the setting can.
    RequiresApproval,
}

impl AutostartState {
    /// Whether the app will actually launch at the next login.
    pub const fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Whether the OS holds a registration at all, approved or not.
    pub const fn is_registered(self) -> bool {
        matches!(self, Self::Enabled | Self::RequiresApproval)
    }
}

/// What a binding has to ask the OS for, given current and desired state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AutostartTransition {
    Register,
    Unregister,
    /// Nothing to ask for. The OS is not called at all.
    AlreadySettled,
}

/// Enabling is always re-applied: it is idempotent, and re-applying repairs an
/// entry still pointing at an executable that has since moved. Disabling
/// something already off is skipped, because `SMAppService` and
/// `RegDeleteValueW` both report that as a failure.
pub const fn transition(current: AutostartState, desired: bool) -> AutostartTransition {
    match (current, desired) {
        (_, true) => AutostartTransition::Register,
        (AutostartState::Disabled, false) => AutostartTransition::AlreadySettled,
        (_, false) => AutostartTransition::Unregister,
    }
}

/// The setting, from the point of view of a settings screen.
pub trait Autostart: Send + Sync {
    /// Whether this build can offer the setting at all.
    fn availability(&self) -> PlatformSupport;

    fn state(&self) -> Result<AutostartState, AutostartError>;

    fn set_enabled(&self, enabled: bool) -> Result<PlatformSupport, AutostartError>;

    fn is_enabled(&self) -> Result<bool, AutostartError> {
        Ok(self.state()?.is_enabled())
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AutostartConfig {
    /// Names the desktop entry file on Linux and the registry value on Windows;
    /// unused on macOS. Sanitised before it becomes a file name; see
    /// [`desktop_file_name`].
    pub id: String,
    /// Shown in the desktop environment's startup list. Never a file name.
    pub display_name: String,
    pub executable: PathBuf,
    /// Passed to the executable at login.
    pub arguments: Vec<String>,
}

impl AutostartConfig {
    pub fn for_current_process(
        id: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Result<Self, AutostartError> {
        Ok(Self {
            id: id.into(),
            display_name: display_name.into(),
            executable: std::env::current_exe()?,
            arguments: Vec::new(),
        })
    }

    pub fn with_arguments(mut self, arguments: Vec<String>) -> Self {
        self.arguments = arguments;
        self
    }
}

/// Reserved by the Desktop Entry Specification inside `Exec`; `\r` is added on
/// top of that list so a bare carriage return cannot be read as whitespace.
const EXEC_RESERVED: &str = " \t\n\r\"'\\><~|&;$*?#()`";

/// Renders the whole entry. Every value goes through [`escape_desktop_value`],
/// so a name carrying a newline cannot introduce a key of its own.
pub fn render_desktop_entry(config: &AutostartConfig) -> String {
    let name = escape_desktop_value(&config.display_name);
    let exec = exec_value(&config.executable, &config.arguments);

    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name={name}\n\
         Exec={exec}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// The `Exec` value: each argument quoted, then the result escaped for the file
/// format — in that order, because a reader undoes the escapes before it splits
/// on quoting. A literal backslash therefore comes out as four characters.
pub fn exec_value(executable: &Path, arguments: &[String]) -> String {
    let mut parts = Vec::with_capacity(arguments.len() + 1);
    parts.push(quote_exec_argument(&executable.to_string_lossy()));

    for argument in arguments {
        parts.push(quote_exec_argument(argument));
    }

    escape_desktop_value(&parts.join(" "))
}

fn quote_exec_argument(argument: &str) -> String {
    let reserved = argument
        .chars()
        .any(|character| EXEC_RESERVED.contains(character));
    let quoted = argument.is_empty() || reserved;
    let mut out = String::with_capacity(argument.len() + 2);

    if quoted {
        out.push('"');
    }

    for character in argument.chars() {
        match character {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(character);
            }
            // A lone percent is a field code (`%f`) the launcher would rewrite.
            '%' => out.push_str("%%"),
            _ => out.push(character),
        }
    }

    if quoted {
        out.push('"');
    }

    out
}

/// The string-escape rule of the file format. Applies to every value, not only
/// `Exec`: it is what keeps a newline in user text from becoming a new key.
pub fn escape_desktop_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(character),
        }
    }

    out
}

/// The autostart directory, per the Base Directory Specification. A relative
/// `$XDG_CONFIG_HOME` is ignored rather than joined, as the specification
/// requires: honouring it would resolve against the launcher's working directory.
pub fn autostart_directory(config_home: Option<&Path>, home: &Path) -> PathBuf {
    match config_home {
        Some(directory) if directory.is_absolute() => directory.join("autostart"),
        _ => home.join(".config").join("autostart"),
    }
}

/// The entry's file name, derived from the application id. The id is sanitised
/// rather than trusted: a `/` or `..` would place the entry outside the
/// autostart directory, and a leading dot would hide it from every startup list.
pub fn desktop_file_name(id: &str) -> String {
    let sanitised: String = id
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => character,
            _ => '-',
        })
        .collect();

    let stem = sanitised.trim_start_matches('.');
    if stem.is_empty() {
        return "riseonly.desktop".to_owned();
    }

    format!("{stem}.desktop")
}

/// Reads the enabled state back out of an entry. Existence of the file is not
/// the answer: a desktop environment switches an entry off by rewriting it with
/// `Hidden=true` or `X-GNOME-Autostart-enabled=false`.
pub fn desktop_entry_is_enabled(contents: &str) -> bool {
    let mut enabled = true;

    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let value = value.trim();
        match key.trim() {
            "Hidden" => enabled &= !value.eq_ignore_ascii_case("true"),
            "X-GNOME-Autostart-enabled" => enabled &= !value.eq_ignore_ascii_case("false"),
            _ => {}
        }
    }

    enabled
}

/// The file half of the Linux arm; the caller supplies the resolved XDG
/// directory.
pub fn read_desktop_entry_state(
    directory: &Path,
    config: &AutostartConfig,
) -> Result<AutostartState, AutostartError> {
    match std::fs::read_to_string(directory.join(desktop_file_name(&config.id))) {
        Ok(contents) if desktop_entry_is_enabled(&contents) => Ok(AutostartState::Enabled),
        Ok(_) => Ok(AutostartState::Disabled),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(AutostartState::Disabled),
        Err(error) => Err(error.into()),
    }
}

pub fn apply_desktop_entry(
    directory: &Path,
    config: &AutostartConfig,
    enabled: bool,
) -> Result<PlatformSupport, AutostartError> {
    let path = directory.join(desktop_file_name(&config.id));

    match transition(read_desktop_entry_state(directory, config)?, enabled) {
        AutostartTransition::Register => {
            std::fs::create_dir_all(directory)?;
            std::fs::write(&path, render_desktop_entry(config))?;
        }
        AutostartTransition::Unregister => match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        },
        AutostartTransition::AlreadySettled => {}
    }

    Ok(PlatformSupport::Performed)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegistryRoot {
    CurrentUser,
    LocalMachine,
}

/// HKCU, never HKLM: the machine-wide key needs elevation and would start the
/// client for every account on the machine.
pub const WINDOWS_RUN_ROOT: RegistryRoot = RegistryRoot::CurrentUser;

pub const WINDOWS_RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// The single string Windows stores in the Run value, quoted for
/// `CommandLineToArgvW` — not for `cmd.exe`, whose rules differ.
pub fn windows_run_command(executable: &Path, arguments: &[String]) -> String {
    let mut parts = Vec::with_capacity(arguments.len() + 1);
    parts.push(quote_windows_argument(&executable.to_string_lossy()));

    for argument in arguments {
        parts.push(quote_windows_argument(argument));
    }

    parts.join(" ")
}

fn quote_windows_argument(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_owned();
    }

    let mut out = String::with_capacity(argument.len() + 2);
    out.push('"');
    let mut pending_backslashes = 0usize;

    for character in argument.chars() {
        match character {
            '\\' => pending_backslashes += 1,
            '"' => {
                push_backslashes(&mut out, pending_backslashes * 2 + 1);
                pending_backslashes = 0;
                out.push('"');
            }
            _ => {
                push_backslashes(&mut out, pending_backslashes);
                pending_backslashes = 0;
                out.push(character);
            }
        }
    }

    // The closing quote is a quote too: a trailing run has to be doubled.
    push_backslashes(&mut out, pending_backslashes * 2);
    out.push('"');
    out
}

fn push_backslashes(out: &mut String, count: usize) {
    out.extend(std::iter::repeat_n('\\', count));
}

/// Whether this executable is the main binary of a real `.app` — the shape of
/// the path is the check: `…/Something.app/Contents/MacOS/binary`.
/// `SMAppService.mainAppService` registers the bundle it is called from, so a
/// loose binary has nothing to register and reports Unsupported.
pub fn is_bundled_macos_executable(executable: &Path) -> bool {
    let mut ancestors = executable
        .components()
        .rev()
        .skip(1)
        .map(|component| component.as_os_str().to_string_lossy().into_owned());

    let macos = ancestors.next().unwrap_or_default();
    let contents = ancestors.next().unwrap_or_default();
    let bundle = ancestors.next().unwrap_or_default();

    macos == "MacOS" && contents == "Contents" && bundle.to_ascii_lowercase().ends_with(".app")
}

/// The real setting for the host this binary was built for.
pub struct SystemAutostart {
    config: AutostartConfig,
}

impl SystemAutostart {
    pub fn new(config: AutostartConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &AutostartConfig {
        &self.config
    }

    pub fn mechanism(&self) -> AutostartMechanism {
        AutostartMechanism::for_host(HostOs::current())
    }
}

impl Autostart for SystemAutostart {
    fn availability(&self) -> PlatformSupport {
        platform::availability(&self.config)
    }

    fn state(&self) -> Result<AutostartState, AutostartError> {
        platform::state(&self.config)
    }

    fn set_enabled(&self, enabled: bool) -> Result<PlatformSupport, AutostartError> {
        platform::set_enabled(&self.config, enabled)
    }
}

/// The test double. It reports Performed like the real ones, and forgets
/// everything at exit — a shipped build wired to it gives the user a switch
/// that resets on every launch.
#[derive(Default)]
pub struct InMemoryAutostart {
    enabled: AtomicBool,
}

impl InMemoryAutostart {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Autostart for InMemoryAutostart {
    fn availability(&self) -> PlatformSupport {
        PlatformSupport::Performed
    }

    fn state(&self) -> Result<AutostartState, AutostartError> {
        Ok(if self.enabled.load(Ordering::Acquire) {
            AutostartState::Enabled
        } else {
            AutostartState::Disabled
        })
    }

    fn set_enabled(&self, enabled: bool) -> Result<PlatformSupport, AutostartError> {
        self.enabled.store(enabled, Ordering::Release);
        Ok(PlatformSupport::Performed)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSError;

    use super::*;

    // Linking the framework is what puts SMAppService into the runtime's class
    // table; without it the lookup fails even where the class exists.
    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    const STATUS_NOT_REGISTERED: isize = 0;
    const STATUS_ENABLED: isize = 1;
    const STATUS_REQUIRES_APPROVAL: isize = 2;
    const STATUS_NOT_FOUND: isize = 3;

    fn unavailable() -> AutostartError {
        AutostartError::Unavailable(AutostartMechanism::for_host(HostOs::current()))
    }

    /// None means no bundle, or a macOS older than 13.
    fn main_app_service(config: &AutostartConfig) -> Option<Retained<AnyObject>> {
        if !is_bundled_macos_executable(&config.executable) {
            return None;
        }

        let class = AnyClass::get(c"SMAppService")?;

        // SAFETY: +mainAppService takes no arguments and returns an autoreleased
        // SMAppService or nil, which is what the Option here is for.
        unsafe { msg_send![class, mainAppService] }
    }

    fn status_of(service: &AnyObject) -> Result<AutostartState, AutostartError> {
        // SAFETY: -status takes no arguments and returns an NSInteger.
        let status: isize = unsafe { msg_send![service, status] };

        match status {
            STATUS_NOT_REGISTERED => Ok(AutostartState::Disabled),
            STATUS_ENABLED => Ok(AutostartState::Enabled),
            STATUS_REQUIRES_APPROVAL => Ok(AutostartState::RequiresApproval),
            STATUS_NOT_FOUND => Err(unavailable()),
            other => Err(AutostartError::Refused(format!(
                "SMAppService reported an unknown status {other}"
            ))),
        }
    }

    pub fn availability(config: &AutostartConfig) -> PlatformSupport {
        match main_app_service(config) {
            Some(_) => PlatformSupport::Performed,
            None => PlatformSupport::Unsupported,
        }
    }

    pub fn state(config: &AutostartConfig) -> Result<AutostartState, AutostartError> {
        let service = main_app_service(config).ok_or_else(unavailable)?;
        status_of(&service)
    }

    pub fn set_enabled(
        config: &AutostartConfig,
        enabled: bool,
    ) -> Result<PlatformSupport, AutostartError> {
        let Some(service) = main_app_service(config) else {
            return Ok(PlatformSupport::Unsupported);
        };

        let action = transition(status_of(&service)?, enabled);
        if action == AutostartTransition::AlreadySettled {
            return Ok(PlatformSupport::Performed);
        }

        // SAFETY: both selectors take a trailing NSError out-parameter and
        // return BOOL; objc2's `_` builds it and yields this Result.
        let outcome: Result<(), Retained<NSError>> = unsafe {
            if action == AutostartTransition::Register {
                msg_send![&*service, registerAndReturnError: _]
            } else {
                msg_send![&*service, unregisterAndReturnError: _]
            }
        };

        match outcome {
            Ok(()) => Ok(PlatformSupport::Performed),
            // Both calls report failure for what is a no-op on some system
            // versions, so the status is the authority.
            Err(error) => {
                if matches!(status_of(&service), Ok(state) if state.is_registered() == enabled) {
                    Ok(PlatformSupport::Performed)
                } else {
                    Err(AutostartError::Refused(error.to_string()))
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;

    fn directory() -> Result<PathBuf, AutostartError> {
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);

        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            AutostartError::Unavailable(AutostartMechanism::for_host(HostOs::current()))
        })?;

        Ok(autostart_directory(config_home.as_deref(), &home))
    }

    pub fn availability(_config: &AutostartConfig) -> PlatformSupport {
        match directory() {
            Ok(_) => PlatformSupport::Performed,
            Err(_) => PlatformSupport::Unsupported,
        }
    }

    pub fn state(config: &AutostartConfig) -> Result<AutostartState, AutostartError> {
        read_desktop_entry_state(&directory()?, config)
    }

    pub fn set_enabled(
        config: &AutostartConfig,
        enabled: bool,
    ) -> Result<PlatformSupport, AutostartError> {
        apply_desktop_entry(&directory()?, config, enabled)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS,
        REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };
    use windows::core::HSTRING;

    use super::*;

    fn root_handle(root: RegistryRoot) -> HKEY {
        match root {
            RegistryRoot::CurrentUser => HKEY_CURRENT_USER,
            RegistryRoot::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }

    fn refused(status: WIN32_ERROR) -> AutostartError {
        AutostartError::Refused(format!("registry error {}", status.0))
    }

    /// Opened, never created: `RegCreateKeyExW` needs the `Win32_Security`
    /// feature this workspace does not enable, and the Run key always exists.
    struct RunKey(HKEY);

    impl RunKey {
        fn open(access: REG_SAM_FLAGS) -> Result<Self, AutostartError> {
            let subkey = HSTRING::from(WINDOWS_RUN_SUBKEY);
            let mut handle = HKEY::default();

            // SAFETY: the out-parameter points at a live HKEY for the call, and
            // the handle it writes is closed exactly once, in Drop.
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
            // SAFETY: the handle came from a successful RegOpenKeyExW and is
            // closed only here.
            let _ = unsafe { RegCloseKey(self.0) };
        }
    }

    /// REG_SZ is NUL-terminated UTF-16, and its length is counted in bytes.
    fn wide_bytes(value: &str) -> Vec<u8> {
        value
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    pub fn availability(_config: &AutostartConfig) -> PlatformSupport {
        match RunKey::open(KEY_QUERY_VALUE) {
            Ok(_) => PlatformSupport::Performed,
            Err(_) => PlatformSupport::Unsupported,
        }
    }

    pub fn state(config: &AutostartConfig) -> Result<AutostartState, AutostartError> {
        let key = RunKey::open(KEY_QUERY_VALUE)?;
        let name = HSTRING::from(config.id.as_str());
        let mut size: u32 = 0;

        // SAFETY: a null data pointer asks only for the size, which probes the
        // value's presence without allocating a buffer.
        let status =
            unsafe { RegQueryValueExW(key.0, &name, None, None, None, Some(&raw mut size)) };

        match status {
            ERROR_SUCCESS => Ok(AutostartState::Enabled),
            ERROR_FILE_NOT_FOUND => Ok(AutostartState::Disabled),
            other => Err(refused(other)),
        }
    }

    pub fn set_enabled(
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
            // SAFETY: the slice outlives the call; its length becomes cbData.
            unsafe { RegSetValueExW(key.0, &name, None, REG_SZ, Some(value.as_slice())) }
        } else {
            // SAFETY: deleting needs nothing beyond the open key and the name.
            unsafe { RegDeleteValueW(key.0, &name) }
        };

        match status {
            ERROR_SUCCESS => Ok(PlatformSupport::Performed),
            other => Err(refused(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AutostartConfig {
        AutostartConfig {
            id: "net.riseonly.desktop".into(),
            display_name: "Riseonly".into(),
            executable: PathBuf::from("/opt/riseonly/riseonly"),
            arguments: vec!["--autostart".into()],
        }
    }

    fn autostart_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rise-autostart-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    /// Reads an `Exec` value the way a launcher must: unescape, then split on
    /// quoting, then collapse `%%`.
    fn exec_arguments(entry: &str) -> Vec<String> {
        let raw = entry
            .lines()
            .find_map(|line| line.strip_prefix("Exec="))
            .expect("the entry has no Exec key");

        let mut unescaped = String::new();
        let mut characters = raw.chars();
        while let Some(character) = characters.next() {
            if character != '\\' {
                unescaped.push(character);
                continue;
            }
            match characters.next() {
                Some('n') => unescaped.push('\n'),
                Some('t') => unescaped.push('\t'),
                Some('r') => unescaped.push('\r'),
                Some(other) => unescaped.push(other),
                None => {}
            }
        }

        let mut arguments = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut started = false;
        let mut characters = unescaped.chars().peekable();

        while let Some(character) = characters.next() {
            match character {
                '"' => {
                    in_quotes = !in_quotes;
                    started = true;
                }
                '\\' if in_quotes => {
                    if let Some(escaped) = characters.next() {
                        current.push(escaped);
                        started = true;
                    }
                }
                '%' if characters.peek() == Some(&'%') => {
                    characters.next();
                    current.push('%');
                    started = true;
                }
                whitespace if whitespace.is_whitespace() && !in_quotes => {
                    if started {
                        arguments.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                other => {
                    current.push(other);
                    started = true;
                }
            }
        }

        if started {
            arguments.push(current);
        }

        arguments
    }

    #[test]
    fn every_host_gets_the_mechanism_that_belongs_to_it() {
        assert_eq!(
            AutostartMechanism::for_host(HostOs::MacOs),
            AutostartMechanism::LoginItemService
        );
        assert_eq!(
            AutostartMechanism::for_host(HostOs::Linux),
            AutostartMechanism::XdgDesktopEntry
        );
        assert_eq!(
            AutostartMechanism::for_host(HostOs::Windows),
            AutostartMechanism::CurrentUserRunKey
        );

        let mut seen: Vec<AutostartMechanism> = Vec::new();
        for host in HostOs::ALL {
            let mechanism = AutostartMechanism::for_host(host);
            assert!(
                !seen.contains(&mechanism),
                "{host:?} reuses another mechanism"
            );
            assert!(!mechanism.to_string().is_empty());
            seen.push(mechanism);
        }
    }

    #[test]
    fn only_macos_registers_an_application_rather_than_a_path() {
        for host in HostOs::ALL {
            assert_eq!(
                AutostartMechanism::for_host(host).needs_application_bundle(),
                host == HostOs::MacOs,
                "{host:?}"
            );
        }
    }

    #[test]
    fn launch_at_login_is_written_per_user_never_machine_wide() {
        assert_eq!(
            WINDOWS_RUN_ROOT,
            RegistryRoot::CurrentUser,
            "HKLM needs elevation and would start the client for every account"
        );
        assert!(WINDOWS_RUN_SUBKEY.ends_with(r"CurrentVersion\Run"));
        assert!(!WINDOWS_RUN_SUBKEY.starts_with('\\'));
    }

    #[test]
    fn enabling_is_re_applied_even_when_it_is_already_on() {
        for current in [
            AutostartState::Enabled,
            AutostartState::Disabled,
            AutostartState::RequiresApproval,
        ] {
            assert_eq!(
                transition(current, true),
                AutostartTransition::Register,
                "{current:?} must still be re-registered so a moved executable is repaired"
            );
        }
    }

    #[test]
    fn disabling_something_that_was_never_enabled_asks_the_os_for_nothing() {
        let cases = [
            (
                AutostartState::Disabled,
                AutostartTransition::AlreadySettled,
            ),
            (AutostartState::Enabled, AutostartTransition::Unregister),
            (
                AutostartState::RequiresApproval,
                AutostartTransition::Unregister,
            ),
        ];

        for (current, expected) in cases {
            assert_eq!(transition(current, false), expected, "{current:?}");
        }
    }

    #[test]
    fn a_login_item_awaiting_approval_is_registered_but_will_not_launch() {
        assert!(AutostartState::RequiresApproval.is_registered());
        assert!(!AutostartState::RequiresApproval.is_enabled());
        assert!(AutostartState::Enabled.is_registered());
        assert!(AutostartState::Enabled.is_enabled());
        assert!(!AutostartState::Disabled.is_registered());
        assert!(!AutostartState::Disabled.is_enabled());
    }

    #[test]
    fn an_executable_path_containing_a_space_survives_the_exec_key() {
        let config = AutostartConfig {
            executable: PathBuf::from("/Applications/Rise Only/riseonly"),
            ..config()
        };

        let arguments = exec_arguments(&render_desktop_entry(&config));
        assert_eq!(arguments[0], "/Applications/Rise Only/riseonly");
        assert_eq!(arguments[1], "--autostart");
    }

    #[test]
    fn a_quote_or_a_backslash_in_a_path_survives_both_escaping_layers() {
        for path in [
            r#"/opt/we"ird/riseonly"#,
            r"/opt/we\ird/riseonly",
            "/opt/it's here/riseonly",
            "/opt/$HOME/riseonly",
            "/opt/50%/riseonly",
            "/opt/a b/c d/riseonly",
        ] {
            let config = AutostartConfig {
                executable: PathBuf::from(path),
                ..config()
            };

            let arguments = exec_arguments(&render_desktop_entry(&config));
            assert_eq!(arguments[0], path, "{path} did not survive the Exec key");
            assert_eq!(arguments.len(), 2, "{path} split into extra arguments");
        }
    }

    #[test]
    fn a_newline_in_the_application_name_cannot_forge_a_second_key() {
        let config = AutostartConfig {
            display_name: "Riseonly\nExec=/bin/sh -c evil".into(),
            ..config()
        };

        let entry = render_desktop_entry(&config);
        assert_eq!(
            entry
                .lines()
                .filter(|line| line.starts_with("Exec="))
                .count(),
            1,
            "a display name is data and must not be able to introduce a key"
        );
        assert_eq!(exec_arguments(&entry)[0], "/opt/riseonly/riseonly");
    }

    #[test]
    fn an_id_containing_a_path_separator_cannot_escape_the_autostart_directory() {
        let directory = Path::new("/home/u/.config/autostart");

        for id in [
            "../../evil",
            "/etc/xdg/autostart/evil",
            "..\\..\\evil",
            ".hidden",
            "",
        ] {
            let path = directory.join(desktop_file_name(id));
            assert_eq!(path.parent(), Some(directory), "{id:?} left the directory");
            assert!(
                path.extension()
                    .is_some_and(|extension| extension == "desktop"),
                "{id:?} produced {path:?}"
            );
            assert!(
                !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.')),
                "{id:?} produced an entry no startup list will show"
            );
        }
    }

    #[test]
    fn a_relative_xdg_config_home_is_ignored_as_the_specification_requires() {
        let home = Path::new("/home/u");
        let fallback = PathBuf::from("/home/u/.config/autostart");

        let explicit = autostart_directory(Some(Path::new("/xdg")), home);
        assert_eq!(explicit, PathBuf::from("/xdg/autostart"));

        let relative = autostart_directory(Some(Path::new("relative/config")), home);
        assert_eq!(
            relative, fallback,
            "a relative XDG_CONFIG_HOME would resolve against the launcher's cwd"
        );

        assert_eq!(autostart_directory(None, home), fallback);
    }

    #[test]
    fn an_entry_switched_off_by_the_desktop_environment_reads_as_disabled() {
        let entry = render_desktop_entry(&config());
        assert!(desktop_entry_is_enabled(&entry));

        assert!(!desktop_entry_is_enabled(&format!("{entry}Hidden=true\n")));
        assert!(!desktop_entry_is_enabled(
            "[Desktop Entry]\nType=Application\nX-GNOME-Autostart-enabled=false\n"
        ));
        assert!(!desktop_entry_is_enabled(
            "[Desktop Entry]\nHidden = TRUE\n"
        ));
    }

    #[test]
    fn the_entry_written_to_disk_reads_back_as_enabled() {
        let directory = autostart_dir("write");
        let config = config();

        assert_eq!(
            apply_desktop_entry(&directory, &config, true).unwrap(),
            PlatformSupport::Performed
        );

        let state = read_desktop_entry_state(&directory, &config).unwrap();
        assert_eq!(state, AutostartState::Enabled);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn enabling_twice_leaves_exactly_one_entry() {
        let directory = autostart_dir("twice");
        let config = config();

        apply_desktop_entry(&directory, &config, true).unwrap();
        apply_desktop_entry(&directory, &config, true).unwrap();

        assert_eq!(
            std::fs::read_dir(&directory).unwrap().count(),
            1,
            "a second enable must overwrite, not accumulate"
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn disabling_something_that_was_never_enabled_is_not_an_error() {
        let directory = autostart_dir("never");
        let config = config();

        assert_eq!(
            apply_desktop_entry(&directory, &config, false).unwrap(),
            PlatformSupport::Performed
        );

        let state = read_desktop_entry_state(&directory, &config).unwrap();
        assert_eq!(state, AutostartState::Disabled);
        assert!(
            !directory.exists(),
            "turning off a switch that was never on must not create anything"
        );
    }

    #[test]
    fn disabling_removes_the_entry_and_reads_back_off() {
        let directory = autostart_dir("remove");
        let config = config();

        apply_desktop_entry(&directory, &config, true).unwrap();
        apply_desktop_entry(&directory, &config, false).unwrap();
        apply_desktop_entry(&directory, &config, false).unwrap();

        let state = read_desktop_entry_state(&directory, &config).unwrap();
        assert_eq!(state, AutostartState::Disabled);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn re_enabling_rewrites_an_entry_left_pointing_at_a_moved_executable() {
        let directory = autostart_dir("moved");
        let stale = AutostartConfig {
            executable: PathBuf::from("/old/riseonly"),
            ..config()
        };
        apply_desktop_entry(&directory, &stale, true).unwrap();

        let moved = AutostartConfig {
            executable: PathBuf::from("/new/riseonly"),
            ..config()
        };
        apply_desktop_entry(&directory, &moved, true).unwrap();

        let path = directory.join(desktop_file_name(&moved.id));
        let contents = std::fs::read_to_string(path).unwrap();
        assert_eq!(exec_arguments(&contents)[0], "/new/riseonly");

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn only_an_executable_inside_a_bundle_can_be_registered_as_a_login_item() {
        assert!(is_bundled_macos_executable(Path::new(
            "/Applications/Riseonly.app/Contents/MacOS/riseonly"
        )));

        for loose in [
            "/Users/u/riseonly-desktop/target/debug/rise-app",
            "/Applications/Riseonly.app/Contents/Resources/riseonly",
            "/Applications/Riseonly/Contents/MacOS/riseonly",
            "/Applications/Riseonly.app/MacOS/riseonly",
            "riseonly",
        ] {
            assert!(
                !is_bundled_macos_executable(Path::new(loose)),
                "{loose} is not a bundled executable"
            );
        }
    }

    #[test]
    fn the_windows_run_value_quotes_a_path_with_spaces() {
        assert_eq!(
            windows_run_command(Path::new(r"C:\Program Files\Riseonly\riseonly.exe"), &[]),
            r#""C:\Program Files\Riseonly\riseonly.exe""#
        );
        assert_eq!(
            windows_run_command(Path::new(r"C:\Riseonly\rise.exe"), &["--autostart".into()]),
            r"C:\Riseonly\rise.exe --autostart"
        );
    }

    #[test]
    fn a_quote_or_a_trailing_backslash_is_escaped_for_command_line_to_argv() {
        assert_eq!(
            windows_run_command(Path::new(r"C:\Program Files\"), &[r#"a"b"#.into()]),
            r#""C:\Program Files\\" "a\"b""#,
            "a run of backslashes before a quote has to be doubled or it escapes it"
        );
    }

    #[test]
    fn the_in_memory_double_remembers_what_it_was_told() {
        let autostart = InMemoryAutostart::new();
        assert!(!autostart.is_enabled().unwrap());

        let first = autostart.set_enabled(true).unwrap();
        let second = autostart.set_enabled(true).unwrap();
        assert_eq!(first, PlatformSupport::Performed);
        assert_eq!(second, PlatformSupport::Performed);
        assert!(autostart.is_enabled().unwrap());

        autostart.set_enabled(false).unwrap();
        autostart.set_enabled(false).unwrap();
        assert!(!autostart.is_enabled().unwrap());
    }

    #[test]
    fn a_test_binary_is_not_a_bundle_so_macos_has_nothing_to_register() {
        let config = AutostartConfig::for_current_process("net.riseonly.test", "Riseonly").unwrap();
        let autostart = SystemAutostart::new(config);
        let expected = AutostartMechanism::for_host(HostOs::current());
        assert_eq!(autostart.mechanism(), expected);

        if HostOs::current() == HostOs::MacOs {
            assert_eq!(
                autostart.availability(),
                PlatformSupport::Unsupported,
                "cargo test runs a loose binary and SMAppService registers bundles"
            );
            let state = autostart.state();
            assert!(matches!(state, Err(AutostartError::Unavailable(_))));
            assert_eq!(
                autostart.set_enabled(true).unwrap(),
                PlatformSupport::Unsupported,
                "an unregisterable build must say so rather than pretend"
            );
        }
    }
}
