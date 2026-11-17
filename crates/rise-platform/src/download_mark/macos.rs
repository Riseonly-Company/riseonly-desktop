use std::path::Path;

use objc2::runtime::AnyObject;
use objc2_foundation::{NSMutableDictionary, NSString, NSURLQuarantinePropertiesKey, NSURL};

use super::download_mark::*;

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    static kLSQuarantineTypeKey: &'static NSString;
    static kLSQuarantineTypeWebDownload: &'static NSString;
    static kLSQuarantineDataURLKey: &'static NSString;
    static kLSQuarantineOriginURLKey: &'static NSString;
}

pub(super) struct NativeDownloadMarker;

impl DownloadMarker for NativeDownloadMarker {
    fn mark(&self, path: &Path, origin: &DownloadOrigin) -> Result<(), DownloadMarkError> {
        if !path.is_file() {
            return Err(DownloadMarkError::Refused(format!("{} is not a file", path.display())));
        }
        let url = NSURL::from_file_path(path).ok_or_else(|| {
            DownloadMarkError::Refused(format!("{} is not an absolute file path", path.display()))
        })?;
        let properties = NSMutableDictionary::<NSString, AnyObject>::new();
        let (type_key, web_download, data_key, origin_key) = unsafe {
            (
                kLSQuarantineTypeKey,
                kLSQuarantineTypeWebDownload,
                kLSQuarantineDataURLKey,
                kLSQuarantineOriginURLKey,
            )
        };
        properties.insert(type_key, web_download);
        if let Some(host) = quarantine_url(origin.host_url.as_deref()) {
            properties.insert(data_key, &host);
        }
        if let Some(referrer) = quarantine_url(origin.referrer_url.as_deref()) {
            properties.insert(origin_key, &referrer);
        }
        let value: &AnyObject = &properties;
        unsafe { url.setResourceValue_forKey_error(Some(value), NSURLQuarantinePropertiesKey) }
            .map_err(|error| {
                DownloadMarkError::Refused(format!(
                    "{}: {}",
                    path.display(),
                    error.localizedDescription()
                ))
            })
    }
}

/// recorded part of url (no user info
/// query or fragment) as the nsurl launch services keeps in the attribute
fn quarantine_url(url: Option<&str>) -> Option<objc2::rc::Retained<NSURL>> {
    let recorded = recorded_url(url?)?;
    NSURL::URLWithString(&NSString::from_str(&recorded))
}
