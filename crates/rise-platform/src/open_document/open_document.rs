use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::reveal_in_file_manager::{self, ExecutableConsent, PathKind, RevealError};

static TESTER_RECEIVER: AtomicBool = AtomicBool::new(false);

pub fn set_tester_receiver(on: bool) {
    TESTER_RECEIVER.store(on, Ordering::Release);
}

pub fn uses_tester_receiver() -> bool {
    TESTER_RECEIVER.load(Ordering::Acquire)
}

fn hand_to_tester(action: DocumentAction, path: &Path) {
    tracing::warn!(target: "riseonly", ?action, path = %path.display(), "X1981 document handed to the tester receiver");
}

/// what the document open asks the system to do with a local file
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DocumentAction {
    Preview,
    Open,
    Reveal,
}

/// the system call an action turns into on a host
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DocumentRoute {
    QuickLook,
    DefaultApplication,
    FileManager,
}

/// true when the host has a previewer the app can show a file in
pub const fn quick_look_on(host: HostOs) -> bool {
    matches!(host, HostOs::MacOs)
}

/// performed where preview shows the file in place, unsupported where it opens the default app instead
pub const fn preview_support(host: HostOs) -> PlatformSupport {
    if quick_look_on(host) {
        PlatformSupport::Performed
    } else {
        PlatformSupport::Unsupported
    }
}

pub const fn route(host: HostOs, action: DocumentAction) -> DocumentRoute {
    match action {
        DocumentAction::Preview if quick_look_on(host) => DocumentRoute::QuickLook,
        DocumentAction::Preview | DocumentAction::Open => DocumentRoute::DefaultApplication,
        DocumentAction::Reveal => DocumentRoute::FileManager,
    }
}

/// checks before any system call, rooted and present and not a program
/// unless the user agreed to run it, a preview never runs a file so it takes Granted
pub fn plan(
    host: HostOs,
    path: &Path,
    kind: PathKind,
    action: DocumentAction,
    consent: ExecutableConsent,
) -> Result<DocumentRoute, RevealError> {
    let chosen = route(host, action);
    match chosen {
        DocumentRoute::QuickLook => {
            reveal_in_file_manager::plan_open(host, path, kind, ExecutableConsent::Granted)?;
        }
        DocumentRoute::DefaultApplication => {
            reveal_in_file_manager::plan_open(host, path, kind, consent)?;
        }
        DocumentRoute::FileManager => {
            reveal_in_file_manager::plan_reveal(host, path, kind)?;
        }
    }
    Ok(chosen)
}

/// opens the file in its default app, a program is refused with WouldExecute
/// the feature shows the warning and calls the consent variant with Granted
pub fn open(app: &gpui::App, path: &Path) -> Result<PlatformSupport, RevealError> {
    open_with_consent(app, path, ExecutableConsent::Refuse)
}

pub fn open_with_consent(
    app: &gpui::App,
    path: &Path,
    consent: ExecutableConsent,
) -> Result<PlatformSupport, RevealError> {
    let host = HostOs::current();
    let kind = reveal_in_file_manager::inspect(path)?;
    plan(host, path, kind, DocumentAction::Open, consent)?;
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if resolved != path {
        plan(host, &resolved, kind, DocumentAction::Open, consent)?;
    }
    if uses_tester_receiver() {
        hand_to_tester(DocumentAction::Open, path);
        return Ok(PlatformSupport::Performed);
    }
    super::os::open_default(app, path);
    Ok(PlatformSupport::Performed)
}

/// shows the file in quick look on mac, elsewhere or if quick look is unreachable opens the default app
/// returns the route taken
pub fn preview(app: &gpui::App, path: &Path) -> Result<DocumentRoute, RevealError> {
    let host = HostOs::current();
    let kind = reveal_in_file_manager::inspect(path)?;
    if plan(host, path, kind, DocumentAction::Preview, ExecutableConsent::Refuse)?
        == DocumentRoute::QuickLook
    {
        if uses_tester_receiver() {
            hand_to_tester(DocumentAction::Preview, path);
            return Ok(DocumentRoute::QuickLook);
        }
        match super::os::quick_look(path) {
            Ok(()) => return Ok(DocumentRoute::QuickLook),
            Err(reason) => {
                tracing::warn!(target: "riseonly", %reason, "quick look unavailable, opening instead");
            }
        }
    }
    open(app, path)?;
    Ok(DocumentRoute::DefaultApplication)
}

/// selects the file in the system file manager
pub fn reveal(app: &gpui::App, path: &Path) -> Result<PlatformSupport, RevealError> {
    if uses_tester_receiver() {
        let kind = reveal_in_file_manager::inspect(path)?;
        reveal_in_file_manager::plan_reveal(HostOs::current(), path, kind)?;
        hand_to_tester(DocumentAction::Reveal, path);
        return Ok(PlatformSupport::Performed);
    }
    reveal_in_file_manager::reveal(app, path)
}

pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const OPEN_URI_INTERFACE: &str = "org.freedesktop.portal.OpenURI";
pub const OPEN_FILE_METHOD: &str = "OpenFile";
pub const PARENT_WINDOW_NONE: &str = "";
pub const XDG_OPEN: &str = "xdg-open";
