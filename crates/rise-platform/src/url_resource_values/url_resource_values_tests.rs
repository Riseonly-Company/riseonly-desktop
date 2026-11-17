use std::cell::Cell;
use std::path::{Path, PathBuf};

use crate::file_manager::{CocoaError, CocoaErrorCode};
use crate::host_os::HostOs;
use crate::url_resource_values::*;

struct DefaultWriter;

impl ResourceValuesWriter for DefaultWriter {}

#[derive(Default)]
struct CountingWriter {
    calls: Cell<usize>,
}

impl ResourceValuesWriter for CountingWriter {
    fn set_excluded_from_backup(&self, _path: &Path, _excluded: bool) -> Result<BackupFlag, CocoaError> {
        self.calls.set(self.calls.get() + 1);
        Ok(BackupFlag::Set)
    }
}

fn excluded() -> URLResourceValues {
    URLResourceValues {
        is_excluded_from_backup: Some(true),
    }
}

#[test]
fn a_missing_item_throws_file_no_such_file() {
    let missing = std::env::temp_dir().join(format!("rise-resource-missing-{}", std::process::id()));
    let error = set_resource_values_with(&CountingWriter::default(), &missing, &excluded()).unwrap_err();
    assert_eq!(error.code, CocoaErrorCode::FileNoSuchFile);
    assert_eq!(error.path.as_deref(), Some(missing.as_path()));
}

#[test]
fn values_without_the_backup_key_touch_nothing() {
    let writer = CountingWriter::default();
    let dir = std::env::temp_dir();
    set_resource_values_with(&writer, &dir, &URLResourceValues::default()).unwrap();
    assert_eq!(writer.calls.get(), 0);
    set_resource_values_with(&writer, &dir, &excluded()).unwrap();
    assert_eq!(writer.calls.get(), 1);
}

#[test]
fn a_host_without_a_branch_does_not_fail_the_store() {
    let dir = std::env::temp_dir();
    assert!(set_resource_values_with(&DefaultWriter, &dir, &excluded()).is_ok());
    let flag = DefaultWriter.set_excluded_from_backup(&dir, true).unwrap();
    assert_eq!(flag, BackupFlag::BranchNotWritten(HostOs::current()));
    assert!(!flag.is_honoured());
}

#[test]
fn on_windows_only_the_roaming_folder_travels() {
    let roaming = PathBuf::from("C:/Users/u/AppData/Roaming");
    let local = Path::new("C:/Users/u/AppData/Local/Riseonly/store.sqlite");
    let roamed = Path::new("C:/Users/u/AppData/Roaming/Riseonly/store.sqlite");
    assert_eq!(windows_backup_flag(local, Some(&roaming)), BackupFlag::NotNeeded(HostOs::Windows));
    assert_eq!(windows_backup_flag(roamed, Some(&roaming)), BackupFlag::Roams(HostOs::Windows));
    assert_eq!(windows_backup_flag(roamed, None), BackupFlag::NotNeeded(HostOs::Windows));
    assert!(BackupFlag::NotNeeded(HostOs::Windows).is_honoured());
    assert!(!BackupFlag::Roams(HostOs::Windows).is_honoured());
}

#[test]
fn a_host_without_an_allocation_arm_answers_nil() {
    assert_eq!(DefaultWriter.total_file_allocated_size(&std::env::temp_dir()), None);
}

#[test]
fn posix_blocks_count_in_512_byte_units() {
    assert_eq!(allocated_size_from_blocks(0), Some(0));
    assert_eq!(allocated_size_from_blocks(8), Some(4096));
    assert_eq!(allocated_size_from_blocks(u64::MAX), None);
    assert_eq!(allocated_size_from_blocks(i64::MAX as u64 / 512), Some(i64::MAX / 512 * 512));
}

#[test]
fn windows_size_on_disk_rounds_up_to_whole_clusters() {
    assert_eq!(allocated_size_from_clusters(0, 4096), Some(0));
    assert_eq!(allocated_size_from_clusters(1, 4096), Some(4096));
    assert_eq!(allocated_size_from_clusters(4096, 4096), Some(4096));
    assert_eq!(allocated_size_from_clusters(4097, 4096), Some(8192));
    assert_eq!(allocated_size_from_clusters(10, 0), None);
    assert_eq!(allocated_size_from_clusters(u64::MAX, 4096), None);
}

#[test]
fn a_missing_item_has_no_allocated_size() {
    let missing = std::env::temp_dir().join(format!("rise-allocated-missing-{}", std::process::id()));
    assert_eq!(total_file_allocated_size(&missing), None);
}
