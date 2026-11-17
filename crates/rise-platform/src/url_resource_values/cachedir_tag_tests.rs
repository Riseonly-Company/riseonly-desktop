use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::host_os::HostOs;

use super::cachedir_tag::*;
use super::url_resource_values::BackupFlag;

fn item(path: &Path, is_dir: bool, excluded: bool) -> LinuxBackupItem<'_> {
    LinuxBackupItem {
        path,
        is_dir,
        excluded,
        cache_home: Some(Path::new("/home/u/.cache")),
        has_own_tag: false,
        inside_tagged_dir: false,
    }
}

#[test]
fn the_written_tag_carries_the_signature() {
    assert!(is_cachedir_tag(Some(CACHEDIR_TAG_CONTENTS.as_bytes())));
    assert!(is_cachedir_tag(Some(CACHEDIR_TAG_SIGNATURE)));
    assert!(!is_cachedir_tag(Some(b"Signature: 8a477f597d28d172789f06886806bc5")));
    assert!(!is_cachedir_tag(Some(b"signature: 8a477f597d28d172789f06886806bc55")));
    assert!(!is_cachedir_tag(None));
}

#[test]
fn cache_home_follows_the_base_directory_spec() {
    let home = Path::new("/home/u");
    assert_eq!(xdg_cache_home(Some(OsStr::new("/tmp/c")), Some(home)), Some(PathBuf::from("/tmp/c")));
    assert_eq!(xdg_cache_home(Some(OsStr::new("rel")), Some(home)), Some(PathBuf::from("/home/u/.cache")));
    assert_eq!(xdg_cache_home(None, Some(home)), Some(PathBuf::from("/home/u/.cache")));
    assert_eq!(xdg_cache_home(None, Some(Path::new("rel"))), None);
    assert_eq!(xdg_cache_home(None, None), None);
}

#[test]
fn an_excluded_directory_gets_the_tag_once() {
    let dir = Path::new("/home/u/.local/share/Riseonly/drafts");
    assert_eq!(linux_backup_step(&item(dir, true, true)), LinuxBackupStep::WriteTag);
    let tagged = LinuxBackupItem { has_own_tag: true, ..item(dir, true, true) };
    assert_eq!(linux_backup_step(&tagged), LinuxBackupStep::Answer(BackupFlag::Set));
    let cache = Path::new("/home/u/.cache/Riseonly/media");
    assert_eq!(linux_backup_step(&item(cache, true, true)), LinuxBackupStep::WriteTag);
}

#[test]
fn an_excluded_file_is_honoured_only_where_backups_skip_it() {
    let db = Path::new("/home/u/.local/share/Riseonly/store.sqlite");
    assert_eq!(
        linux_backup_step(&item(db, false, true)),
        LinuxBackupStep::Answer(BackupFlag::NoItemFlag(HostOs::Linux))
    );
    assert!(!BackupFlag::NoItemFlag(HostOs::Linux).is_honoured());
    let in_tagged = LinuxBackupItem { inside_tagged_dir: true, ..item(db, false, true) };
    assert_eq!(linux_backup_step(&in_tagged), LinuxBackupStep::Answer(BackupFlag::NotNeeded(HostOs::Linux)));
    let cached = Path::new("/home/u/.cache/Riseonly/voice.m4a");
    assert_eq!(
        linux_backup_step(&item(cached, false, true)),
        LinuxBackupStep::Answer(BackupFlag::NotNeeded(HostOs::Linux))
    );
    let no_home = LinuxBackupItem { cache_home: None, ..item(cached, false, true) };
    assert_eq!(linux_backup_step(&no_home), LinuxBackupStep::Answer(BackupFlag::NoItemFlag(HostOs::Linux)));
}

#[test]
fn including_again_removes_only_the_own_tag() {
    let dir = Path::new("/home/u/.local/share/Riseonly/drafts");
    let tagged = LinuxBackupItem { has_own_tag: true, ..item(dir, true, false) };
    assert_eq!(linux_backup_step(&tagged), LinuxBackupStep::RemoveTag);
    assert_eq!(
        linux_backup_step(&item(dir, true, false)),
        LinuxBackupStep::Answer(BackupFlag::NotNeeded(HostOs::Linux))
    );
    let under_tag = LinuxBackupItem { inside_tagged_dir: true, ..item(dir, true, false) };
    assert_eq!(linux_backup_step(&under_tag), LinuxBackupStep::Answer(BackupFlag::NoItemFlag(HostOs::Linux)));
    let cached = Path::new("/home/u/.cache/Riseonly/media");
    assert_eq!(
        linux_backup_step(&item(cached, true, false)),
        LinuxBackupStep::Answer(BackupFlag::NoItemFlag(HostOs::Linux))
    );
}
