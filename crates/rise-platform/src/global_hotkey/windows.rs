use std::sync::OnceLock;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc;

use ::windows::Win32::Foundation::{LPARAM, WPARAM};
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP, WM_HOTKEY,
};
use parking_lot::Mutex;

use super::global_hotkey::{Hotkey, HotkeyError};
use crate::host_os::HostOs;

const MAX_PENDING: usize = 64;

const LAST_SLOT: i32 = 0xBFFF;

const WAKE: u32 = WM_APP + 1;

const SLOTS_EXHAUSTED: i32 = 87;

const WIN32_FACILITY_PREFIX: u32 = 0x8007;

static SLOTS: Mutex<Vec<(i32, String)>> = Mutex::new(Vec::new());
static PRESSED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static NEXT_SLOT: AtomicI32 = AtomicI32::new(1);
static WORKER: OnceLock<Result<Worker, i32>> = OnceLock::new();

enum Request {
    Register {
        slot: i32,
        modifiers: u32,
        key: u32,
        reply: mpsc::SyncSender<Result<(), i32>>,
    },
    Release {
        slot: i32,
    },
}

struct Worker {
    thread_id: u32,
    requests: Mutex<mpsc::Sender<Request>>,
}

impl Worker {
    fn start() -> Result<Self, i32> {
        let (requests, inbox) = mpsc::channel::<Request>();
        let (ready, started) = mpsc::sync_channel::<u32>(1);

        std::thread::Builder::new()
            .name("rise-hotkey-win".into())
            .spawn(move || serve(&inbox, &ready))
            .map_err(|error| error.raw_os_error().unwrap_or(-1))?;

        let thread_id = started.recv().map_err(|_| -1)?;
        Ok(Self {
            thread_id,
            requests: Mutex::new(requests),
        })
    }

    fn send(&self, request: Request) -> bool {
        if self.requests.lock().send(request).is_err() {
            return false;
        }

        unsafe { PostThreadMessageW(self.thread_id, WAKE, WPARAM(0), LPARAM(0)) }.is_ok()
    }
}

fn serve(inbox: &mpsc::Receiver<Request>, ready: &mpsc::SyncSender<u32>) {
    let mut message = MSG::default();

    unsafe {
        let _ = PeekMessageW(&raw mut message, None, 0, 0, PM_NOREMOVE);
    }

    let thread_id = unsafe { GetCurrentThreadId() };
    if ready.send(thread_id).is_err() {
        return;
    }

    loop {
        let status = unsafe { GetMessageW(&raw mut message, None, 0, 0) };

        if status.0 <= 0 {
            tracing::warn!(target: "riseonly", status = status.0, "global hotkeys: the Windows hotkey thread stopped");
            return;
        }

        match message.message {
            WM_HOTKEY => queue_press(message.wParam.0),
            WAKE => drain(inbox),
            _ => {}
        }
    }
}

fn drain(inbox: &mpsc::Receiver<Request>) {
    while let Ok(request) = inbox.try_recv() {
        match request {
            Request::Register {
                slot,
                modifiers,
                key,
                reply,
            } => {
                let outcome = unsafe {
                    RegisterHotKey(None, slot, HOT_KEY_MODIFIERS(modifiers) | MOD_NOREPEAT, key)
                };
                let _ = reply.send(outcome.map_err(|error| win32_code(error.code().0)));
            }
            Request::Release { slot } => {
                let _ = unsafe { UnregisterHotKey(None, slot) };
            }
        }
    }
}

fn queue_press(raw_slot: usize) {
    let Ok(slot) = i32::try_from(raw_slot) else {
        return;
    };

    let action = SLOTS
        .lock()
        .iter()
        .find(|(existing, _)| *existing == slot)
        .map(|(_, action)| action.clone());

    if let Some(action) = action {
        let mut pressed = PRESSED.lock();
        if pressed.len() >= MAX_PENDING {
            pressed.remove(0);
        }
        pressed.push(action);
    }
}

fn win32_code(hresult: i32) -> i32 {
    let bits = hresult as u32;
    if bits >> 16 == WIN32_FACILITY_PREFIX {
        (bits & 0xFFFF) as i32
    } else {
        hresult
    }
}

pub(super) struct Registration {
    action: String,
    slot: i32,
}

impl Registration {
    pub(super) fn action(&self) -> &str {
        &self.action
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        SLOTS.lock().retain(|(slot, _)| *slot != self.slot);
        if let Some(Ok(worker)) = WORKER.get() {
            worker.send(Request::Release { slot: self.slot });
        }
    }
}

pub(super) fn register(action: &str, hotkey: Hotkey) -> Result<Option<Registration>, HotkeyError> {
    let worker = match WORKER.get_or_init(Worker::start) {
        Ok(worker) => worker,
        Err(code) => return Err(HotkeyError::HandlerUnavailable(*code)),
    };

    let refused = |code| HotkeyError::Refused {
        chord: hotkey.display(HostOs::Windows),
        code,
    };

    let slot = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
    if slot > LAST_SLOT {
        return Err(refused(SLOTS_EXHAUSTED));
    }
    SLOTS.lock().push((slot, action.to_owned()));

    let (reply, answer) = mpsc::sync_channel(1);
    let request = Request::Register {
        slot,
        modifiers: hotkey.modifier_mask(HostOs::Windows),
        key: hotkey.native_key_code(HostOs::Windows),
        reply,
    };

    let outcome = if worker.send(request) {
        answer.recv().unwrap_or(Err(-1))
    } else {
        Err(-1)
    };

    match outcome {
        Ok(()) => Ok(Some(Registration {
            action: action.to_owned(),
            slot,
        })),
        Err(-1) => {
            SLOTS.lock().retain(|(existing, _)| *existing != slot);
            Err(HotkeyError::HandlerUnavailable(-1))
        }
        Err(code) => {
            SLOTS.lock().retain(|(existing, _)| *existing != slot);
            Err(refused(code))
        }
    }
}

pub(super) fn take_pressed() -> Vec<String> {
    std::mem::take(&mut *PRESSED.lock())
}
