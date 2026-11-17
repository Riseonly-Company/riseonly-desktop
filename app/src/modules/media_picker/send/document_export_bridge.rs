use std::path::{Path, PathBuf};

use gpui::{AnyWindowHandle, App, Task};
use rise_i18n::tr;
use rise_platform::file_dialog::{self, FileDialogError, FileFilter, OpenRequest, Selection};
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::uniform_type::UTType;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};

const LOG_NAME: &str = "DocumentExportBridge";

/// completion with the picked paths, the app context stands in for the main queue
pub type DocumentExportBridgeCompletion = Box<dyn FnOnce(Vec<PathBuf>, &mut App)>;

/// desktop only, the system dialog refused to open
/// gets the text to show
pub type DocumentExportBridgeFailure = Box<dyn FnOnce(String, &mut App)>;

/// system document picker, opens files and hands back their paths once the dialog closes
/// cancel or refusal gives an empty list so the completion always runs
/// a refusal is also reported so a button whose dialog cant open never stays silent
pub struct DocumentExportBridge {
    content_types: Vec<UTType>,
    allows_multiple_selection: bool,
    file_filter: FileFilter,
    completion: DocumentExportBridgeCompletion,
    on_failure: Option<DocumentExportBridgeFailure>,
}

impl DocumentExportBridge {
    /// defaults are content types none meaning every document kind
    /// and multiple selection true
    pub fn new(
        content_types: Option<Vec<UTType>>,
        allows_multiple_selection: bool,
        completion: DocumentExportBridgeCompletion,
    ) -> Self {
        Self {
            content_types: content_types.unwrap_or_else(|| {
                vec![
                    UTType::item(),
                    UTType::image(),
                    UTType::movie(),
                    UTType::pdf(),
                    UTType::text(),
                    UTType::audio(),
                    UTType::data(),
                ]
            }),
            allows_multiple_selection,
            file_filter: FileFilter::any(),
            completion,
            on_failure: None,
        }
    }

    /// desktop only, what the dialog itself offers before content_types narrows the pick
    pub fn file_filter(mut self, file_filter: FileFilter) -> Self {
        self.file_filter = file_filter;
        self
    }

    /// desktop only, who shows a refusal of the dialog
    /// without it the notifier does
    pub fn on_failure(mut self, on_failure: DocumentExportBridgeFailure) -> Self {
        self.on_failure = Some(on_failure);
        self
    }

    pub fn present(self, from: AnyWindowHandle, cx: &mut App) {
        self.present_owned(from, cx).detach();
    }

    /// desktop only, present with the wait owned by the caller
    /// dropping the task drops the wait so a closed picker no longer holds an open panel
    pub fn present_owned(self, from: AnyWindowHandle, cx: &mut App) -> Task<()> {
        let _ = from.update(cx, |_, window, _| window.activate_window());
        let request = OpenRequest::files(self.file_filter).allowing_multiple(self.allows_multiple_selection);
        let selection = file_dialog::open(cx, &request);
        let content_types = self.content_types;
        let completion = self.completion;
        let on_failure = self.on_failure;
        cx.spawn(async move |cx| {
            let (urls, failure) = match selection.await {
                Selection::Picked(picked) => (
                    picked
                        .accepted
                        .into_iter()
                        .filter(|path| Self::conforms(path, &content_types))
                        .collect::<Vec<_>>(),
                    None,
                ),
                Selection::Cancelled => (Vec::new(), None),
                Selection::Failed(error) => (Vec::new(), Some(error)),
            };
            cx.update(|cx| {
                completion(urls, cx);
                if let Some(error) = failure {
                    Self::report_failure(&error, on_failure, cx);
                }
            });
        })
    }

    fn report_failure(error: &FileDialogError, on_failure: Option<DocumentExportBridgeFailure>, cx: &mut App) {
        let detail = error.to_string();
        Sai::log(LOG_NAME, move || detail, SaiLogKind::Debug, cx);
        let message = tr("desktop_file_dialog_unavailable");
        if let Some(on_failure) = on_failure {
            on_failure(message, cx);
            return;
        }
        notification(NotificationKind::Error);
        Notifier::send(
            message,
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn conforms(path: &Path, content_types: &[UTType]) -> bool {
        let declared = path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(UTType::from_filename_extension);
        content_types.iter().any(|content_type| match &declared {
            Some(declared) => declared.conforms_to(content_type),
            None => *content_type == UTType::item(),
        })
    }
}
