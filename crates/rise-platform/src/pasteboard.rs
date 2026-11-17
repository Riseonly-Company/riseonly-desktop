use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, ClipboardEntry, ClipboardItem, Image, ImageFormat};
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::{is_whitespace_or_newline, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;

use crate::host_os::HostOs;
use crate::pasteboard_write;

/// the system pasteboard, general() is the shared one
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UIPasteboard;

/// one value to put on the pasteboard, keyed by type
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UIPasteboardItem {
    PlainText(String),
    Url(Url),
    Image(Image),
}

/// options for setting pasteboard items
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UIPasteboardOptions {
    pub local_only: bool,
    pub expiration_date: Option<Date>,
}

impl UIPasteboard {
    pub const fn general() -> Self {
        Self
    }

    /// text on the clipboard, none when it has none
    pub fn string(&self, cx: &App) -> Option<String> {
        cx.read_from_clipboard().as_ref().and_then(string_in)
    }

    /// replaces the clipboard with this text
    pub fn set_string(&self, value: &str, cx: &App) {
        cx.write_to_clipboard(ClipboardItem::new_string(value.to_owned()));
    }

    /// every text entry on the clipboard in order
    pub fn strings(&self, cx: &App) -> Vec<String> {
        cx.read_from_clipboard().as_ref().map(strings_in).unwrap_or_default()
    }

    pub fn has_strings(&self, cx: &App) -> bool {
        cx.read_from_clipboard().as_ref().is_some_and(has_strings_in)
    }

    /// first image on the clipboard
    pub fn image(&self, cx: &App) -> Option<Image> {
        cx.read_from_clipboard().as_ref().and_then(image_in)
    }

    pub fn set_image(&self, value: &Image, cx: &App) {
        if value.format == ImageFormat::Png {
            match pasteboard_write::write_image_png(value.bytes()) {
                Ok(()) => return,
                Err(unavailable) => {
                    tracing::debug!(target: "riseonly", %unavailable, "the image is copied through gpui");
                }
            }
        }
        cx.write_to_clipboard(ClipboardItem::new_image(value));
    }

    pub fn has_images(&self, cx: &App) -> bool {
        cx.read_from_clipboard().as_ref().is_some_and(has_images_in)
    }

    /// desktop only, the files copied in a file manager
    /// paths arent checked so the caller deals with ones that are gone
    pub fn file_paths(&self, cx: &App) -> Vec<PathBuf> {
        cx.read_from_clipboard()
            .as_ref()
            .map(|item| file_paths_in(item, HostOs::current()))
            .unwrap_or_default()
    }

    /// clipboard text when it is exactly one absolute url
    pub fn url(&self, cx: &App) -> Option<Url> {
        cx.read_from_clipboard().as_ref().and_then(url_in)
    }

    /// replaces the clipboard with the absolute url string
    /// written under the host url type if there is one and always as plain text so other apps can paste it
    pub fn set_url(&self, value: &Url, cx: &App) {
        if let Err(unavailable) = pasteboard_write::write_url(value.absolute_string()) {
            tracing::debug!(target: "riseonly", %unavailable, "the link is copied as text only");
            self.set_string(value.absolute_string(), cx);
        }
    }

    pub fn has_urls(&self, cx: &App) -> bool {
        self.url(cx).is_some()
    }

    pub fn set_items(&self, items: Vec<UIPasteboardItem>, options: UIPasteboardOptions, cx: &App) {
        let written = clipboard_item_of(items);
        if !(options.local_only && write_host_only(&written)) {
            cx.write_to_clipboard(written.clone());
        }
        let Some(expiration_date) = options.expiration_date else {
            return;
        };
        let delay = expiry_delay(expiration_date, Date::now());
        let written_at_quit = written.clone();
        let expire_at_quit = cx.on_app_quit(move |cx| {
            expire_if_still_held(&written_at_quit, cx);
            async {}
        });
        cx.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            drop(expire_at_quit);
            cx.update(|cx| expire_if_still_held(&written, cx));
        })
        .detach();
    }
}

fn expire_if_still_held(written: &ClipboardItem, cx: &App) {
    if still_holds(cx.read_from_clipboard().as_ref(), written) {
        cx.write_to_clipboard(expired_item());
    }
}

fn write_host_only(written: &ClipboardItem) -> bool {
    let [ClipboardEntry::String(text)] = written.entries() else {
        return false;
    };
    match pasteboard_write::write_host_only_text(text.text()) {
        Ok(()) => true,
        Err(unavailable) => {
            tracing::debug!(target: "riseonly", %unavailable, "the local-only copy is an ordinary copy");
            false
        }
    }
}

/// clipboard item for setting items, every value as one entry in the given order
pub fn clipboard_item_of(items: Vec<UIPasteboardItem>) -> ClipboardItem {
    let entries = items
        .into_iter()
        .map(|item| match item {
            UIPasteboardItem::PlainText(text) => ClipboardEntry::from(text),
            UIPasteboardItem::Url(url) => ClipboardEntry::from(url.absolute_string().to_owned()),
            UIPasteboardItem::Image(image) => ClipboardEntry::from(image),
        })
        .collect();
    ClipboardItem { entries }
}

/// text entries joined as one string, a copied file path isnt text unlike gpui text()
pub fn string_in(item: &ClipboardItem) -> Option<String> {
    let joined = strings_in(item).concat();
    (!joined.is_empty()).then_some(joined)
}

/// every text entry of the item in order
pub fn strings_in(item: &ClipboardItem) -> Vec<String> {
    item.entries()
        .iter()
        .filter_map(|entry| match entry {
            ClipboardEntry::String(string) => Some(string.text().clone()),
            ClipboardEntry::Image(_) | ClipboardEntry::ExternalPaths(_) => None,
        })
        .collect()
}

/// true when the item has any non empty text
pub fn has_strings_in(item: &ClipboardItem) -> bool {
    string_in(item).is_some()
}

/// first image entry of the item
pub fn image_in(item: &ClipboardItem) -> Option<Image> {
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(image.clone()),
        ClipboardEntry::String(_) | ClipboardEntry::ExternalPaths(_) => None,
    })
}

pub fn has_images_in(item: &ClipboardItem) -> bool {
    image_in(item).is_some()
}

/// files on the item for this host, its path entries
/// and on linux its text when that is only a list of files
pub fn file_paths_in(item: &ClipboardItem, host: HostOs) -> Vec<PathBuf> {
    let paths: Vec<PathBuf> = item
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            ClipboardEntry::ExternalPaths(paths) => Some(paths.paths().to_vec()),
            ClipboardEntry::String(_) | ClipboardEntry::Image(_) => None,
        })
        .flatten()
        .collect();
    if !paths.is_empty() || host != HostOs::Linux {
        return paths;
    }
    string_in(item).map(|text| file_paths_in_text(&text)).unwrap_or_default()
}

/// text where every line names a local file, an absolute path or a file url
/// blank lines and # comments of uri-list are skipped, one other line means not a file list
pub fn file_paths_in_text(text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let line = trimming_whitespaces_and_newlines(line);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(path) = file_path_of_line(line) else {
            return Vec::new();
        };
        paths.push(path);
    }
    paths
}

fn file_path_of_line(line: &str) -> Option<PathBuf> {
    if line.starts_with('/') {
        return Some(PathBuf::from(line));
    }
    let url = Url::from_string(line)?;
    if !url.is_file_url() {
        return None;
    }
    if url.host().is_some_and(|host| !host.eq_ignore_ascii_case("localhost")) {
        return None;
    }
    let path = url.path();
    path.starts_with('/').then(|| PathBuf::from(path))
}

/// the item text as a url, one token with a scheme
pub fn url_in(item: &ClipboardItem) -> Option<Url> {
    let text = string_in(item)?;
    let candidate = trimming_whitespaces_and_newlines(&text);
    if candidate.is_empty() || candidate.contains(is_whitespace_or_newline) {
        return None;
    }
    let url = Url::from_string(candidate)?;
    url.scheme().is_some_and(|scheme| !scheme.is_empty()).then_some(url)
}

/// how long until the expiration date, a date already past expires on the next tick
pub fn expiry_delay(expiration_date: Date, now: Date) -> Duration {
    let seconds = expiration_date.time_interval_since(&now);
    if seconds.is_finite() && seconds > 0.0 {
        Duration::from_secs_f64(seconds)
    } else {
        Duration::ZERO
    }
}

/// true when the clipboard still holds what the app wrote
/// another copy since then is the users and expiring must not erase it
pub fn still_holds(current: Option<&ClipboardItem>, written: &ClipboardItem) -> bool {
    current.is_some_and(|current| current.entries() == written.entries())
}

/// what an expired clipboard holds, no text at all
pub fn expired_item() -> ClipboardItem {
    ClipboardItem::new_string(String::new())
}
