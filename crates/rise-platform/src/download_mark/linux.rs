use std::ffi::{c_char, c_int, c_void, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::download_mark::*;
use super::xdg_origin::*;

unsafe extern "C" {
    fn setxattr(
        path: *const c_char,
        name: *const c_char,
        value: *const c_void,
        size: usize,
        flags: c_int,
    ) -> c_int;
}

pub(super) struct NativeDownloadMarker;

impl DownloadMarker for NativeDownloadMarker {
    fn mark(&self, path: &Path, origin: &DownloadOrigin) -> Result<(), DownloadMarkError> {
        if !path.is_file() {
            return Err(DownloadMarkError::Refused(format!("{} is not a file", path.display())));
        }
        let attributes = xdg_origin_attributes(origin);
        if attributes.is_empty() {
            return Ok(());
        }
        let file = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            DownloadMarkError::Refused(format!("{} holds a NUL byte", path.display()))
        })?;
        for (name, value) in attributes {
            let name = CString::new(name).map_err(|_| {
                DownloadMarkError::Refused(format!("attribute name {name} holds a NUL byte"))
            })?;
            let status = unsafe {
                setxattr(file.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0)
            };
            if status != 0 {
                let error = std::io::Error::last_os_error();
                return Err(DownloadMarkError::Refused(format!("{}: {error}", path.display())));
            }
        }
        Ok(())
    }
}
