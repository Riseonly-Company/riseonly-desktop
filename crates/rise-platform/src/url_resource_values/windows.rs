use super::url_resource_values::*;

use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{GetLastError, MAX_PATH, NO_ERROR};
use windows::Win32::Storage::FileSystem::{
    GetCompressedFileSizeW, GetDiskFreeSpaceW, GetVolumePathNameW, INVALID_FILE_SIZE,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::core::PCWSTR;
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

use crate::file_manager::CocoaError;

pub(super) struct NativeResourceValues;

impl ResourceValuesWriter for NativeResourceValues {
    fn set_excluded_from_backup(&self, path: &Path, _excluded: bool) -> Result<BackupFlag, CocoaError> {
        Ok(windows_backup_flag(path, roaming_app_data().as_deref()))
    }

    fn total_file_allocated_size(&self, path: &Path) -> Option<i64> {
        if std::fs::symlink_metadata(path).ok()?.file_type().is_symlink() {
            return None;
        }
        let wide = nul_terminated(path.as_os_str());
        let compressed = compressed_file_size(&wide)?;
        let cluster = cluster_size(&wide)?;
        allocated_size_from_clusters(compressed, cluster)
    }
}

fn nul_terminated(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn compressed_file_size(wide: &[u16]) -> Option<u64> {
    let mut high = 0u32;
    let low = unsafe { GetCompressedFileSizeW(PCWSTR(wide.as_ptr()), Some(&mut high)) };
    if low == INVALID_FILE_SIZE && unsafe { GetLastError() } != NO_ERROR {
        return None;
    }
    Some((u64::from(high) << 32) | u64::from(low))
}

fn cluster_size(wide: &[u16]) -> Option<u64> {
    let mut volume = vec![0u16; wide.len().max(MAX_PATH as usize) + 1];
    let resolved = unsafe { GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut volume) };
    resolved.ok()?;
    let mut sectors_per_cluster = 0u32;
    let mut bytes_per_sector = 0u32;
    let described = unsafe {
        GetDiskFreeSpaceW(
            PCWSTR(volume.as_ptr()),
            Some(&mut sectors_per_cluster),
            Some(&mut bytes_per_sector),
            None,
            None,
        )
    };
    described.ok()?;
    Some(u64::from(sectors_per_cluster) * u64::from(bytes_per_sector))
}

fn roaming_app_data() -> Option<PathBuf> {
    let path = unsafe { SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None) }.ok()?;
    let wide = unsafe { path.as_wide() }.to_vec();
    unsafe { CoTaskMemFree(Some(path.0 as *const c_void)) };
    Some(PathBuf::from(OsString::from_wide(&wide)))
}
