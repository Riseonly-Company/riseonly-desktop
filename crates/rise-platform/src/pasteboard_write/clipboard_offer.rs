use crate::gpui_shim::compositor_is_wayland;

pub const PASSWORD_MANAGER_HINT_MIME: &str = "x-kde-passwordManagerHint";

pub const PASSWORD_MANAGER_HINT_SECRET: &[u8] = b"secret";

pub const URI_LIST_MIME: &str = "text/uri-list";

pub const TEXT_MIMES: [&str; 4] = [
    "text/plain;charset=utf-8",
    "text/plain;charset=UTF-8",
    "UTF8_STRING",
    "text/plain",
];

pub const NO_DATA_CONTROL: &str = "the Wayland compositor offers neither ext_data_control_manager_v1 \
     nor zwlr_data_control_manager_v1, and gpui offers only its own text types";

pub const NO_DISPLAY: &str = "neither WAYLAND_DISPLAY nor DISPLAY names a display server";

/// the selection one write offers, every mime type in the order a reader should prefer
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ClipboardOffer {
    entries: Vec<(&'static str, Vec<u8>)>,
}

impl ClipboardOffer {
    /// this device only text, put under every gpui text type plus the password manager hint
    pub fn host_only_text(text: &str) -> Self {
        let mut entries = text_entries(text);
        entries.push((PASSWORD_MANAGER_HINT_MIME, PASSWORD_MANAGER_HINT_SECRET.to_vec()));
        Self { entries }
    }

    /// a link as a one line uri list first, then as text for text fields
    pub fn url(absolute_string: &str) -> Self {
        let mut entries = vec![(URI_LIST_MIME, uri_list_body(absolute_string))];
        entries.extend(text_entries(absolute_string));
        Self { entries }
    }

    /// offered mime types in order
    pub fn mime_types(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.entries.iter().map(|(mime, _)| *mime)
    }

    /// bytes a reader gets for the mime, none for a type this selection doesnt offer
    pub fn payload(&self, mime: &str) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|(offered, _)| *offered == mime)
            .map(|(_, bytes)| bytes.as_slice())
    }
}

fn text_entries(text: &str) -> Vec<(&'static str, Vec<u8>)> {
    TEXT_MIMES
        .iter()
        .map(|mime| (*mime, text.as_bytes().to_vec()))
        .collect()
}

pub fn uri_list_body(absolute_string: &str) -> Vec<u8> {
    let mut body = absolute_string.as_bytes().to_vec();
    body.extend_from_slice(b"\r\n");
    body
}

/// display protocol the layer writes the clipboard through
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClipboardSession {
    Wayland,
    X11,
    Unreachable,
}

/// same choice as gpui guess_compositor, headless first then a non empty wayland display
/// so it writes the clipboard of the display gpui reads
pub const fn clipboard_session(
    headless: bool,
    wayland_display_set: bool,
    x_display_set: bool,
) -> ClipboardSession {
    if compositor_is_wayland(headless, wayland_display_set) {
        ClipboardSession::Wayland
    } else if !headless && x_display_set {
        ClipboardSession::X11
    } else {
        ClipboardSession::Unreachable
    }
}
