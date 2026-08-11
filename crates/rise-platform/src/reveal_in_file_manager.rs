//! "Show in Finder", "Show in Explorer", "Show in File Manager", and handing a
//! path to whatever program the system opens it with.
//!
//! The binding is two gpui calls; everything above it is the policy that keeps
//! them from misbehaving:
//!
//! - A path that is not there must never reach `reveal_path`: gpui's fallbacks
//!   silently open the home or parent directory instead.
//! - Revealing a directory is not revealing a file. `reveal_path` selects its
//!   argument *inside its parent*.
//! - `open_with_system` pointed at a downloaded binary starts that binary, so a
//!   caller has to say out loud that it means to.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RevealError {
    #[error("{} is relative and means nothing to the file manager", .0.display())]
    NotAbsolute(PathBuf),
    #[error("nothing exists at {}", .0.display())]
    Missing(PathBuf),
    #[error("could not read {}: {}", .path.display(), .reason)]
    Unreadable { path: PathBuf, reason: String },
    #[error("{} starts a program when opened", .0.display())]
    WouldExecute(PathBuf),
}

/// What the filesystem says about a path, as a value, so the policy below can
/// be tested for all three hosts without touching a disk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathKind {
    Missing,
    Directory,
    File { executable_bit: bool },
}

impl PathKind {
    fn has_execute_bit(self) -> bool {
        match self {
            PathKind::File { executable_bit } => executable_bit,
            _ => false,
        }
    }
}

/// Which of the two gpui calls a reveal turns into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RevealAction {
    /// `open_with_system`, so the file manager shows the directory's contents.
    OpenDirectory,
    /// `reveal_path`, which opens the containing directory and highlights the
    /// entry inside it.
    SelectInParent,
}

/// Whether handing a path to the system's default handler starts a program.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpenRisk {
    Inert,
    Executes,
}

/// A caller cannot open something that runs without passing `Granted`. That
/// does not make the operation safe — it makes it deliberate.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ExecutableConsent {
    #[default]
    Refuse,
    /// The user was shown what is about to run and agreed to it.
    Granted,
}

/// A user-visible string this crate owns: the catalogue key, plus the English
/// the app falls back to until `rise-i18n` carries that key. `desktop_`
/// namespaces these away from the keys inherited from the iOS catalogue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UiText {
    pub key: &'static str,
    pub english: &'static str,
}

/// Each host gets its own key rather than one key with a substituted name: the
/// three strings name three different products, and languages that inflect
/// around the name cannot decline the substitution.
pub const fn reveal_label(host: HostOs) -> UiText {
    match host {
        HostOs::MacOs => UiText {
            key: "desktop_reveal_in_finder",
            english: "Show in Finder",
        },
        HostOs::Windows => UiText {
            key: "desktop_reveal_in_explorer",
            english: "Show in Explorer",
        },
        HostOs::Linux => UiText {
            key: "desktop_reveal_in_file_manager",
            english: "Show in File Manager",
        },
    }
}

/// Shown before `ExecutableConsent::Granted` may be passed.
pub const EXECUTABLE_WARNING: UiText = UiText {
    key: "desktop_open_executable_warning",
    english: "This file runs a program when opened. Open it anyway?",
};

/// Whether the path is anchored to a root *on `host`*. `Path::is_absolute`
/// answers for the platform this binary was compiled for, so on a Mac it calls
/// `C:\Users\u\a.txt` relative.
pub fn is_rooted(host: HostOs, path: &Path) -> bool {
    let text = path.to_string_lossy();

    match host {
        HostOs::Windows => {
            if text.starts_with("\\\\") || text.starts_with("//") {
                return true;
            }
            let bytes = text.as_bytes();
            bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && (bytes[2] == b'\\' || bytes[2] == b'/')
        }
        HostOs::MacOs | HostOs::Linux => text.starts_with('/'),
    }
}

/// Whether the path's extension is one the host starts rather than displays.
/// Extensions only: this is the half of the question answerable from a name
/// alone, which is what a menu needs before anything has been stat'd.
pub fn extension_executes(host: HostOs, path: &Path) -> bool {
    let text = path.to_string_lossy();
    let Some(extension) = extension_of(host, &text) else {
        return false;
    };

    executing_extensions(host)
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(extension))
}

/// The whole question, once the path has been stat'd. An extensionless file
/// carrying the execute bit is a program; the bit is ignored when there *is* an
/// extension, because the OS then picks a handler by type. Windows has no bit.
pub fn opening_risk(host: HostOs, path: &Path, kind: PathKind) -> OpenRisk {
    if extension_executes(host, path) {
        return OpenRisk::Executes;
    }

    let text = path.to_string_lossy();
    let bit_is_meaningful = matches!(host, HostOs::MacOs | HostOs::Linux);
    let unidentified = extension_of(host, &text).is_none();

    if bit_is_meaningful && unidentified && kind.has_execute_bit() {
        OpenRisk::Executes
    } else {
        OpenRisk::Inert
    }
}

/// Which of the two calls a reveal turns into. A macOS application bundle is a
/// directory that `open_with_system` would launch, so a directory whose
/// extension executes is selected in its parent like a file instead.
pub fn plan_reveal(host: HostOs, path: &Path, kind: PathKind) -> Result<RevealAction, RevealError> {
    if !is_rooted(host, path) {
        return Err(RevealError::NotAbsolute(path.to_path_buf()));
    }

    match kind {
        PathKind::Missing => Err(RevealError::Missing(path.to_path_buf())),
        PathKind::Directory if !extension_executes(host, path) => Ok(RevealAction::OpenDirectory),
        _ => Ok(RevealAction::SelectInParent),
    }
}

/// Returns the risk the caller accepted, so a launch can be logged as a
/// decision that was taken rather than inferred from something having started.
pub fn plan_open(
    host: HostOs,
    path: &Path,
    kind: PathKind,
    consent: ExecutableConsent,
) -> Result<OpenRisk, RevealError> {
    if !is_rooted(host, path) {
        return Err(RevealError::NotAbsolute(path.to_path_buf()));
    }
    if kind == PathKind::Missing {
        return Err(RevealError::Missing(path.to_path_buf()));
    }

    let risk = opening_risk(host, path, kind);
    match (risk, consent) {
        (OpenRisk::Executes, ExecutableConsent::Refuse) => {
            Err(RevealError::WouldExecute(path.to_path_buf()))
        }
        _ => Ok(risk),
    }
}

/// gpui routes `reveal_path` through the Linux display client, whose headless
/// implementation is an empty body. `compositor_name()` is the only thing that
/// distinguishes that session — `""` on macOS and Windows, hence the Linux
/// scope — and only `"headless"` disqualifies; anything else is a real session.
pub fn reveal_support(host: HostOs, action: RevealAction, compositor: &str) -> PlatformSupport {
    match action {
        RevealAction::OpenDirectory => open_support(host),
        RevealAction::SelectInParent => match host {
            HostOs::MacOs | HostOs::Windows => PlatformSupport::Performed,
            HostOs::Linux if compositor.eq_ignore_ascii_case("headless") => {
                PlatformSupport::Unsupported
            }
            HostOs::Linux => PlatformSupport::Performed,
        },
    }
}

/// `open_with_system` sits on the Linux platform itself rather than on its
/// display client, so unlike `reveal_path` it survives a headless session.
pub const fn open_support(_host: HostOs) -> PlatformSupport {
    PlatformSupport::Performed
}

/// The one filesystem read in this file.
pub fn inspect(path: &Path) -> Result<PathKind, RevealError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(PathKind::Directory),
        Ok(metadata) => Ok(PathKind::File {
            executable_bit: executable_bit(&metadata),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // `metadata` follows symlinks, so a broken one lands here; it is
            // still a real entry its parent directory can select.
            match std::fs::symlink_metadata(path) {
                Ok(_) => Ok(PathKind::File {
                    executable_bit: false,
                }),
                Err(_) => Ok(PathKind::Missing),
            }
        }
        Err(error) => Err(RevealError::Unreadable {
            path: path.to_path_buf(),
            reason: error.to_string(),
        }),
    }
}

/// Shows the path in the system file manager. Nothing here ever runs the path,
/// which is why it takes no consent argument — the bundle test looks at what
/// the path resolves to, since a symlink to a bundle stats as a plain directory.
pub fn reveal(app: &gpui::App, path: &Path) -> Result<PlatformSupport, RevealError> {
    let host = HostOs::current();
    let kind = inspect(path)?;
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    let action = match plan_reveal(host, &resolved, kind)? {
        RevealAction::OpenDirectory => plan_reveal(host, path, kind)?,
        selected => selected,
    };

    if !reveal_support(host, action, app.compositor_name()).is_supported() {
        return Ok(PlatformSupport::Unsupported);
    }

    match action {
        RevealAction::SelectInParent => app.reveal_path(path),
        RevealAction::OpenDirectory => app.open_with_system(path),
    }

    Ok(PlatformSupport::Performed)
}

/// Opens the path with whatever the system associates with it. The risk is
/// judged on the resolved path as well as the caller's spelling — a link to
/// `Foo.app` carries no extension — and the stricter reading always wins.
pub fn open_with_system(
    app: &gpui::App,
    path: &Path,
    consent: ExecutableConsent,
) -> Result<PlatformSupport, RevealError> {
    let host = HostOs::current();
    let kind = inspect(path)?;
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    plan_open(host, path, kind, consent)?;
    if resolved != path {
        plan_open(host, &resolved, kind, consent)?;
    }

    if !open_support(host).is_supported() {
        return Ok(PlatformSupport::Unsupported);
    }

    app.open_with_system(path);
    Ok(PlatformSupport::Performed)
}

fn executing_extensions(host: HostOs) -> &'static [&'static str] {
    match host {
        HostOs::MacOs => &["app", "command", "tool", "pkg", "sh", "bash", "zsh"],
        HostOs::Windows => &["exe", "bat", "cmd", "ps1", "scr", "msi", "vbs", "lnk"],
        HostOs::Linux => &["sh", "bash", "zsh", "run", "desktop", "appimage"],
    }
}

/// Split by hand: `Path` would apply this binary's separators instead of
/// `host`'s, and a backslash is an ordinary character in a macOS file name.
fn file_name_of(host: HostOs, path: &str) -> &str {
    let separator = |character: char| {
        character == '/' || (matches!(host, HostOs::Windows) && character == '\\')
    };

    // Trailing separators first, or `/Applications/Foo.app/` loses its
    // extension and an application bundle reads as an ordinary folder.
    let path = path.trim_end_matches(separator);

    match path.rfind(separator) {
        Some(index) => &path[index + 1..],
        None => path,
    }
}

fn extension_of(host: HostOs, path: &str) -> Option<&str> {
    let (stem, extension) = file_name_of(host, path).rsplit_once('.')?;

    // A leading dot marks a hidden file, not an extension: `.bashrc` is not a
    // `bashrc` file, and treating it as one would misclassify `.sh`.
    if stem.is_empty() || extension.is_empty() {
        return None;
    }

    Some(extension)
}

fn executable_bit(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN_FILE: PathKind = PathKind::File {
        executable_bit: false,
    };

    const EXECUTABLE_FILE: PathKind = PathKind::File {
        executable_bit: true,
    };

    /// A directory's only executability signal is its extension, so losing it
    /// to a trailing separator walks a bundle through the consent gate.
    #[test]
    fn a_trailing_separator_does_not_disarm_the_bundle_check() {
        for path in [
            "/Applications/Riseonly.app",
            "/Applications/Riseonly.app/",
            "/Applications/Riseonly.app///",
        ] {
            assert!(
                extension_executes(HostOs::MacOs, Path::new(path)),
                "{path} is an application bundle"
            );
            assert_eq!(
                plan_open(
                    HostOs::MacOs,
                    Path::new(path),
                    PathKind::Directory,
                    ExecutableConsent::Refuse
                ),
                Err(RevealError::WouldExecute(PathBuf::from(path))),
                "{path} must not open without consent"
            );
        }
    }

    #[test]
    fn a_trailing_separator_leaves_an_ordinary_folder_ordinary() {
        assert_eq!(
            plan_reveal(
                HostOs::MacOs,
                Path::new("/Users/someone/Documents/"),
                PathKind::Directory
            ),
            Ok(RevealAction::OpenDirectory)
        );
    }

    #[test]
    fn a_path_that_is_only_separators_has_no_extension() {
        assert!(!extension_executes(HostOs::MacOs, Path::new("/")));
        assert!(!extension_executes(HostOs::MacOs, Path::new("///")));
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rise-reveal-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn directory_on(host: HostOs) -> &'static Path {
        match host {
            HostOs::Windows => Path::new("C:\\Users\\u\\Downloads"),
            _ => Path::new("/home/u/Downloads"),
        }
    }

    fn file_on(host: HostOs) -> &'static Path {
        match host {
            HostOs::Windows => Path::new("C:\\Users\\u\\Downloads\\photo.png"),
            _ => Path::new("/home/u/Downloads/photo.png"),
        }
    }

    #[test]
    fn a_directory_is_opened_and_a_file_is_selected_inside_its_parent() {
        for host in HostOs::ALL {
            assert_eq!(
                plan_reveal(host, directory_on(host), PathKind::Directory),
                Ok(RevealAction::OpenDirectory),
                "{host:?} would show the folder above the one that was asked for"
            );
            assert_eq!(
                plan_reveal(host, file_on(host), PLAIN_FILE),
                Ok(RevealAction::SelectInParent),
                "{host:?} would show the file's contents instead of the file"
            );
        }
    }

    #[test]
    fn a_macos_application_bundle_is_selected_rather_than_launched() {
        let bundle = Path::new("/Applications/Riseonly.app");

        assert_eq!(
            plan_reveal(HostOs::MacOs, bundle, PathKind::Directory),
            Ok(RevealAction::SelectInParent),
            "opening a bundle directory on macOS runs the application"
        );
        assert_eq!(
            plan_reveal(HostOs::Linux, bundle, PathKind::Directory),
            Ok(RevealAction::OpenDirectory),
            "the same directory is an ordinary folder on Linux"
        );
    }

    #[test]
    fn a_missing_path_is_refused_on_every_host() {
        for host in HostOs::ALL {
            let path = directory_on(host).join("gone.zip");

            assert_eq!(
                plan_reveal(host, &path, PathKind::Missing),
                Err(RevealError::Missing(path.clone())),
                "{host:?} would open some other directory instead"
            );
        }
    }

    #[test]
    fn an_empty_path_is_refused_rather_than_opening_the_home_directory() {
        for host in HostOs::ALL {
            assert_eq!(
                plan_reveal(host, Path::new(""), PathKind::Directory),
                Err(RevealError::NotAbsolute(PathBuf::new())),
                "{host:?} accepted a path with no destination"
            );
        }
    }

    #[test]
    fn a_relative_path_is_refused_because_its_parent_is_nothing() {
        let relative = Path::new("gone.zip");

        for host in HostOs::ALL {
            assert_eq!(
                plan_reveal(host, relative, PLAIN_FILE),
                Err(RevealError::NotAbsolute(relative.to_path_buf())),
                "{host:?} resolves this against a directory the user never chose"
            );
        }
    }

    #[test]
    fn each_host_recognises_only_its_own_shape_of_absolute_path() {
        let windows = Path::new("C:\\Users\\u\\a.txt");
        let unc = Path::new("\\\\server\\share\\a.txt");
        let posix = Path::new("/home/u/a.txt");

        assert!(is_rooted(HostOs::Windows, windows));
        assert!(is_rooted(HostOs::Windows, unc));
        assert!(!is_rooted(HostOs::Windows, posix));
        assert!(!is_rooted(HostOs::Windows, Path::new("Users\\u\\a.txt")));

        for host in [HostOs::MacOs, HostOs::Linux] {
            assert!(is_rooted(host, posix), "{host:?}");
            assert!(!is_rooted(host, windows), "{host:?}");
        }
    }

    #[test]
    fn each_host_flags_the_extensions_that_run_on_it() {
        for name in ["Riseonly.app", "install.command", "setup.sh"] {
            let path = Path::new("/Users/u/Downloads").join(name);
            assert!(extension_executes(HostOs::MacOs, &path), "{name}");
        }

        for name in ["setup.exe", "run.bat", "run.cmd", "task.ps1", "saver.scr"] {
            let path = PathBuf::from(format!("C:\\Users\\u\\Downloads\\{name}"));
            assert!(extension_executes(HostOs::Windows, &path), "{name}");
        }

        for name in ["setup.sh", "Riseonly.AppImage", "riseonly.desktop"] {
            let path = Path::new("/home/u/Downloads").join(name);
            assert!(extension_executes(HostOs::Linux, &path), "{name}");
        }
    }

    #[test]
    fn a_windows_executable_is_dangerous_on_windows_and_inert_elsewhere() {
        let path = Path::new("C:\\Users\\u\\Downloads\\setup.exe");

        assert!(extension_executes(HostOs::Windows, path));
        assert!(!extension_executes(HostOs::MacOs, path));
        assert!(!extension_executes(HostOs::Linux, path));
    }

    #[test]
    fn a_linux_launcher_is_dangerous_on_linux_and_inert_elsewhere() {
        let path = Path::new("/home/u/Downloads/riseonly.desktop");

        assert!(extension_executes(HostOs::Linux, path));
        assert!(!extension_executes(HostOs::MacOs, path));
        assert!(!extension_executes(HostOs::Windows, path));
    }

    #[test]
    fn the_extension_check_ignores_letter_case() {
        let installer = Path::new("C:\\Users\\u\\SETUP.EXE");
        let image = Path::new("/home/u/Riseonly.AppImage");
        let bundle = Path::new("/Applications/Riseonly.APP");

        assert!(extension_executes(HostOs::Windows, installer));
        assert!(extension_executes(HostOs::Linux, image));
        assert!(extension_executes(HostOs::MacOs, bundle));
    }

    #[test]
    fn ordinary_documents_are_never_flagged_on_any_host() {
        for host in HostOs::ALL {
            for name in ["photo.png", "notes.txt", "report.pdf", "clip.mp4"] {
                let path = Path::new("/home/u").join(name);
                assert!(!extension_executes(host, &path), "{host:?} flagged {name}");
            }
        }
    }

    #[test]
    fn a_hidden_file_is_not_read_as_having_an_extension() {
        let path = Path::new("/home/u/.bashrc");

        for host in HostOs::ALL {
            assert!(!extension_executes(host, path), "{host:?}");
        }
    }

    #[test]
    fn an_extensionless_program_is_caught_by_the_execute_bit_where_it_exists() {
        let path = Path::new("/home/u/Downloads/installer");

        assert_eq!(
            opening_risk(HostOs::MacOs, path, EXECUTABLE_FILE),
            OpenRisk::Executes
        );
        assert_eq!(
            opening_risk(HostOs::Linux, path, EXECUTABLE_FILE),
            OpenRisk::Executes
        );
        assert_eq!(
            opening_risk(HostOs::Windows, path, EXECUTABLE_FILE),
            OpenRisk::Inert,
            "Windows has no execute bit; only the extension decides there"
        );
    }

    #[test]
    fn a_document_that_merely_carries_the_execute_bit_is_not_a_program() {
        let path = Path::new("/home/u/notes.txt");

        for host in HostOs::ALL {
            assert_eq!(
                opening_risk(host, path, EXECUTABLE_FILE),
                OpenRisk::Inert,
                "{host:?} picks a handler by type when there is an extension"
            );
        }
    }

    #[test]
    fn a_plain_directory_is_not_a_program_but_a_bundle_is() {
        let downloads = Path::new("/Users/u/Downloads");
        let installer = Path::new("/Users/u/Downloads/Installer.app");

        assert_eq!(
            opening_risk(HostOs::MacOs, downloads, PathKind::Directory),
            OpenRisk::Inert
        );
        assert_eq!(
            opening_risk(HostOs::MacOs, installer, PathKind::Directory),
            OpenRisk::Executes
        );
    }

    #[test]
    fn a_downloaded_program_cannot_be_opened_without_an_acknowledgement() {
        let path = Path::new("/Users/u/Downloads/Installer.app");
        let refused = plan_open(
            HostOs::MacOs,
            path,
            PathKind::Directory,
            ExecutableConsent::Refuse,
        );
        let granted = plan_open(
            HostOs::MacOs,
            path,
            PathKind::Directory,
            ExecutableConsent::Granted,
        );

        assert_eq!(refused, Err(RevealError::WouldExecute(path.to_path_buf())));
        assert_eq!(
            granted,
            Ok(OpenRisk::Executes),
            "an acknowledged launch still reports what it was"
        );
    }

    #[test]
    fn consent_defaults_to_refusing() {
        assert_eq!(ExecutableConsent::default(), ExecutableConsent::Refuse);
    }

    #[test]
    fn an_ordinary_document_opens_without_any_acknowledgement() {
        for host in HostOs::ALL {
            let path = directory_on(host).join("photo.png");
            let planned = plan_open(host, &path, PLAIN_FILE, ExecutableConsent::Refuse);

            assert_eq!(planned, Ok(OpenRisk::Inert), "{host:?}");
        }
    }

    #[test]
    fn a_missing_path_is_not_handed_to_the_default_handler() {
        let path = Path::new("/home/u/Downloads/gone.pdf");
        let planned = plan_open(
            HostOs::Linux,
            path,
            PathKind::Missing,
            ExecutableConsent::Granted,
        );

        assert_eq!(planned, Err(RevealError::Missing(path.to_path_buf())));
    }

    #[test]
    fn each_host_names_the_file_manager_it_actually_ships() {
        assert_eq!(reveal_label(HostOs::MacOs).english, "Show in Finder");
        assert_eq!(reveal_label(HostOs::Windows).english, "Show in Explorer");
        assert_eq!(reveal_label(HostOs::Linux).english, "Show in File Manager");
    }

    #[test]
    fn no_two_hosts_share_a_translation_key() {
        for host in HostOs::ALL {
            let key = reveal_label(host).key;
            let sharing = HostOs::ALL
                .iter()
                .filter(|other| reveal_label(**other).key == key)
                .count();

            assert_eq!(sharing, 1, "{key} would be one string for several hosts");
        }
    }

    #[test]
    fn every_key_is_shaped_the_way_the_catalogue_expects() {
        let mut texts: Vec<UiText> = HostOs::ALL.map(reveal_label).to_vec();
        texts.push(EXECUTABLE_WARNING);

        for text in texts {
            assert!(
                text.key.starts_with("desktop_"),
                "{} is not namespaced away from the inherited iOS keys",
                text.key
            );
            assert!(
                text.key
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character == '_'),
                "{} is not snake_case",
                text.key
            );
            assert!(!text.english.is_empty(), "{} has no fallback", text.key);
        }
    }

    #[test]
    fn a_headless_linux_session_cannot_select_a_file_but_can_still_open_one() {
        assert_eq!(
            reveal_support(HostOs::Linux, RevealAction::SelectInParent, "headless"),
            PlatformSupport::Unsupported,
            "gpui's headless client implements reveal_path as an empty body"
        );
        assert_eq!(
            reveal_support(HostOs::Linux, RevealAction::OpenDirectory, "headless"),
            PlatformSupport::Performed,
            "open_with_system does not go through the display client"
        );
    }

    #[test]
    fn a_linux_desktop_session_can_select_a_file() {
        for compositor in ["Wayland", "X11"] {
            assert_eq!(
                reveal_support(HostOs::Linux, RevealAction::SelectInParent, compositor),
                PlatformSupport::Performed,
                "{compositor}"
            );
        }
    }

    #[test]
    fn the_compositor_name_never_disables_macos_or_windows() {
        for host in [HostOs::MacOs, HostOs::Windows] {
            for compositor in ["", "headless"] {
                assert_eq!(
                    reveal_support(host, RevealAction::SelectInParent, compositor),
                    PlatformSupport::Performed,
                    "{host:?} reports an empty compositor name and is never headless"
                );
            }
        }
    }

    #[test]
    fn every_host_can_hand_a_path_to_its_default_handler() {
        for host in HostOs::ALL {
            assert!(open_support(host).is_supported(), "{host:?}");
        }
    }

    #[test]
    fn a_real_directory_and_a_real_file_are_classified_apart() {
        let directory = scratch_dir("kinds");
        let file = directory.join("note.txt");
        std::fs::write(&file, b"x").unwrap();

        assert_eq!(inspect(&directory).unwrap(), PathKind::Directory);
        assert_eq!(inspect(&file).unwrap(), PLAIN_FILE);

        let host = HostOs::current();
        assert_eq!(
            plan_reveal(host, &directory, inspect(&directory).unwrap()),
            Ok(RevealAction::OpenDirectory)
        );
        assert_eq!(
            plan_reveal(host, &file, inspect(&file).unwrap()),
            Ok(RevealAction::SelectInParent)
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn a_deleted_path_is_reported_missing_rather_than_read_as_a_file() {
        let directory = scratch_dir("missing");
        let gone = directory.join("gone.zip");

        assert_eq!(inspect(&gone).unwrap(), PathKind::Missing);
        assert_eq!(
            plan_reveal(HostOs::current(), &gone, inspect(&gone).unwrap()),
            Err(RevealError::Missing(gone.clone()))
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_symlink_can_still_be_shown_in_its_parent() {
        let directory = scratch_dir("dangling");
        let link = directory.join("broken");
        std::os::unix::fs::symlink(directory.join("absent"), &link).unwrap();

        assert_eq!(
            inspect(&link).unwrap(),
            PLAIN_FILE,
            "the link is a real entry even though its target is gone"
        );
        assert_eq!(
            plan_reveal(HostOs::current(), &link, inspect(&link).unwrap()),
            Ok(RevealAction::SelectInParent)
        );

        std::fs::remove_dir_all(&directory).ok();
    }

    #[cfg(unix)]
    #[test]
    fn the_execute_bit_is_read_from_the_file_rather_than_assumed() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = scratch_dir("execbit");
        let program = directory.join("installer");
        std::fs::write(&program, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        let refused = plan_open(
            HostOs::current(),
            &program,
            EXECUTABLE_FILE,
            ExecutableConsent::Refuse,
        );

        assert_eq!(inspect(&program).unwrap(), EXECUTABLE_FILE);
        assert_eq!(
            refused,
            Err(RevealError::WouldExecute(program.clone())),
            "an extensionless downloaded binary must not open by accident"
        );

        std::fs::remove_dir_all(&directory).ok();
    }
}
