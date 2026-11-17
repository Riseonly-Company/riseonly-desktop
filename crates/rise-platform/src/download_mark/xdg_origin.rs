use super::download_mark::{recorded_url, DownloadOrigin};

pub const ORIGIN_URL_ATTRIBUTE: &str = "user.xdg.origin.url";

pub const REFERRER_URL_ATTRIBUTE: &str = "user.xdg.referrer.url";

/// attributes to write for origin, origin first and as name and value pairs
/// each value is the recorded part of its url, an unrecorded or missing url leaves its attribute out
pub fn xdg_origin_attributes(origin: &DownloadOrigin) -> Vec<(&'static str, String)> {
    let mut attributes = Vec::with_capacity(2);
    if let Some(host) = origin.host_url.as_deref().and_then(recorded_url) {
        attributes.push((ORIGIN_URL_ATTRIBUTE, host));
    }
    if let Some(referrer) = origin.referrer_url.as_deref().and_then(recorded_url) {
        attributes.push((REFERRER_URL_ATTRIBUTE, referrer));
    }
    attributes
}
