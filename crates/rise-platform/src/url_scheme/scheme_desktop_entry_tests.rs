use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::scheme_desktop_entry::*;
use crate::app_info::IDENTITIES;

const SCHEME: &str = "riseonly";

fn entry_at(path: &str) -> SchemeEntry {
    SchemeEntry::new(&IDENTITIES[2], Path::new(path), SCHEME).expect("a valid scheme")
}

fn entry() -> SchemeEntry {
    entry_at("/opt/rise/riseonly")
}

#[test]
fn the_entry_takes_the_url_and_declares_the_scheme() {
    let text = entry().render();
    assert!(text.starts_with("[Desktop Entry]\n"), "{text}");
    assert!(text.contains("Exec=/opt/rise/riseonly %u\n"), "{text}");
    assert!(text.contains("MimeType=x-scheme-handler/riseonly;\n"), "{text}");
    assert!(text.contains("StartupWMClass=net.riseonly.desktop\n"), "{text}");
    assert_eq!(entry().desktop_id(), "net.riseonly.desktop.desktop");
    assert_eq!(entry().mime_type(), "x-scheme-handler/riseonly");
}

#[test]
fn a_path_with_a_space_is_quoted_and_the_field_code_is_not() {
    let text = entry_at("/opt/my apps/riseonly").render();
    assert!(text.contains("Exec=\"/opt/my apps/riseonly\" %u\n"), "{text}");
}

#[test]
fn a_string_that_is_no_scheme_has_no_entry() {
    assert_eq!(SchemeEntry::new(&IDENTITIES[2], Path::new("/x"), "not a scheme"), None);
}

#[test]
fn a_first_claim_writes_the_entry() {
    assert_eq!(entry().entry_step(None, None), EntryStep::Write);
}

#[test]
fn an_entry_of_ours_is_kept_until_the_binary_moves() {
    let current = entry().render();
    let moved = entry_at("/old/riseonly").render();
    assert_eq!(entry().entry_step(Some(&current), None), EntryStep::Keep);
    assert_eq!(entry().entry_step(Some(&moved), None), EntryStep::Write);
}

#[test]
fn an_entry_the_user_wrote_stays() {
    let own = "[Desktop Entry]\nType=Application\nExec=my-wrapper %u\n";
    assert_eq!(entry().entry_step(Some(own), None), EntryStep::Keep);
}

#[test]
fn a_package_entry_is_shadowed_only_when_it_does_not_declare_the_type() {
    let declares = "[Desktop Entry]\nExec=riseonly %u\nMimeType=x-scheme-handler/riseonly;\n";
    let silent = "[Desktop Entry]\nExec=riseonly %u\n";
    let hidden = "[Desktop Entry]\nHidden=true\nMimeType=x-scheme-handler/riseonly;\n";
    assert_eq!(entry().entry_step(None, Some(declares)), EntryStep::Keep);
    assert_eq!(entry().entry_step(None, Some(silent)), EntryStep::Write);
    assert_eq!(entry().entry_step(None, Some(hidden)), EntryStep::Write);
}

#[test]
fn the_claim_holds_only_as_the_default_and_with_the_current_binary() {
    let id = entry().desktop_id().to_owned();
    let current = entry().render();
    let moved = entry_at("/old/riseonly").render();
    assert!(entry().claim_holds(Some(&id), Some(&current)));
    assert!(entry().claim_holds(Some(&id), None), "a package entry answers");
    assert!(!entry().claim_holds(Some(&id), Some(&moved)));
    assert!(!entry().claim_holds(Some("other.desktop"), Some(&current)));
    assert!(!entry().claim_holds(None, Some(&current)));
}

#[test]
fn an_appimage_is_started_by_its_image_file() {
    let exe = Path::new("/tmp/.mount_rise/usr/bin/riseonly");
    assert_eq!(
        launch_path(Some(OsStr::new("/home/a/Rise.AppImage")), exe),
        PathBuf::from("/home/a/Rise.AppImage")
    );
    assert_eq!(launch_path(Some(OsStr::new("Rise.AppImage")), exe), exe);
    assert_eq!(launch_path(None, exe), exe);
}

#[test]
fn a_cargo_build_tree_is_not_an_install() {
    assert!(is_build_tree_executable(Path::new("/w/target/debug/riseonly")));
    assert!(is_build_tree_executable(Path::new("/w/target/release/riseonly")));
    assert!(is_build_tree_executable(Path::new("/b/target-linux/debug/riseonly")));
    assert!(is_build_tree_executable(Path::new(
        "/w/target/x86_64-unknown-linux-gnu/release/riseonly"
    )));
    assert!(!is_build_tree_executable(Path::new("/usr/bin/riseonly")));
    assert!(!is_build_tree_executable(Path::new("/opt/target/riseonly")));
}
