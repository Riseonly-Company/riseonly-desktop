//! Outbound file drag (Riseonly patch 0003): `PlatformWindow::start_external_drag` on Windows.
//!
//! gpui promotes an in-window drag once the pressed pointer leaves the viewport and asks the
//! platform to take the gesture over (gpui window.rs promote_external_drag_to_platform). macOS
//! starts an asynchronous NSDraggingSession and later reports `FileDropEvent::Ended`; this file
//! gives the same contract on Windows with an OLE drag of `CF_HDROP`.

use std::{mem::ManuallyDrop, os::windows::ffi::OsStrExt, path::PathBuf};

use gpui::{ExternalDragPayload, FileDropEvent, PlatformInput};
use gpui_util::ResultExt;
use windows::{
    Win32::{
        Foundation::{GlobalFree, HWND, LPARAM, POINT, WPARAM},
        System::{
            Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL},
            Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{CF_HDROP, DROPEFFECT_COPY, IDropSource},
        },
        UI::{
            Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, VK_LBUTTON},
            Shell::{DROPFILES, SHCreateDataObject, SHDoDragDrop},
            WindowsAndMessaging::{PostMessageW, WM_USER},
        },
    },
    core::Result,
};

use crate::WindowsWindowInner;

/// Posted to the window to run the OLE drag from the top of its message loop.
pub(crate) const WM_GPUI_START_EXTERNAL_DRAG: u32 = WM_USER + 9;
/// Posted again when `FileDropEvent::Ended` could not be delivered because the input callback
/// was in use by an outer dispatch.
pub(crate) const WM_GPUI_EXTERNAL_DRAG_ENDED: u32 = WM_USER + 10;

impl WindowsWindowInner {
    /// Takes the drag over from gpui. The OLE drag itself starts on the next turn of the message
    /// loop, so the call returns at once like the macOS dragging session does.
    pub(crate) fn queue_external_drag(&self, hwnd: HWND, payload: &ExternalDragPayload) -> bool {
        let ExternalDragPayload::Files(paths) = payload;
        let paths: Vec<PathBuf> = paths.entries().iter().map(|(path, _)| path.clone()).collect();
        if paths.is_empty() {
            log::warn!("start_external_drag declined: no paths");
            return false;
        }
        let mut pending = self.state.pending_external_drag.borrow_mut();
        if pending.is_some() {
            log::warn!("start_external_drag declined: a drag is already queued");
            return false;
        }
        // why: DoDragDrop runs a modal loop until the drop. Started here, inside gpui's input
        // dispatch, the window's input callback is taken (drops onto our own window would be
        // lost) and the App stays borrowed for the whole drag, so it starts from a posted message.
        // https://learn.microsoft.com/windows/win32/api/ole2/nf-ole2-dodragdrop
        // SAFETY: hwnd is this live window; PostMessageW only queues the message.
        match unsafe { PostMessageW(Some(hwnd), WM_GPUI_START_EXTERNAL_DRAG, WPARAM(0), LPARAM(0)) } {
            Ok(()) => {
                *pending = Some(paths);
                true
            }
            Err(error) => {
                log::warn!("start_external_drag declined: {error}");
                false
            }
        }
    }

    pub(crate) fn handle_start_external_drag(&self, handle: HWND) -> Option<isize> {
        let Some(paths) = self.state.pending_external_drag.borrow_mut().take() else {
            return Some(0);
        };
        // invariant: the drag only starts while the left button that began it is still held;
        // a release already queued before this message means the gesture is over.
        // SAFETY: GetKeyState reads the thread's input state for the message just retrieved.
        let left_button_held = unsafe { GetKeyState(VK_LBUTTON.0 as i32) } < 0;
        if left_button_held {
            match file_data_object(&paths) {
                Ok(data_object) => {
                    // SAFETY: the window owns the capture taken on mouse down; DoDragDrop
                    // captures the pointer itself for the drag.
                    unsafe { ReleaseCapture() }.log_err();
                    // why: SHDoDragDrop with a null IDropSource uses the shell's default source
                    // (Esc cancels, releasing the button drops) and builds the drag image from
                    // the shell data object.
                    // https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shdodragdrop
                    // SAFETY: called on the window's STA thread where OleInitialize ran
                    // (WindowsPlatform::new); data_object lives for the whole call.
                    unsafe {
                        SHDoDragDrop(
                            Some(handle),
                            &data_object,
                            None::<&IDropSource>,
                            DROPEFFECT_COPY,
                        )
                    }
                    .log_err();
                }
                Err(error) => log::warn!("start_external_drag failed: {error}"),
            }
        }
        self.send_external_drag_ended(handle);
        Some(0)
    }

    pub(crate) fn handle_external_drag_ended(&self, handle: HWND) -> Option<isize> {
        self.send_external_drag_ended(handle);
        Some(0)
    }

    /// Ends the platform-owned drag in gpui, as the macOS dragging_session_ended does.
    fn send_external_drag_ended(&self, handle: HWND) {
        if let Some(mut func) = self.state.callbacks.input.take() {
            func(PlatformInput::FileDrop(FileDropEvent::Ended));
            self.state.callbacks.input.set(Some(func));
            return;
        }
        // why: an outer dispatch holds the callback; gpui keeps the drag suspended until Ended
        // arrives, so the event is retried from the message loop instead of being dropped.
        // SAFETY: handle is this live window; PostMessageW only queues the message.
        unsafe { PostMessageW(Some(handle), WM_GPUI_EXTERNAL_DRAG_ENDED, WPARAM(0), LPARAM(0)) }
            .log_err();
    }
}

/// A shell data object carrying the paths as `CF_HDROP`.
/// why: SHCreateDataObject with no items gives an empty shell data object that accepts SetData
/// of any format and the drag image helper data, so no IDataObject of our own is needed.
/// https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shcreatedataobject
fn file_data_object(paths: &[PathBuf]) -> Result<IDataObject> {
    // SAFETY: no folder and no items are passed; the call only creates the object.
    let data_object: IDataObject = unsafe { SHCreateDataObject(None, None, None::<&IDataObject>) }?;
    let wide = hdrop_file_list(paths);
    let header = std::mem::size_of::<DROPFILES>();
    let size = header + wide.len() * std::mem::size_of::<u16>();
    // SAFETY: GMEM_MOVEABLE memory is locked before it is written and unlocked after; on success
    // SetData with fRelease takes ownership, on failure it is freed here.
    unsafe {
        let hglobal = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, size)?;
        let base = GlobalLock(hglobal) as *mut u8;
        if base.is_null() {
            GlobalFree(Some(hglobal)).log_err();
            return Err(windows::core::Error::from_win32());
        }
        // why: the file list follows the header, wide, double null terminated.
        // https://learn.microsoft.com/windows/win32/shell/clipboard#cf_hdrop
        base.cast::<DROPFILES>().write_unaligned(DROPFILES {
            pFiles: header as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: false.into(),
            fWide: true.into(),
        });
        std::ptr::copy_nonoverlapping(
            wide.as_ptr().cast::<u8>(),
            base.add(header),
            wide.len() * std::mem::size_of::<u16>(),
        );
        // why: GlobalUnlock reports failure once the lock count drops to zero, which is the
        // expected outcome here, so its result carries no error.
        let _ = GlobalUnlock(hglobal);
        let format = FORMATETC {
            cfFormat: CF_HDROP.0,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        let medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: hglobal },
            pUnkForRelease: ManuallyDrop::new(None),
        };
        if let Err(error) = data_object.SetData(&format, &medium, true) {
            GlobalFree(Some(hglobal)).log_err();
            return Err(error);
        }
    }
    Ok(data_object)
}

/// Each path as UTF-16 with its null, then the closing null of the list.
fn hdrop_file_list(paths: &[PathBuf]) -> Vec<u16> {
    let mut wide = Vec::new();
    for path in paths {
        wide.extend(path.as_os_str().encode_wide());
        wide.push(0);
    }
    wide.push(0);
    wide
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_list_is_double_null_terminated() {
        let wide = hdrop_file_list(&[PathBuf::from(r"C:\a.png"), PathBuf::from(r"C:\b")]);
        let expected: Vec<u16> = "C:\\a.png\0C:\\b\0\0".encode_utf16().collect();
        assert_eq!(wide, expected);
    }
}
