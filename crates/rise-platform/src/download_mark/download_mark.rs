use std::path::Path;

use thiserror::Error;

use crate::host_os::HostOs;

pub const ZONE_IDENTIFIER_STREAM: &str = "Zone.Identifier";

pub const INTERNET_ZONE_ID: u32 = 3;

/// where a downloaded file came from
/// both are optional, the zone alone is enough for the warnings and the urls only tell the
/// user the source
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DownloadOrigin {
    pub host_url: Option<String>,
    pub referrer_url: Option<String>,
}

/// why a file stays without the mark, the file is fine and only the warning is missing
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum DownloadMarkError {
    #[error("download marking is not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("the file system refused the download mark: {0}")]
    Refused(String),
}

pub trait DownloadMarker {
    fn mark(&self, path: &Path, origin: &DownloadOrigin) -> Result<(), DownloadMarkError> {
        let _ = (path, origin);
        Err(DownloadMarkError::BranchNotWritten(HostOs::current()))
    }
}

/// text of the Zone.Identifier stream for origin, in the form browsers write
/// zoneTransfer section with ZoneId, then ReferrerUrl and HostUrl when known and crlf line ends
pub fn zone_identifier_text(origin: &DownloadOrigin) -> String {
    let mut text = format!("[ZoneTransfer]\r\nZoneId={INTERNET_ZONE_ID}\r\n");
    if let Some(referrer) = origin.referrer_url.as_deref().and_then(recorded_url) {
        text.push_str(&format!("ReferrerUrl={referrer}\r\n"));
    }
    if let Some(host) = origin.host_url.as_deref().and_then(recorded_url) {
        text.push_str(&format!("HostUrl={host}\r\n"));
    }
    text
}

/// part of a url the mark keeps, scheme host and path
/// user info and query and fragment are dropped, signed links carry tokens there
/// not an absolute one line url means nothing is recorded
pub fn recorded_url(url: &str) -> Option<String> {
    if url.contains(['\r', '\n']) {
        return None;
    }
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty() || !scheme.chars().all(|character| character.is_ascii_alphanumeric() || "+-.".contains(character)) {
        return None;
    }
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = match rest.find('/') {
        Some(index) => rest.split_at(index),
        None => (rest, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or_default();
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}{path}"))
}

/// marks the file at path as downloaded from origin
/// blocking file io, call it off the main thread right after the file is complete
/// on err the caller logs it and keeps the file
pub fn mark_downloaded(path: &Path, origin: &DownloadOrigin) -> Result<(), DownloadMarkError> {
    super::os::NativeDownloadMarker.mark(path, origin)
}
