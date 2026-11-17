use std::path::PathBuf;

use gpui::{Bounds, Pixels};
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// one thing handed to the system share sheet
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ShareItem {
    Link(String),
    Text(String),
    File(PathBuf),
}

/// why the sheet wasnt shown, each variant has a working answer in ShareOutcome
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum ShareUnavailable {
    #[error("the desktop has no system share sheet")]
    NoSystemSheet,
    #[error("system sharing is not written for {0:?}")]
    BranchNotWritten(HostOs),
    #[error("nothing left to share after empty items were dropped")]
    NothingToShare,
    #[error("{} is not an absolute UTF-8 path to share", .0.display())]
    UnusableFile(PathBuf),
    #[error("the share sheet was requested off the main thread")]
    NotOnMainThread,
    #[error("the window exposes no native view to anchor the sheet to")]
    NoNativeView,
}

/// default answer of an os layer that doesnt override ShareSheet present
pub const fn unavailable_by_default(host: HostOs) -> ShareUnavailable {
    match host {
        HostOs::Linux => ShareUnavailable::NoSystemSheet,
        HostOs::MacOs | HostOs::Windows => ShareUnavailable::BranchNotWritten(host),
    }
}

/// a validated share, at least one non empty item and no duplicates
/// anchor is in window coordinates
#[derive(Clone, PartialEq, Debug)]
pub struct ShareRequest {
    items: Vec<ShareItem>,
    anchor: Bounds<Pixels>,
}

impl ShareRequest {
    /// anchor is the control that opened the sheet, in window pixels with origin top left
    pub fn new(items: Vec<ShareItem>, anchor: Bounds<Pixels>) -> Result<Self, ShareUnavailable> {
        let mut kept: Vec<ShareItem> = Vec::with_capacity(items.len());
        for item in items {
            let Some(item) = normalized(item)? else {
                continue;
            };
            if !kept.contains(&item) {
                kept.push(item);
            }
        }
        if kept.is_empty() {
            return Err(ShareUnavailable::NothingToShare);
        }
        Ok(Self {
            items: kept,
            anchor,
        })
    }

    pub fn items(&self) -> &[ShareItem] {
        &self.items
    }

    pub fn anchor(&self) -> Bounds<Pixels> {
        self.anchor
    }

    /// what gets copied to the clipboard when no sheet shows, links and text one per line
    pub fn clipboard_text(&self) -> Option<String> {
        let lines: Vec<&str> = self
            .items
            .iter()
            .filter_map(|item| match item {
                ShareItem::Link(text) | ShareItem::Text(text) => Some(text.as_str()),
                ShareItem::File(_) => None,
            })
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

fn normalized(item: ShareItem) -> Result<Option<ShareItem>, ShareUnavailable> {
    Ok(match item {
        ShareItem::Link(link) if link.is_empty() => None,
        ShareItem::Link(link) if is_web_link(&link) => Some(ShareItem::Link(link)),
        ShareItem::Link(link) => Some(ShareItem::Text(link)),
        ShareItem::Text(text) => (!text.is_empty()).then_some(ShareItem::Text(text)),
        ShareItem::File(path) => {
            if !path.is_absolute() || path.to_str().is_none() {
                return Err(ShareUnavailable::UnusableFile(path));
            }
            Some(ShareItem::File(path))
        }
    })
}

fn is_web_link(text: &str) -> bool {
    let lower = text.get(..8).unwrap_or(text).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// what the feature does after asking for the sheet
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ShareOutcome {
    Presented,
    Unavailable {
        reason: ShareUnavailable,
        copy: Option<String>,
    },
}

impl ShareOutcome {
    pub fn support(&self) -> PlatformSupport {
        match self {
            ShareOutcome::Presented => PlatformSupport::Performed,
            ShareOutcome::Unavailable { .. } => PlatformSupport::Unsupported,
        }
    }
}

/// turns the os layer answer into the next step for the feature
pub fn outcome_of(shown: Result<(), ShareUnavailable>, request: &ShareRequest) -> ShareOutcome {
    match shown {
        Ok(()) => ShareOutcome::Presented,
        Err(reason) => ShareOutcome::Unavailable {
            reason,
            copy: request.clipboard_text(),
        },
    }
}

/// how a share ended, succeeded only when the chosen service said it shared the items
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ShareFinished {
    pub succeeded: bool,
}

/// runs on_finished exactly once
/// dropped unfinished it reports succeeded false so the feature always hears back
pub struct ShareCompletion(Option<Box<dyn FnOnce(ShareFinished)>>);

impl ShareCompletion {
    pub fn new(on_finished: impl FnOnce(ShareFinished) + 'static) -> Self {
        Self(Some(Box::new(on_finished)))
    }

    pub fn finish(mut self, succeeded: bool) {
        self.run(succeeded);
    }

    fn run(&mut self, succeeded: bool) {
        if let Some(on_finished) = self.0.take() {
            on_finished(ShareFinished { succeeded });
        }
    }
}

impl Drop for ShareCompletion {
    fn drop(&mut self) {
        self.run(false);
    }
}

/// answer of an os layer with no sheet, completion runs now as unsuccessful and reason goes back
pub fn refuse(
    completion: ShareCompletion,
    reason: ShareUnavailable,
) -> Result<(), ShareUnavailable> {
    completion.finish(false);
    Err(reason)
}

/// what another app selection hands to this one through an os service
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct IncomingShare {
    pub files: Vec<PathBuf>,
    pub texts: Vec<String>,
}

impl IncomingShare {
    /// adds one item, a file wins over text since its path is its text form
    /// blank text is nothing
    pub fn push_item(&mut self, file: Option<PathBuf>, text: Option<String>) {
        if let Some(file) = file {
            self.files.push(file);
        } else if let Some(text) = text.filter(|text| !text.chars().all(char::is_whitespace)) {
            self.texts.push(text);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.texts.is_empty()
    }
}

/// the os layer, hosts that dont override present answer unavailable_by_default
pub trait ShareSheet {
    /// shows the sheet and holds completion until the user shares or dismisses, or refuses via refuse
    fn present(
        &self,
        _window: &gpui::Window,
        _request: &ShareRequest,
        completion: ShareCompletion,
    ) -> Result<(), ShareUnavailable> {
        refuse(completion, unavailable_by_default(HostOs::current()))
    }

    fn register_open_with(&self, _cx: &gpui::App) -> PlatformSupport {
        PlatformSupport::Unsupported
    }

    fn on_incoming_share(&self, _callback: Box<dyn Fn(IncomingShare)>) -> PlatformSupport {
        PlatformSupport::Unsupported
    }
}

pub fn register_open_with(cx: &gpui::App) -> PlatformSupport {
    super::os::NativeShare.register_open_with(cx)
}

pub fn on_incoming_share(callback: impl Fn(IncomingShare) + 'static) -> PlatformSupport {
    super::os::NativeShare.on_incoming_share(Box::new(callback))
}

/// shows the share sheet next to the anchor or says what to do instead
/// call from on_mouse_down not on_click since appkit wants mouse down
/// on_finished runs once on the main thread outside gpui, even before share returns when no sheet shows
pub fn share(
    window: &gpui::Window,
    request: &ShareRequest,
    on_finished: impl FnOnce(ShareFinished) + 'static,
) -> ShareOutcome {
    let completion = ShareCompletion::new(on_finished);
    if crate::open_document::uses_tester_receiver() {
        tracing::warn!(target: "riseonly", items = request.items().len(), "X1981 share handed to the tester receiver");
        completion.finish(false);
        return ShareOutcome::Presented;
    }
    outcome_of(super::os::NativeShare.present(window, request, completion), request)
}

/// the anchor in the native view own coordinates
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ViewRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub below_is_min_y: bool,
}

/// gpui measures from the top left but an unflipped appkit view measures y from the bottom
pub fn anchor_in_view(anchor: Bounds<Pixels>, view_height: f64, flipped: bool) -> ViewRect {
    let top = f64::from(anchor.origin.y);
    let height = f64::from(anchor.size.height).max(1.0);
    ViewRect {
        x: f64::from(anchor.origin.x),
        y: if flipped {
            top
        } else {
            view_height - top - height
        },
        width: f64::from(anchor.size.width).max(1.0),
        height,
        below_is_min_y: !flipped,
    }
}
