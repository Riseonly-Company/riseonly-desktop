use std::path::Path;

use crate::file_manager::{CocoaError, CocoaErrorCode};
use crate::host_os::HostOs;

/// the resource values the app sets, same keys as foundation
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct URLResourceValues {
    pub is_excluded_from_backup: Option<bool>,
}

/// what the os layer did with the excluded from backup flag on one item
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupFlag {
    Set,
    NotNeeded(HostOs),
    Roams(HostOs),
    BranchNotWritten(HostOs),
    NoItemFlag(HostOs),
}

impl BackupFlag {
    /// true when the item now behaves as the caller asked
    pub const fn is_honoured(&self) -> bool {
        matches!(self, Self::Set | Self::NotNeeded(_))
    }
}

pub(crate) trait ResourceValuesWriter {
    fn set_excluded_from_backup(&self, path: &Path, excluded: bool) -> Result<BackupFlag, CocoaError> {
        let _ = (path, excluded);
        Ok(BackupFlag::BranchNotWritten(HostOs::current()))
    }

    fn total_file_allocated_size(&self, path: &Path) -> Option<i64> {
        let _ = path;
        None
    }
}

pub const POSIX_BLOCK_UNIT: u64 = 512;

/// bytes allocated for st_blocks blocks, None if it overflows Int64
pub fn allocated_size_from_blocks(blocks: u64) -> Option<i64> {
    blocks.checked_mul(POSIX_BLOCK_UNIT).and_then(|bytes| i64::try_from(bytes).ok())
}

/// windows size on disk, compressed or sparse size rounded up to whole clusters like explorer shows
/// cluster size zero means an undescribed volume so None
pub fn allocated_size_from_clusters(compressed_size: u64, cluster_size: u64) -> Option<i64> {
    if cluster_size == 0 {
        return None;
    }
    compressed_size
        .div_ceil(cluster_size)
        .checked_mul(cluster_size)
        .and_then(|bytes| i64::try_from(bytes).ok())
}

pub fn total_file_allocated_size(url: &Path) -> Option<i64> {
    super::os::NativeResourceValues.total_file_allocated_size(url)
}

/// windows policy, an item under roaming app data travels with the profile
/// anything else (local app data) stays
pub fn windows_backup_flag(path: &Path, roaming_root: Option<&Path>) -> BackupFlag {
    match roaming_root {
        Some(root) if path.starts_with(root) => BackupFlag::Roams(HostOs::Windows),
        _ => BackupFlag::NotNeeded(HostOs::Windows),
    }
}

/// sets resource values on a path, a missing item fails with fileNoSuchFile
pub fn set_resource_values(url: &Path, values: &URLResourceValues) -> Result<(), CocoaError> {
    set_resource_values_with(&super::os::NativeResourceValues, url, values)
}

pub(crate) fn set_resource_values_with(
    writer: &impl ResourceValuesWriter,
    url: &Path,
    values: &URLResourceValues,
) -> Result<(), CocoaError> {
    if std::fs::symlink_metadata(url).is_err() {
        return Err(CocoaError {
            code: CocoaErrorCode::FileNoSuchFile,
            path: Some(url.to_path_buf()),
            underlying: None,
        });
    }
    let Some(excluded) = values.is_excluded_from_backup else {
        return Ok(());
    };
    let flag = writer.set_excluded_from_backup(url, excluded)?;
    if !flag.is_honoured() {
        tracing::warn!(target: "riseonly", path = %url.display(), ?flag, excluded, "isExcludedFromBackup not applied");
    }
    Ok(())
}
