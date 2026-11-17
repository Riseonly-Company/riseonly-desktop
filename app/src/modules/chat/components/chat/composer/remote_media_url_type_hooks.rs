use std::sync::LazyLock;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::{UrlComponents, removing_percent_encoding};
use rise_core::foundation::foundation_uuid::Uuid;

use crate::core::stores::http::http_service::HttpService;

const SECURE_FILE_ROUTE_PREFIX: &str = "/api/file/secure/";
const SECURE_CHAT_ROUTE_PREFIX: &str = "/api/file/secure/chat/";
const SECURE_ROUTE_MAX_LENGTH: usize = 256;
const SHARED_MUSIC_SEGMENT: &str = "shared-music";
const SHARED_MUSIC_EXTENSIONS: [&str; 5] = ["mp3", "jpg", "jpeg", "png", "webp"];
const THUMBNAIL_PREFIX: &str = "thumb_";
const OWNER_SCOPED_SECURE_ROUTES: [&str; 2] = ["story", "circle"];
const PUBLIC_MEDIA_BUCKET_PREFIX: &str = "/riseonly-s3/";

static LEGACY_PUBLIC_MEDIA_ORIGIN_SUFFIXES: LazyLock<Vec<String>> = LazyLock::new(|| {
    rise_validate::PUBLIC_MEDIA_LEGACY_ORIGIN_SUFFIXES
        .iter()
        .map(|suffix| suffix.to_lowercase())
        .collect()
});
static CANONICAL_PUBLIC_MEDIA_HOST: LazyLock<String> =
    LazyLock::new(|| rise_validate::PUBLIC_MEDIA_CANONICAL_HOST.to_lowercase());

pub enum RemoteMediaURL {}

impl RemoteMediaURL {
    pub fn optional_normalized(raw: Option<&str>) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(raw?);
        if trimmed.is_empty() {
            return None;
        }
        Some(Self::normalize(trimmed))
    }

    pub fn display_source(url: &str, thumbnail: Option<&str>) -> String {
        if let Some(thumb) = Self::optional_normalized(thumbnail) {
            return thumb;
        }
        Self::optional_normalized(Some(url))
            .unwrap_or_else(|| trimming_whitespaces_and_newlines(url).to_owned())
    }

    fn normalize(raw: &str) -> String {
        let s = raw.replace("\\/", "/").replace('\\', "");
        if let Some(secure_url) = Self::riseonly_secure_chat_media_url(&s) {
            return secure_url;
        }
        if let Some(secure_url) = Self::riseonly_secure_route_media_url(&s) {
            return secure_url;
        }
        let Some(proto) = s.find("://") else {
            return s;
        };
        let head = &s[..proto + 3];
        let mut tail = s[proto + 3..].to_owned();
        while tail.contains("//") {
            tail = tail.replace("//", "/");
        }
        let normalized = format!("{head}{tail}");
        Self::riseonly_public_cdn_url(&normalized).unwrap_or(normalized)
    }

    fn is_bounded_secure_route(raw: &str, prefix: &str) -> bool {
        raw.starts_with(prefix)
            && raw.chars().count() <= SECURE_ROUTE_MAX_LENGTH
            && !raw.contains("..")
            && !raw.contains('%')
            && !raw.contains('?')
            && !raw.contains('#')
            && !raw.contains('\\')
    }

    fn is_lowercase_uuid(raw: &str) -> bool {
        Uuid::from_uuid_string(raw).is_some_and(|uuid| uuid.uuid_string().to_lowercase() == raw)
    }

    fn riseonly_secure_chat_media_url(raw: &str) -> Option<String> {
        if !Self::is_bounded_secure_route(raw, SECURE_CHAT_ROUTE_PREFIX) {
            return None;
        }
        let parts: Vec<&str> = raw.split('/').collect();
        if !(parts.len() == 7 || parts.len() == 8)
            || parts[1] != "api"
            || parts[2] != "file"
            || parts[3] != "secure"
            || parts[4] != "chat"
            || !Self::is_lowercase_uuid(parts[5])
        {
            return None;
        }

        if parts.len() == 8 {
            if parts[6] != SHARED_MUSIC_SEGMENT
                || !Self::is_secure_chat_object_name(parts[7], Some(SHARED_MUSIC_EXTENSIONS.as_slice()), false)
            {
                return None;
            }
        } else if !Self::is_secure_chat_object_name(parts[6], None, true) {
            return None;
        }

        Self::absolute_api_url(raw)
    }

    fn riseonly_secure_route_media_url(raw: &str) -> Option<String> {
        if !Self::is_bounded_secure_route(raw, SECURE_FILE_ROUTE_PREFIX) {
            return None;
        }
        let parts: Vec<&str> = raw.split('/').collect();
        if parts.len() != 7
            || parts[1] != "api"
            || parts[2] != "file"
            || parts[3] != "secure"
            || !OWNER_SCOPED_SECURE_ROUTES.contains(&parts[4])
            || !Self::is_secure_route_segment(parts[5])
            || !Self::is_secure_route_segment(parts[6])
            || !parts[6].contains('.')
        {
            return None;
        }
        Self::absolute_api_url(raw)
    }

    fn is_secure_route_segment(segment: &str) -> bool {
        !segment.is_empty()
            && !segment.starts_with('.')
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.' || byte == b'_')
    }

    fn absolute_api_url(path: &str) -> Option<String> {
        let mut components = UrlComponents::from_url(&HttpService::resolved_api_base_url(), false)?;
        let scheme = components.scheme()?.to_lowercase();
        if !(scheme == "http" || scheme == "https")
            || components.host().is_none()
            || components.user().is_some()
            || components.password().is_some()
        {
            return None;
        }
        components.set_path(path);
        components.set_query(None);
        components.set_fragment(None);
        components.url().map(|url| url.absolute_string().to_owned())
    }

    fn is_secure_chat_object_name(
        filename: &str,
        allowed_extensions: Option<&[&str]>,
        allow_thumbnail_prefix: bool,
    ) -> bool {
        let Some(separator) = filename.rfind('.') else {
            return false;
        };
        let mut identifier = &filename[..separator];
        if allow_thumbnail_prefix {
            if let Some(stripped) = identifier.strip_prefix(THUMBNAIL_PREFIX) {
                identifier = stripped;
            }
        }
        if !Self::is_lowercase_uuid(identifier) {
            return false;
        }

        let ext = filename[separator + 1..].to_lowercase();
        if let Some(allowed_extensions) = allowed_extensions {
            return allowed_extensions.contains(&ext.as_str());
        }
        let length = ext.chars().count();
        (1..=8).contains(&length)
            && ext
                .chars()
                .all(|character| character.is_ascii() && (character.is_lowercase() || character.is_numeric()))
    }

    fn riseonly_public_cdn_url(raw: &str) -> Option<String> {
        let source = UrlComponents::from_string(raw)?;
        let host = source.host()?.to_lowercase();
        let is_legacy_origin = LEGACY_PUBLIC_MEDIA_ORIGIN_SUFFIXES
            .iter()
            .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")));
        if !is_legacy_origin {
            return None;
        }

        let decoded_path =
            removing_percent_encoding(source.percent_encoded_path()).unwrap_or_else(|| source.path());
        let object_key = decoded_path.strip_prefix(PUBLIC_MEDIA_BUCKET_PREFIX)?;
        let public_prefix = rise_validate::PUBLIC_MEDIA_PATH_PREFIX;
        let public_prefix = public_prefix.strip_prefix('/').unwrap_or(public_prefix);
        if !object_key.starts_with(public_prefix) {
            return None;
        }

        let mut target = UrlComponents::new();
        target.set_scheme(Some("https"));
        target.set_host(Some(CANONICAL_PUBLIC_MEDIA_HOST.as_str()));
        target.set_path(&format!("/{object_key}"));
        target.set_percent_encoded_query(source.percent_encoded_query());
        target.string()
    }
}
