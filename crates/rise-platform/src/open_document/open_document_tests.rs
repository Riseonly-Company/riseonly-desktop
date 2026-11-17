use std::path::{Path, PathBuf};

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::open_document::*;
use crate::reveal_in_file_manager::{inspect, ExecutableConsent, PathKind, RevealError};

const DOCUMENT: PathKind = PathKind::File {
    executable_bit: false,
};

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rise-open-document-{name}"));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn document_on(host: HostOs) -> &'static Path {
    match host {
        HostOs::Windows => Path::new("C:\\Users\\u\\Downloads\\report.pdf"),
        _ => Path::new("/home/u/Downloads/report.pdf"),
    }
}

#[test]
fn only_macos_previews_in_place() {
    assert_eq!(route(HostOs::MacOs, DocumentAction::Preview), DocumentRoute::QuickLook);
    assert_eq!(preview_support(HostOs::MacOs), PlatformSupport::Performed);
    for host in [HostOs::Linux, HostOs::Windows] {
        assert_eq!(
            route(host, DocumentAction::Preview),
            DocumentRoute::DefaultApplication,
            "{host:?} has no previewer, so a preview opens the default application"
        );
        assert_eq!(preview_support(host), PlatformSupport::Unsupported, "{host:?}");
    }
}

#[test]
fn open_and_reveal_take_the_same_route_everywhere() {
    for host in HostOs::ALL {
        assert_eq!(route(host, DocumentAction::Open), DocumentRoute::DefaultApplication);
        assert_eq!(route(host, DocumentAction::Reveal), DocumentRoute::FileManager);
    }
}

#[test]
fn every_action_plans_for_an_ordinary_document() {
    for host in HostOs::ALL {
        for action in [DocumentAction::Preview, DocumentAction::Open, DocumentAction::Reveal] {
            assert_eq!(
                plan(host, document_on(host), DOCUMENT, action, ExecutableConsent::Refuse),
                Ok(route(host, action)),
                "{host:?} {action:?}"
            );
        }
    }
}

#[test]
fn a_missing_document_is_refused_by_every_action() {
    for host in HostOs::ALL {
        for action in [DocumentAction::Preview, DocumentAction::Open, DocumentAction::Reveal] {
            assert_eq!(
                plan(
                    host,
                    document_on(host),
                    PathKind::Missing,
                    action,
                    ExecutableConsent::Granted
                ),
                Err(RevealError::Missing(document_on(host).to_path_buf())),
                "{host:?} {action:?}"
            );
        }
    }
}

#[test]
fn a_program_opens_only_with_consent_but_previews_without_it() {
    let installer = Path::new("/Users/u/Downloads/Installer.app");
    let refused = plan(
        HostOs::MacOs,
        installer,
        PathKind::Directory,
        DocumentAction::Open,
        ExecutableConsent::Refuse,
    );
    assert_eq!(refused, Err(RevealError::WouldExecute(installer.to_path_buf())));
    assert_eq!(
        plan(
            HostOs::MacOs,
            installer,
            PathKind::Directory,
            DocumentAction::Open,
            ExecutableConsent::Granted
        ),
        Ok(DocumentRoute::DefaultApplication)
    );
    assert_eq!(
        plan(
            HostOs::MacOs,
            installer,
            PathKind::Directory,
            DocumentAction::Preview,
            ExecutableConsent::Refuse
        ),
        Ok(DocumentRoute::QuickLook),
        "Quick Look shows a bundle without running it"
    );
    let setup = Path::new("C:\\Users\\u\\Downloads\\setup.exe");
    assert_eq!(
        plan(HostOs::Windows, setup, DOCUMENT, DocumentAction::Preview, ExecutableConsent::Refuse),
        Err(RevealError::WouldExecute(setup.to_path_buf())),
        "on Windows a preview is an open, so it needs consent too"
    );
}

#[test]
fn a_relative_path_is_refused() {
    let relative = Path::new("report.pdf");
    for host in HostOs::ALL {
        assert_eq!(
            plan(host, relative, DOCUMENT, DocumentAction::Open, ExecutableConsent::Refuse),
            Err(RevealError::NotAbsolute(relative.to_path_buf())),
            "{host:?}"
        );
    }
}

#[test]
fn a_real_file_with_spaces_and_non_ascii_letters_plans_on_this_host() {
    let dir = scratch_dir("names");
    let file = dir.join("r\u{e9}sum\u{e9} final \u{4e2d}\u{6587}.pdf");
    std::fs::write(&file, b"%PDF").unwrap();
    let host = HostOs::current();
    let kind = inspect(&file).unwrap();

    assert_eq!(kind, DOCUMENT);
    for action in [DocumentAction::Preview, DocumentAction::Open, DocumentAction::Reveal] {
        assert_eq!(
            plan(host, &file, kind, action, ExecutableConsent::Refuse),
            Ok(route(host, action)),
            "{action:?}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_deleted_file_reads_as_missing_before_any_system_call() {
    let dir = scratch_dir("deleted");
    let gone = dir.join("gone file.docx");
    let kind = inspect(&gone).unwrap();

    assert_eq!(kind, PathKind::Missing);
    assert_eq!(
        plan(HostOs::current(), &gone, kind, DocumentAction::Preview, ExecutableConsent::Refuse),
        Err(RevealError::Missing(gone.clone()))
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_portal_call_names_the_documented_interface() {
    assert_eq!(PORTAL_BUS_NAME, "org.freedesktop.portal.Desktop");
    assert_eq!(PORTAL_OBJECT_PATH, "/org/freedesktop/portal/desktop");
    assert_eq!(OPEN_URI_INTERFACE, "org.freedesktop.portal.OpenURI");
    assert_eq!(OPEN_FILE_METHOD, "OpenFile");
    assert!(PARENT_WINDOW_NONE.is_empty());
}
