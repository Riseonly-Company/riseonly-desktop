use std::path::Path;

use super::folder_library::ThumbnailBox;
use super::xdg_library::thumbnail_candidates;

pub const THUMBNAILER_BUS_NAME: &str = "org.freedesktop.thumbnails.Thumbnailer1";
pub const THUMBNAILER_OBJECT_PATH: &str = "/org/freedesktop/thumbnails/Thumbnailer1";
pub const THUMBNAILER_INTERFACE: &str = "org.freedesktop.thumbnails.Thumbnailer1";
pub const THUMBNAILER_QUEUE: &str = "Queue";
pub const THUMBNAILER_GET_FLAVORS: &str = "GetFlavors";
pub const THUMBNAILER_GET_SUPPORTED: &str = "GetSupported";
pub const THUMBNAILER_READY: &str = "Ready";
pub const THUMBNAILER_ERROR: &str = "Error";
pub const THUMBNAILER_FINISHED: &str = "Finished";
pub const THUMBNAILER_SCHEDULER: &str = "default";
pub const THUMBNAILER_NO_UNQUEUE: u32 = 0;

pub const VIDEO_MIME_TYPES: [(&str, &str); 8] = [
    ("mp4", "video/mp4"),
    ("mov", "video/quicktime"),
    ("m4v", "video/x-m4v"),
    ("avi", "video/x-msvideo"),
    ("mkv", "video/x-matroska"),
    ("webm", "video/webm"),
    ("wmv", "video/x-ms-wmv"),
    ("3gp", "video/3gpp"),
];

/// mime type of a video file by extension ignoring case, none for any other file
pub fn video_mime_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    VIDEO_MIME_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, mime)| *mime)
}

/// flavor to ask for, the folder the cache lookup tries first among those the service makes
/// so the made thumbnail is the one the next lookup reads
pub fn thumbnailer_flavor(target: ThumbnailBox, supported: &[String]) -> Option<&'static str> {
    thumbnail_candidates(target)
        .into_iter()
        .map(|(name, _)| name)
        .find(|name| supported.iter().any(|flavor| flavor == name))
}

/// one signal of the thumbnailer decoded from its body
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThumbnailerSignal {
    Ready { handle: u32, uris: Vec<String> },
    Error { handle: u32, uris: Vec<String> },
    Finished { handle: u32 },
    Unreadable,
}

/// what the wait for one requested uri does after a signal
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbnailerStep {
    Wait,
    LookInCache,
    GiveUp,
}

/// step for the signal while waiting on handle for uri
pub fn thumbnailer_step(handle: u32, uri: &str, signal: &ThumbnailerSignal) -> ThumbnailerStep {
    let names = |uris: &[String]| uris.iter().any(|item| item == uri);
    match signal {
        ThumbnailerSignal::Ready { handle: from, uris } if *from == handle && names(uris) => {
            ThumbnailerStep::LookInCache
        }
        ThumbnailerSignal::Error { handle: from, uris } if *from == handle && names(uris) => ThumbnailerStep::GiveUp,
        ThumbnailerSignal::Finished { handle: from } if *from == handle => ThumbnailerStep::LookInCache,
        _ => ThumbnailerStep::Wait,
    }
}
