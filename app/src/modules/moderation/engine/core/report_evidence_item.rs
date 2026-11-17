use rise_core::foundation::foundation_text::{character_count, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::UrlComponents;

use crate::core::stores::http::http_service::HttpService;

const HTTP_PREFIX: &str = "http://";
const HTTPS_PREFIX: &str = "https://";
const HTTP_SCHEME: &str = "http";
const HTTPS_SCHEME: &str = "https";
const MAX_ROUTE_CHARACTERS: usize = 512;
const VIDEO_EXTENSIONS: [&str; 4] = ["mp4", "mov", "m4v", "webm"];

/// evidence lives on a secure route, file-service answers only the reporter and moderation roles
/// stored address is site relative so the image loader needs the api origin
pub enum ReportEvidenceURL {}

impl ReportEvidenceURL {
    pub const ROUTE: &'static str = "/api/file/secure/report-evidence/";

    pub fn absolute(raw: &str) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw);
        if value.is_empty() {
            return None;
        }
        if value.starts_with(HTTP_PREFIX) || value.starts_with(HTTPS_PREFIX) {
            return Some(value.to_owned());
        }
        if !value.starts_with(Self::ROUTE)
            || character_count(value) > MAX_ROUTE_CHARACTERS
            || value.contains("..")
            || value.contains('?')
            || value.contains('#')
            || value.contains('\\')
        {
            return None;
        }
        let mut components = UrlComponents::from_url(&HttpService::resolved_api_base_url(), false)?;
        let scheme = components.scheme()?.to_lowercase();
        if scheme != HTTP_SCHEME && scheme != HTTPS_SCHEME {
            return None;
        }
        if components.host().is_none() {
            return None;
        }
        components.set_path(value);
        components.set_query(None);
        components.set_fragment(None);
        components.url().map(|url| url.absolute_string().to_owned())
    }

    pub fn is_video(raw: &str) -> bool {
        let ext = raw.split('?').find(|part| !part.is_empty()).unwrap_or(raw);
        let extension = ns_string_path_extension(ext).to_lowercase();
        VIDEO_EXTENSIONS.iter().any(|video| *video == extension)
    }
}

fn ns_string_path_extension(candidate: &str) -> &str {
    let trimmed = candidate.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or_default();
    match last.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => extension,
        _ => "",
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReportEvidenceItem {
    pub id: String,
    pub url: String,
    pub absolute_url: Option<String>,
    pub is_video: bool,
}

impl ReportEvidenceItem {
    pub fn items(urls: &[String], ids: &[String]) -> Vec<ReportEvidenceItem> {
        urls.iter()
            .enumerate()
            .map(|(index, url)| {
                let id = ids.get(index).filter(|id| !id.is_empty()).unwrap_or(url);
                ReportEvidenceItem {
                    id: format!("{index}_{id}"),
                    url: url.clone(),
                    absolute_url: ReportEvidenceURL::absolute(url),
                    is_video: ReportEvidenceURL::is_video(url),
                }
            })
            .collect()
    }
}
