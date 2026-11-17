use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, Context, Entity, FocusHandle, Render, Subscription, Window, div, prelude::*};
use rise_core::foundation::foundation_url::Url;
use rise_i18n::{FormatArg, tr, tr_args};

use crate::app::shell::file_drop::{self, DropOutcome, DropRejection};
use crate::app::shell::handover;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::share_extension::entry::share_item_loader::{ShareExtensionItem, ShareItemProvider};
use crate::modules::share_extension::entry::share_view_controller::ShareViewController;
use crate::modules::system_share::desktop_share_intake_hooks::desktop_share_intake_hooks;

pub use crate::app::shell::desktop_share_arguments_hooks::{
    DesktopShareArguments, split_launch_arguments, split_opened_urls,
};

const WEB_SCHEMES: [&str; 2] = ["http", "https"];

/// share items of files, one item holding every provider like a multi selection share
pub fn items_for_files(paths: &[PathBuf]) -> Vec<ShareExtensionItem> {
    let attachments = paths
        .iter()
        .map(|path| ShareItemProvider::File {
            path: path.clone(),
            suggested_name: path.file_name().and_then(|name| name.to_str()).map(str::to_owned),
        })
        .collect();
    vec![ShareExtensionItem { attachments: Some(attachments), attributed_content_text: None }]
}

/// a link is shared as text, the loader tells a web link from plain text by content
pub fn items_for_text(text: &str) -> Vec<ShareExtensionItem> {
    vec![ShareExtensionItem {
        attachments: Some(vec![ShareItemProvider::Text(text.to_owned())]),
        attributed_content_text: None,
    }]
}

/// canon limit of file_drop and its file check, in share items
pub fn accepted_files(
    paths: &[PathBuf],
    is_file: impl Fn(&Path) -> bool,
) -> Result<Vec<ShareExtensionItem>, DropRejection> {
    match file_drop::evaluate(paths, is_file) {
        DropOutcome::Accepted(paths) => Ok(items_for_files(&paths)),
        DropOutcome::Rejected(rejection) => Err(rejection),
    }
}

/// files dropped on the window outside the composer or opened from outside,
/// a refusal is told like the composer tells it
pub fn accept_files(paths: &[PathBuf], cx: &mut App) -> Option<Vec<ShareExtensionItem>> {
    match accepted_files(paths, |path| path.is_file()) {
        Ok(items) => Some(items),
        Err(DropRejection::Empty) => None,
        Err(DropRejection::TooMany { limit, .. }) => {
            let title = tr_args("share_error_too_many_items", &[FormatArg::Int(limit as i64)]);
            Notifier::send(title, NotifierSendOptions::default(), cx);
            None
        }
        Err(DropRejection::NotAFile(_)) => {
            Notifier::send(tr("sticker_pack_media_unsupported"), NotifierSendOptions::default(), cx);
            None
        }
    }
}

/// a web link nothing in the app claimed,
/// a browser keeps those so one reaching the app was sent on purpose and goes on as text
pub fn is_shareable_link(url: &Url) -> bool {
    url.scheme().is_some_and(|scheme| WEB_SCHEMES.contains(&scheme))
}

/// opens the layer with what came from outside in the window that answers for the process
pub fn deliver_shared(files: &[PathBuf], texts: &[String], cx: &mut App) {
    let mut items = Vec::new();
    if !files.is_empty()
        && let Some(file_items) = accept_files(files, cx)
    {
        items.extend(file_items);
    }
    for text in texts {
        items.extend(items_for_text(text));
    }
    if items.is_empty() {
        return;
    }

    let Some(handle) = handover::answering_shell(cx) else {
        return;
    };
    if let Some(hooks) = desktop_share_intake_hooks() {
        (hooks.present_shared_items)(handle, items, cx);
    }
}

/// share layer of one window,
/// a share arriving while another is open waits its turn since the open sheet may be mid send
pub struct DesktopShareLayer {
    share_view_controller: Option<Entity<ShareViewController>>,
    pending_items: VecDeque<Vec<ShareExtensionItem>>,
    return_focus: Option<FocusHandle>,
    _auth_observer: Subscription,
}

impl DesktopShareLayer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let auth_observer = cx.observe(&auth, |_, _, cx| cx.notify());
        Self {
            share_view_controller: None,
            pending_items: VecDeque::new(),
            return_focus: None,
            _auth_observer: auth_observer,
        }
    }

    pub fn enqueue(&mut self, items: Vec<ShareExtensionItem>, cx: &mut Context<Self>) {
        self.pending_items.push_back(items);
        cx.notify();
    }

    fn present_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.share_view_controller.is_some() {
            return;
        }
        let Some(items) = self.pending_items.pop_front() else {
            return;
        };
        if self.return_focus.is_none() {
            self.return_focus = window.focused(cx);
        }
        let layer = cx.weak_entity();
        let on_dismiss: Rc<dyn Fn(&mut App)> = Rc::new(move |cx: &mut App| {
            let _ = layer.update(cx, |layer, cx| {
                layer.share_view_controller = None;
                cx.notify();
            });
        });
        self.share_view_controller = Some(cx.new(|cx| ShareViewController::new(items, on_dismiss, window, cx)));
        cx.notify();
    }
}

impl Render for DesktopShareLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.share_view_controller.is_none() {
            let auth = AuthServicesStore::shared(cx);
            if !self.pending_items.is_empty() && auth.read(cx).has_restored_persisted_session() {
                let layer = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = layer.update(cx, |layer, cx| layer.present_next(window, cx));
                });
            } else if let Some(return_focus) = self.return_focus.take() {
                window.focus(&return_focus, cx);
            }
        }
        match &self.share_view_controller {
            Some(controller) => div().size_full().child(controller.clone()),
            None => div(),
        }
    }
}
