use std::ffi::OsString;
use std::path::Path;

use super::download_mark::*;

pub(super) struct NativeDownloadMarker;

impl DownloadMarker for NativeDownloadMarker {
    fn mark(&self, path: &Path, origin: &DownloadOrigin) -> Result<(), DownloadMarkError> {
        if !path.is_file() {
            return Err(DownloadMarkError::Refused(format!("{} is not a file", path.display())));
        }
        let mut stream = OsString::from(path.as_os_str());
        stream.push(":");
        stream.push(ZONE_IDENTIFIER_STREAM);
        std::fs::write(&stream, zone_identifier_text(origin))
            .map_err(|error| DownloadMarkError::Refused(format!("{}: {error}", path.display())))
    }
}
