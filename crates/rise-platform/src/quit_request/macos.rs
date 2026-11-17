use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, define_class, msg_send, sel};
use objc2_app_kit::NSApplication;

use crate::gpui_shim::PlatformSupport;

const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const QUIT_APPLICATION: u32 = u32::from_be_bytes(*b"quit");
const QUIT_REASON: u32 = u32::from_be_bytes(*b"why?");

struct QuitEventHandlerIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "RiseQuitEventHandler"]
    #[ivars = QuitEventHandlerIvars]
    struct QuitEventHandler;

    impl QuitEventHandler {
        #[unsafe(method(handleQuitEvent:withReplyEvent:))]
        fn handle_quit_event(&self, event: &AnyObject, _reply: &AnyObject) {
            let reason: Option<Retained<AnyObject>> =
                unsafe { msg_send![event, attributeDescriptorForKeyword: QUIT_REASON] };
            if reason.is_none() {
                (self.ivars().callback)();
                return;
            }
            if let Some(mtm) = MainThreadMarker::new() {
                let app = NSApplication::sharedApplication(mtm);
                let sender: Option<&AnyObject> = None;
                unsafe {
                    let _: () = msg_send![&*app, terminate: sender];
                }
            }
        }
    }
);

thread_local! {
    static HANDLER: RefCell<Option<Retained<QuitEventHandler>>> = const { RefCell::new(None) };
}

pub(super) fn install(callback: Box<dyn Fn()>) -> PlatformSupport {
    if MainThreadMarker::new().is_none() {
        return PlatformSupport::Unsupported;
    }
    let Some(manager_class) = AnyClass::get(c"NSAppleEventManager") else {
        return PlatformSupport::Unsupported;
    };
    let handler = QuitEventHandler::alloc().set_ivars(QuitEventHandlerIvars { callback });
    let handler: Retained<QuitEventHandler> = unsafe { msg_send![super(handler), init] };
    let handler_object: &AnyObject = &handler;
    unsafe {
        let manager: Retained<AnyObject> = msg_send![manager_class, sharedAppleEventManager];
        let _: () = msg_send![
            &*manager,
            setEventHandler: handler_object,
            andSelector: sel!(handleQuitEvent:withReplyEvent:),
            forEventClass: CORE_EVENT_CLASS,
            andEventID: QUIT_APPLICATION
        ];
    }
    HANDLER.with(|slot| *slot.borrow_mut() = Some(handler));
    PlatformSupport::Performed
}
