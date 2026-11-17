use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    define_class, msg_send, AnyThread as _, DefinedClass, MainThreadMarker, MainThreadOnly,
    Message as _,
};
use objc2_app_kit::{
    NSSharingService, NSSharingServiceDelegate, NSSharingServicePicker,
    NSSharingServicePickerDelegate, NSView,
};
use objc2_foundation::{NSArray, NSError, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
use std::path::PathBuf;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::system_share::*;
use crate::gpui_shim::PlatformSupport;

struct Picking {
    picker: Retained<NSSharingServicePicker>,
    delegate: Retained<ShareDelegate>,
}

thread_local! {
    static PICKING: RefCell<Option<Picking>> = const { RefCell::new(None) };
    static SHARING: RefCell<Vec<Retained<ShareDelegate>>> = const { RefCell::new(Vec::new()) };
}

struct DelegateIvars {
    completion: RefCell<Option<ShareCompletion>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "RiseShareDelegate"]
    #[ivars = DelegateIvars]
    struct ShareDelegate;

    unsafe impl NSObjectProtocol for ShareDelegate {}

    unsafe impl NSSharingServicePickerDelegate for ShareDelegate {
        #[unsafe(method_id(sharingServicePicker:delegateForSharingService:))]
        fn delegate_for_service(
            &self,
            _picker: &NSSharingServicePicker,
            _service: &NSSharingService,
        ) -> Option<Retained<ProtocolObject<dyn NSSharingServiceDelegate>>> {
            Some(ProtocolObject::from_retained(self.retain()))
        }

        #[unsafe(method(sharingServicePicker:didChooseSharingService:))]
        fn did_choose(
            &self,
            _picker: &NSSharingServicePicker,
            service: Option<&NSSharingService>,
        ) {
            let me: *const Self = self;
            let closed = PICKING.with(|slot| {
                let mut slot = slot.borrow_mut();
                let mine = slot.as_ref().is_some_and(|open| std::ptr::eq(&*open.delegate, me));
                if mine { slot.take() } else { None }
            });
            match service {
                None => self.finish(false),
                Some(_) => SHARING.with(|list| list.borrow_mut().push(self.retain())),
            }
            if let Some(closed) = closed {
                let _ = Retained::autorelease_ptr(closed.picker);
                let _ = Retained::autorelease_ptr(closed.delegate);
            }
        }
    }

    unsafe impl NSSharingServiceDelegate for ShareDelegate {
        #[unsafe(method(sharingService:didShareItems:))]
        fn did_share(&self, _service: &NSSharingService, _items: &NSArray) {
            self.finish(true);
        }

        #[unsafe(method(sharingService:didFailToShareItems:error:))]
        fn did_fail(&self, _service: &NSSharingService, _items: &NSArray, _error: &NSError) {
            self.finish(false);
        }
    }
);

impl ShareDelegate {
    fn new(mtm: MainThreadMarker, completion: ShareCompletion) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars {
            completion: RefCell::new(Some(completion)),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn finish(&self, succeeded: bool) {
        let completion = self.ivars().completion.borrow_mut().take();
        let me: *const Self = self;
        let held = SHARING.with(|list| {
            let mut list = list.borrow_mut();
            let index = list.iter().position(|kept| std::ptr::eq(&**kept, me))?;
            Some(list.swap_remove(index))
        });
        if let Some(completion) = completion {
            completion.finish(succeeded);
        }
        if let Some(held) = held {
            let _ = Retained::autorelease_ptr(held);
        }
    }
}

pub(super) struct NativeShare;

struct ServiceProviderIvars {
    callback: Box<dyn Fn(IncomingShare)>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "RiseShareServiceProvider"]
    #[ivars = ServiceProviderIvars]
    struct ServiceProvider;

    impl ServiceProvider {
        #[unsafe(method(shareToRiseonly:userData:error:))]
        fn share_to_riseonly(
            &self,
            pasteboard: &AnyObject,
            _user_data: Option<&AnyObject>,
            _error: *mut *mut NSString,
        ) {
            let incoming = read_incoming(pasteboard);
            if !incoming.is_empty() {
                (self.ivars().callback)(incoming);
            }
        }
    }
);

thread_local! {
    static SERVICE_PROVIDER: RefCell<Option<Retained<ServiceProvider>>> = const { RefCell::new(None) };
}

fn read_incoming(pasteboard: &AnyObject) -> IncomingShare {
    let mut incoming = IncomingShare::default();
    let file_url_type = NSString::from_str("public.file-url");
    let text_type = NSString::from_str("public.utf8-plain-text");
    let items: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![pasteboard, pasteboardItems] };
    let Some(items) = items else {
        return incoming;
    };
    for item in items.iter() {
        let (file_url, text): (Option<Retained<NSString>>, Option<Retained<NSString>>) = unsafe {
            (
                msg_send![&*item, stringForType: &*file_url_type],
                msg_send![&*item, stringForType: &*text_type],
            )
        };
        incoming.push_item(
            file_url.as_deref().and_then(path_of_file_url),
            text.map(|text| text.to_string()),
        );
    }
    incoming
}

fn path_of_file_url(url: &NSString) -> Option<PathBuf> {
    let path: Option<Retained<NSString>> = unsafe {
        let url: Option<Retained<NSURL>> = msg_send![objc2::class!(NSURL), URLWithString: url];
        let resolved: Option<Retained<NSURL>> = url.and_then(|url| msg_send![&*url, filePathURL]);
        resolved.and_then(|resolved| msg_send![&*resolved, path])
    };
    path.map(|path| PathBuf::from(path.to_string()))
}

impl ShareSheet for NativeShare {
    fn on_incoming_share(&self, callback: Box<dyn Fn(IncomingShare)>) -> PlatformSupport {
        let Some(mtm) = MainThreadMarker::new() else {
            return PlatformSupport::Unsupported;
        };
        let provider = ServiceProvider::alloc(mtm).set_ivars(ServiceProviderIvars { callback });
        let provider: Retained<ServiceProvider> = unsafe { msg_send![super(provider), init] };
        let provider_object: &AnyObject = &provider;
        unsafe {
            let _: () = msg_send![
                &*objc2_app_kit::NSApplication::sharedApplication(mtm),
                setServicesProvider: provider_object
            ];
        }
        SERVICE_PROVIDER.with(|slot| *slot.borrow_mut() = Some(provider));
        PlatformSupport::Performed
    }

    fn present(
        &self,
        window: &gpui::Window,
        request: &ShareRequest,
        completion: ShareCompletion,
    ) -> Result<(), ShareUnavailable> {
        let Some(mtm) = MainThreadMarker::new() else {
            return refuse(completion, ShareUnavailable::NotOnMainThread);
        };
        let open = PICKING.with(|slot| slot.borrow_mut().take());
        if let Some(open) = open {
            open.picker.close();
        }

        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return refuse(completion, ShareUnavailable::NoNativeView);
        };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return refuse(completion, ShareUnavailable::NoNativeView);
        };

        let objects: Vec<Retained<AnyObject>> =
            request.items().iter().filter_map(share_object).collect();
        if objects.is_empty() {
            return refuse(completion, ShareUnavailable::NothingToShare);
        }
        let items = NSArray::from_retained_slice(&objects);

        let view: &NSView = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
        let rect = anchor_in_view(request.anchor(), view.bounds().size.height, view.isFlipped());
        let edge = if rect.below_is_min_y {
            NSRectEdge::MinY
        } else {
            NSRectEdge::MaxY
        };

        let picker = unsafe {
            NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items)
        };
        let delegate = ShareDelegate::new(mtm, completion);
        picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        PICKING.with(|slot| {
            slot.replace(Some(Picking {
                picker: picker.clone(),
                delegate,
            }))
        });
        picker.showRelativeToRect_ofView_preferredEdge(
            NSRect::new(NSPoint::new(rect.x, rect.y), NSSize::new(rect.width, rect.height)),
            view,
            edge,
        );
        Ok(())
    }
}

fn share_object(item: &ShareItem) -> Option<Retained<AnyObject>> {
    match item {
        ShareItem::Link(link) => {
            let text = NSString::from_str(link);
            Some(match NSURL::URLWithString(&text) {
                Some(url) => Retained::into_super(Retained::into_super(url)),
                None => Retained::into_super(Retained::into_super(text)),
            })
        }
        ShareItem::Text(text) => Some(Retained::into_super(Retained::into_super(
            NSString::from_str(text),
        ))),
        ShareItem::File(path) => {
            let path = path.to_str()?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            Some(Retained::into_super(Retained::into_super(url)))
        }
    }
}
