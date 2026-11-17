use std::collections::HashSet;
use std::sync::LazyLock;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMediaKind {
    Image,
    Video,
    Audio,
    File,
}

impl RiseMediaKind {
    pub fn raw_value(&self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::File => "file",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

static IMAGE_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "jpg", "jpeg", "png", "heic", "heif", "gif", "tiff", "tif", "bmp", "webp", "dng",
    ])
});

static VIDEO_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "mp4", "mov", "m4v", "hevc", "avi", "mkv", "webm", "3gp", "mpeg", "mpg", "wmv",
    ])
});

static AUDIO_EXTENSIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "mp3", "m4a", "aac", "wav", "flac", "ogg", "oga", "opus", "aiff", "aif", "caf", "wma",
    ])
});

/// classifies media kind for save to gallery and the storage report
/// mime wins when real, octet-stream means unknown so the extension decides
pub enum RiseMediaKindClassifier {}

impl RiseMediaKindClassifier {
    pub fn image_extensions() -> &'static HashSet<&'static str> {
        &IMAGE_EXTENSIONS
    }

    pub fn video_extensions() -> &'static HashSet<&'static str> {
        &VIDEO_EXTENSIONS
    }

    pub fn audio_extensions() -> &'static HashSet<&'static str> {
        &AUDIO_EXTENSIONS
    }

    pub fn kind(mime_type: Option<&str>, file_name: Option<&str>) -> RiseMediaKind {
        Self::kind_file_extension(mime_type, &Self::file_extension(file_name))
    }

    pub fn kind_file_extension(mime_type: Option<&str>, ext: &str) -> RiseMediaKind {
        if let Some(mime) = Self::normalized_mime(mime_type) {
            if mime.starts_with("video/") {
                return RiseMediaKind::Video;
            }
            if mime.starts_with("image/") {
                return RiseMediaKind::Image;
            }
            if mime.starts_with("audio/") {
                return RiseMediaKind::Audio;
            }
            return RiseMediaKind::File;
        }
        if VIDEO_EXTENSIONS.contains(ext) {
            return RiseMediaKind::Video;
        }
        if IMAGE_EXTENSIONS.contains(ext) {
            return RiseMediaKind::Image;
        }
        if AUDIO_EXTENSIONS.contains(ext) {
            return RiseMediaKind::Audio;
        }
        RiseMediaKind::File
    }

    pub fn normalized_mime(raw: Option<&str>) -> Option<String> {
        let raw = raw?;
        let first = raw.split(';').find(|piece| !piece.is_empty()).unwrap_or_default();
        let value = trimming_whitespaces_and_newlines(first).to_lowercase();
        if value.is_empty() || value == "application/octet-stream" {
            return None;
        }
        Some(value)
    }

    pub fn file_extension(candidate: Option<&str>) -> String {
        let Some(candidate) = candidate else {
            return String::new();
        };
        Self::ns_string_path_extension(candidate)
            .to_lowercase()
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .collect()
    }

    pub fn first_file_extension(candidates: &[Option<&str>]) -> String {
        for candidate in candidates {
            let value = Self::file_extension(*candidate);
            if !value.is_empty() {
                return value;
            }
        }
        String::new()
    }

    fn ns_string_path_extension(candidate: &str) -> &str {
        let trimmed = candidate.trim_end_matches('/');
        let last = trimmed.rsplit('/').next().unwrap_or_default();
        match last.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => extension,
            _ => "",
        }
    }
}
