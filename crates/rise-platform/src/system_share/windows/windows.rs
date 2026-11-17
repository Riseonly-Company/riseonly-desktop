use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::OnceLock;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::AppContext as _;

use crate::gpui_shim::PlatformSupport;
use crate::url_scheme::association_entries::{
    open_with_command_key, open_with_extensions_key, open_with_is_settled, open_with_values,
};
use crate::url_scheme::windows::{installed_executable, read_default_value, write_association_values};
use ::windows::ApplicationModel::DataTransfer::{
    DataPackage, DataRequest, DataRequestDeferral, DataRequestedEventArgs, DataTransferManager,
    ShareCompletedEventArgs,
};
use ::windows::Foundation::{TypedEventHandler, Uri};
use ::windows::Storage::{IStorageItem, StorageFile};
use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::Shell::IDataTransferManagerInterop;
use ::windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use ::windows::core::{HSTRING, IInspectable, Interface as _};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_collections::IIterable;

use crate::system_share::*;

const TITLE_LIMIT: usize = 80;

struct Sharing {
    id: u64,
    manager: DataTransferManager,
    token: i64,
    completion: Option<ShareCompletion>,
}

thread_local! {
    static SHARING: RefCell<Option<Sharing>> = const { RefCell::new(None) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

pub(crate) struct NativeShare;

impl ShareSheet for NativeShare {
    fn register_open_with(&self, cx: &gpui::App) -> PlatformSupport {
        let Ok(executable) = installed_executable() else {
            return PlatformSupport::Unsupported;
        };
        cx.background_spawn(async move {
            let command = read_default_value(&open_with_command_key());
            let extensions = read_default_value(&open_with_extensions_key());
            if open_with_is_settled(command.as_deref(), extensions.as_deref(), &executable) {
                return;
            }
            if let Err(error) = write_association_values(&open_with_values(&executable)) {
                tracing::warn!(%error, "the Open With registration was refused");
            }
        })
        .detach();
        PlatformSupport::Performed
    }

    fn present(
        &self,
        window: &gpui::Window,
        request: &ShareRequest,
        completion: ShareCompletion,
    ) -> Result<(), ShareUnavailable> {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return refuse(completion, ShareUnavailable::NoNativeView);
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return refuse(completion, ShareUnavailable::NoNativeView);
        };
        let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
        let on_window_thread =
            unsafe { GetWindowThreadProcessId(hwnd, None) == GetCurrentThreadId() };
        if !on_window_thread {
            return refuse(completion, ShareUnavailable::NotOnMainThread);
        }
        listen(window);

        if let Some(open) = SHARING.with(|slot| slot.borrow_mut().take()) {
            if let Err(error) = open.manager.RemoveDataRequested(open.token) {
                tracing::warn!(?error, "system_share: RemoveDataRequested failed");
            }
        }

        let manager = match manager_for(hwnd) {
            Ok(manager) => manager,
            Err(error) => {
                tracing::warn!(?error, "system_share: no DataTransferManager for the window");
                return refuse(completion, ShareUnavailable::NoSystemSheet);
            }
        };
        let id = NEXT_ID.with(|next| next.replace(next.get().wrapping_add(1)));
        let content = PackageContent::of(request);
        let handler = TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(
            move |_, args| {
                if is_current(id) {
                    fill(id, &args.ok()?.Request()?, &content)?;
                }
                Ok(())
            },
        );
        let token = match manager.DataRequested(&handler) {
            Ok(token) => token,
            Err(error) => {
                tracing::warn!(?error, "system_share: DataRequested registration failed");
                return refuse(completion, ShareUnavailable::NoSystemSheet);
            }
        };
        SHARING.with(|slot| {
            slot.replace(Some(Sharing {
                id,
                manager: manager.clone(),
                token,
                completion: Some(completion),
            }))
        });

        if let Err(error) = show(hwnd) {
            tracing::warn!(?error, "system_share: ShowShareUIForWindow failed");
            let sharing = SHARING.with(|slot| slot.borrow_mut().take());
            let completion = sharing.and_then(|mut sharing| {
                let _ = sharing.manager.RemoveDataRequested(sharing.token);
                sharing.completion.take()
            });
            return match completion {
                Some(completion) => refuse(completion, ShareUnavailable::NoSystemSheet),
                None => Err(ShareUnavailable::NoSystemSheet),
            };
        }
        Ok(())
    }
}

fn manager_for(hwnd: HWND) -> ::windows::core::Result<DataTransferManager> {
    let interop =
        ::windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()?;
    unsafe { interop.GetForWindow::<DataTransferManager>(hwnd) }
}

fn show(hwnd: HWND) -> ::windows::core::Result<()> {
    let interop =
        ::windows::core::factory::<DataTransferManager, IDataTransferManagerInterop>()?;
    unsafe { interop.ShowShareUIForWindow(hwnd) }
}

fn is_current(id: u64) -> bool {
    SHARING.with(|slot| slot.borrow().as_ref().is_some_and(|sharing| sharing.id == id))
}

/// how a package ended, raised on a winrt thread pool or rpc thread
struct Report {
    id: u64,
    succeeded: bool,
}

struct Reports {
    sender: mpsc::UnboundedSender<Report>,
    pending: parking_lot::Mutex<Option<mpsc::UnboundedReceiver<Report>>>,
}

fn reports() -> &'static Reports {
    static REPORTS: OnceLock<Reports> = OnceLock::new();
    REPORTS.get_or_init(|| {
        let (sender, receiver) = mpsc::unbounded();
        Reports {
            sender,
            pending: parking_lot::Mutex::new(Some(receiver)),
        }
    })
}

/// receiver on its way to the drain task
/// dropped before that it goes back to reports() for the next share
pub(crate) struct PendingReports(Option<mpsc::UnboundedReceiver<Report>>);

impl PendingReports {
    pub(crate) fn take() -> Option<Self> {
        reports().pending.lock().take().map(|receiver| Self(Some(receiver)))
    }

    fn into_receiver(mut self) -> Option<mpsc::UnboundedReceiver<Report>> {
        self.0.take()
    }
}

impl Drop for PendingReports {
    fn drop(&mut self) {
        if let Some(receiver) = self.0.take() {
            *reports().pending.lock() = Some(receiver);
        }
    }
}

fn listen(window: &gpui::Window) {
    let Some(pending) = PendingReports::take() else {
        return;
    };
    window.on_next_frame(move |_, cx| {
        let Some(mut receiver) = pending.into_receiver() else {
            return;
        };
        cx.spawn(async move |_cx| {
            while let Some(report) = receiver.next().await {
                finish(report.id, report.succeeded);
            }
        })
        .detach();
    });
}

/// called by package events on any thread
/// send only fails while no drain task runs, then the share reports unsuccessful on replace
fn report(id: u64, succeeded: bool) {
    if reports().sender.unbounded_send(Report { id, succeeded }).is_err() {
        tracing::debug!(id, "system_share: the outcome found no listener");
    }
}

/// runs the completion of share id once on the main thread
/// a later report for the same share finds it taken and does nothing
fn finish(id: u64, succeeded: bool) {
    let completion = SHARING.with(|slot| {
        let mut slot = slot.borrow_mut();
        let sharing = slot.as_mut().filter(|sharing| sharing.id == id)?;
        sharing.completion.take()
    });
    if let Some(completion) = completion {
        completion.finish(succeeded);
    }
}

/// what goes into the DataPackage, taken from the request before the sheet asks
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct PackageContent {
    pub(crate) title: String,
    pub(crate) web_link: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) files: Vec<PathBuf>,
}

impl PackageContent {
    pub(crate) fn of(request: &ShareRequest) -> Self {
        let web_link = request.items().iter().find_map(|item| match item {
            ShareItem::Link(link) => Some(link.clone()),
            _ => None,
        });
        let files = request
            .items()
            .iter()
            .filter_map(|item| match item {
                ShareItem::File(path) => Some(path.clone()),
                _ => None,
            })
            .collect();
        Self {
            title: share_title(request.items()),
            web_link,
            text: request.clipboard_text(),
            files,
        }
    }
}

/// sheet refuses a package with no title
/// file uses its name, link or text its first line cut to TITLE_LIMIT chars
pub(crate) fn share_title(items: &[ShareItem]) -> String {
    let file = items.iter().find_map(|item| match item {
        ShareItem::File(path) => Some(
            path.file_name()
                .and_then(|name| name.to_str())
                .map_or_else(|| path.display().to_string(), str::to_owned),
        ),
        _ => None,
    });
    if let Some(name) = file {
        return name;
    }
    let first = items
        .iter()
        .find_map(|item| match item {
            ShareItem::Link(text) | ShareItem::Text(text) => Some(text.as_str()),
            ShareItem::File(_) => None,
        })
        .unwrap_or_default();
    let line = first.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    if line.chars().count() <= TITLE_LIMIT {
        return line.to_owned();
    }
    let mut short: String = line.chars().take(TITLE_LIMIT - 1).collect();
    short.push('\u{2026}');
    short
}

fn fill(id: u64, request: &DataRequest, content: &PackageContent) -> ::windows::core::Result<()> {
    let data = request.Data()?;
    data.Properties()?.SetTitle(&HSTRING::from(content.title.as_str()))?;
    if let Some(link) = &content.web_link {
        if let Ok(uri) = Uri::CreateUri(&HSTRING::from(link.as_str())) {
            data.SetWebLink(&uri)?;
        }
    }
    if let Some(text) = &content.text {
        data.SetText(&HSTRING::from(text.as_str()))?;
    }
    report_end(id, &data);
    if !content.files.is_empty() {
        let deferral = request.GetDeferral()?;
        let files = content.files.clone();
        std::thread::Builder::new()
            .name("rise-share-files".into())
            .spawn(move || attach_files(&data, &deferral, &files))
            .map_err(::windows::core::Error::from)?;
    }
    Ok(())
}

fn report_end(id: u64, data: &DataPackage) {
    let completed = TypedEventHandler::<DataPackage, ShareCompletedEventArgs>::new(move |_, _| {
        report(id, true);
        Ok(())
    });
    if let Err(error) = data.ShareCompleted(&completed) {
        tracing::debug!(?error, "system_share: ShareCompleted not offered by this build");
    }
    let canceled = TypedEventHandler::<DataPackage, IInspectable>::new(move |_, _| {
        report(id, false);
        Ok(())
    });
    if let Err(error) = data.ShareCanceled(&canceled) {
        tracing::debug!(?error, "system_share: ShareCanceled not offered by this build");
    }
    let destroyed = TypedEventHandler::<DataPackage, IInspectable>::new(move |_, _| {
        report(id, false);
        Ok(())
    });
    if let Err(error) = data.Destroyed(&destroyed) {
        tracing::debug!(?error, "system_share: Destroyed registration failed");
    }
}

fn attach_files(data: &DataPackage, deferral: &DataRequestDeferral, files: &[PathBuf]) {
    let items: Vec<Option<IStorageItem>> = files
        .iter()
        .filter_map(|path| {
            let path = path.to_str()?;
            let lookup = StorageFile::GetFileFromPathAsync(&HSTRING::from(path)).ok()?;
            match lookup.join().and_then(|file| file.cast::<IStorageItem>()) {
                Ok(item) => Some(Some(item)),
                Err(error) => {
                    tracing::warn!(?error, path, "system_share: file is not shareable");
                    None
                }
            }
        })
        .collect();
    if !items.is_empty() {
        let items = IIterable::<IStorageItem>::from(items);
        if let Err(error) = data.SetStorageItems(&items, true) {
            tracing::warn!(?error, "system_share: SetStorageItems failed");
        }
    }
    if let Err(error) = deferral.Complete() {
        tracing::warn!(?error, "system_share: deferral Complete failed");
    }
}
