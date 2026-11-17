use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use parking_lot::Mutex;

use super::global_hotkey::{Hotkey, HotkeyError};
use crate::host_os::HostOs;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {}

type OsStatus = i32;
type EventRef = *mut c_void;
type EventTargetRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;
type EventHotKeyRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

type EventHandlerUpp =
    unsafe extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OsStatus;

unsafe extern "C" {
    #[link_name = "GetApplicationEventTarget"]
    fn get_application_event_target() -> EventTargetRef;

    #[link_name = "InstallEventHandler"]
    fn install_event_handler(
        target: EventTargetRef,
        handler: EventHandlerUpp,
        num_types: usize,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OsStatus;

    #[link_name = "RegisterEventHotKey"]
    fn register_event_hot_key(
        key_code: u32,
        modifiers: u32,
        hot_key_id: EventHotKeyId,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OsStatus;

    #[link_name = "UnregisterEventHotKey"]
    fn unregister_event_hot_key(hot_key: EventHotKeyRef) -> OsStatus;

    #[link_name = "GetEventParameter"]
    fn get_event_parameter(
        event: EventRef,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> OsStatus;
}

const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

const NO_ERR: OsStatus = 0;
const EVENT_CLASS_KEYBOARD: u32 = fourcc(b"keyb");
const EVENT_HOT_KEY_PRESSED: u32 = 5;
const PARAM_DIRECT_OBJECT: u32 = fourcc(b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = fourcc(b"hkid");

const SIGNATURE: u32 = fourcc(b"rise");

const NO_OPTIONS: u32 = 0;

const MAX_PENDING: usize = 64;

static SLOTS: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());
static PRESSED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static NEXT_SLOT: AtomicU32 = AtomicU32::new(1);
static HANDLER: OnceLock<OsStatus> = OnceLock::new();

unsafe extern "C" fn hot_key_pressed(
    _call: EventHandlerCallRef,
    event: EventRef,
    _user_data: *mut c_void,
) -> OsStatus {
    let mut id = EventHotKeyId {
        signature: 0,
        id: 0,
    };

    let status = unsafe {
        get_event_parameter(
            event,
            PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            std::ptr::null_mut(),
            size_of::<EventHotKeyId>(),
            std::ptr::null_mut(),
            (&raw mut id).cast(),
        )
    };

    if status == NO_ERR && id.signature == SIGNATURE {
        let action = SLOTS
            .lock()
            .iter()
            .find(|(slot, _)| *slot == id.id)
            .map(|(_, action)| action.clone());

        if let Some(action) = action {
            let mut pressed = PRESSED.lock();
            if pressed.len() >= MAX_PENDING {
                pressed.remove(0);
            }
            pressed.push(action);
        }
    }

    NO_ERR
}

fn ensure_handler() -> OsStatus {
    *HANDLER.get_or_init(|| {
        let spec = EventTypeSpec {
            event_class: EVENT_CLASS_KEYBOARD,
            event_kind: EVENT_HOT_KEY_PRESSED,
        };

        unsafe {
            install_event_handler(
                get_application_event_target(),
                hot_key_pressed,
                1,
                &raw const spec,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        }
    })
}

pub(super) struct Registration {
    action: String,
    slot: u32,
    handle: EventHotKeyRef,
}

impl Registration {
    pub(super) fn action(&self) -> &str {
        &self.action
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        unsafe {
            unregister_event_hot_key(self.handle);
        }
        SLOTS.lock().retain(|(slot, _)| *slot != self.slot);
    }
}

pub(super) fn register(action: &str, hotkey: Hotkey) -> Result<Option<Registration>, HotkeyError> {
    let status = ensure_handler();
    if status != NO_ERR {
        return Err(HotkeyError::HandlerUnavailable(status));
    }

    let slot = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
    SLOTS.lock().push((slot, action.to_owned()));

    let mut handle: EventHotKeyRef = std::ptr::null_mut();

    let status = unsafe {
        register_event_hot_key(
            hotkey.native_key_code(HostOs::MacOs),
            hotkey.modifier_mask(HostOs::MacOs),
            EventHotKeyId {
                signature: SIGNATURE,
                id: slot,
            },
            get_application_event_target(),
            NO_OPTIONS,
            &raw mut handle,
        )
    };

    if status != NO_ERR || handle.is_null() {
        SLOTS.lock().retain(|(existing, _)| *existing != slot);
        return Err(HotkeyError::Refused {
            chord: hotkey.display(HostOs::MacOs),
            code: status,
        });
    }

    Ok(Some(Registration {
        action: action.to_owned(),
        slot,
        handle,
    }))
}

pub(super) fn take_pressed() -> Vec<String> {
    std::mem::take(&mut *PRESSED.lock())
}
