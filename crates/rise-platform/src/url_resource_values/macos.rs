use super::url_resource_values::*;

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

use objc2::runtime::AnyObject;
use objc2_foundation::{NSCocoaErrorDomain, NSError, NSNumber, NSURLIsExcludedFromBackupKey, NSURL};

use crate::file_manager::{CocoaError, CocoaErrorCode};

pub(super) struct NativeResourceValues;

impl ResourceValuesWriter for NativeResourceValues {
    fn set_excluded_from_backup(&self, path: &Path, excluded: bool) -> Result<BackupFlag, CocoaError> {
        let url = NSURL::from_file_path(path).ok_or_else(|| CocoaError {
            code: CocoaErrorCode::FileWriteInvalidFileName,
            path: Some(path.to_path_buf()),
            underlying: None,
        })?;
        let flag = NSNumber::numberWithBool(excluded);
        let value: &AnyObject = &flag;
        unsafe { url.setResourceValue_forKey_error(Some(value), NSURLIsExcludedFromBackupKey) }
            .map_err(|error| cocoa_error(path, &error))?;
        Ok(BackupFlag::Set)
    }

    fn total_file_allocated_size(&self, path: &Path) -> Option<i64> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        allocated_size_from_blocks(metadata.blocks())
    }
}

fn cocoa_error(path: &Path, error: &NSError) -> CocoaError {
    let cocoa_domain = unsafe { NSCocoaErrorDomain };
    let raw = error.code() as i64;
    let code = if error.domain().isEqualToString(cocoa_domain) {
        CocoaErrorCode::from_raw(raw).unwrap_or(CocoaErrorCode::FileWriteUnknown)
    } else {
        CocoaErrorCode::FileWriteUnknown
    };
    let detail = format!("{} {raw}: {}", error.domain(), error.localizedDescription());
    CocoaError {
        code,
        path: Some(path.to_path_buf()),
        underlying: Some(Arc::new(io::Error::other(detail))),
    }
}
