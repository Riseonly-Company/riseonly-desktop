use super::cachedir_tag::*;
use super::url_resource_values::*;

use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::file_manager::CocoaError;
use crate::host_os::HostOs;

pub(super) struct NativeResourceValues;

impl ResourceValuesWriter for NativeResourceValues {
    fn set_excluded_from_backup(&self, path: &Path, excluded: bool) -> Result<BackupFlag, CocoaError> {
        let is_dir = std::fs::metadata(path).is_ok_and(|meta| meta.is_dir());
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let cache_home = xdg_cache_home(std::env::var_os("XDG_CACHE_HOME").as_deref(), home.as_deref());
        let mut item = LinuxBackupItem {
            path,
            is_dir,
            excluded,
            cache_home: cache_home.as_deref(),
            has_own_tag: is_dir && holds_tag(path),
            inside_tagged_dir: path
                .ancestors()
                .skip(1)
                .filter(|dir| dir.is_absolute())
                .any(holds_tag),
        };
        Ok(match linux_backup_step(&item) {
            LinuxBackupStep::Answer(flag) => flag,
            LinuxBackupStep::WriteTag => match std::fs::write(path.join(CACHEDIR_TAG_NAME), CACHEDIR_TAG_CONTENTS) {
                Ok(()) => BackupFlag::Set,
                Err(error) => {
                    tracing::warn!(target: "riseonly", path = %path.display(), %error, "CACHEDIR.TAG not written");
                    BackupFlag::NoItemFlag(HostOs::Linux)
                }
            },
            LinuxBackupStep::RemoveTag => match std::fs::remove_file(path.join(CACHEDIR_TAG_NAME)) {
                Ok(()) => {
                    item.has_own_tag = false;
                    match linux_backup_step(&item) {
                        LinuxBackupStep::Answer(flag) => flag,
                        LinuxBackupStep::WriteTag | LinuxBackupStep::RemoveTag => BackupFlag::NoItemFlag(HostOs::Linux),
                    }
                }
                Err(error) => {
                    tracing::warn!(target: "riseonly", path = %path.display(), %error, "CACHEDIR.TAG not removed");
                    BackupFlag::NoItemFlag(HostOs::Linux)
                }
            },
        })
    }

    fn total_file_allocated_size(&self, path: &Path) -> Option<i64> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        allocated_size_from_blocks(metadata.blocks())
    }
}

/// true when the dir has a file whose head is the tag signature
fn holds_tag(dir: &Path) -> bool {
    let mut head = Vec::with_capacity(CACHEDIR_TAG_SIGNATURE.len());
    let read = std::fs::File::open(dir.join(CACHEDIR_TAG_NAME))
        .and_then(|file| file.take(CACHEDIR_TAG_SIGNATURE.len() as u64).read_to_end(&mut head));
    is_cachedir_tag(read.ok().map(|_| head.as_slice()))
}
