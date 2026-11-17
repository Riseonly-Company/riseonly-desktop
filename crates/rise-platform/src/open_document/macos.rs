use std::cell::RefCell;
use std::path::Path;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSResponder, NSWindow};
use objc2_foundation::{NSString, NSURL};

#[link(name = "Quartz", kind = "framework")]
unsafe extern "C" {}

thread_local! {
    static SOURCE: RefCell<Option<ChainedSource>> = const { RefCell::new(None) };
}

struct ChainedSource {
    source: Retained<PreviewSource>,
    window: Weak<NSWindow>,
}

struct PreviewIvars {
    item: Retained<NSURL>,
}

define_class!(
    #[unsafe(super(NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[name = "RiseDocumentPreviewSource"]
    #[ivars = PreviewIvars]
    struct PreviewSource;

    unsafe impl NSObjectProtocol for PreviewSource {}

    impl PreviewSource {
        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_preview_items(&self, _panel: &AnyObject) -> isize {
            1
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn preview_item(&self, _panel: &AnyObject, _index: isize) -> Retained<NSURL> {
            self.ivars().item.clone()
        }

        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts_preview_panel_control(&self, _panel: &AnyObject) -> bool {
            true
        }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_preview_panel_control(&self, panel: &AnyObject) {
            let _: () = unsafe { msg_send![panel, setDataSource: self] };
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end_preview_panel_control(&self, panel: &AnyObject) {
            let _: () = unsafe { msg_send![panel, setDataSource: None::<&AnyObject>] };
        }
    }
);

impl PreviewSource {
    fn new(mtm: MainThreadMarker, item: Retained<NSURL>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PreviewIvars { item });
        unsafe { msg_send![super(this), init] }
    }
}

pub(super) fn quick_look(path: &Path) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("quick look is shown from the main thread only")?;
    let panel_class =
        AnyClass::get(c"QLPreviewPanel").ok_or("Quartz.framework has no QLPreviewPanel")?;
    let text = path.to_str().ok_or("the document path is not UTF-8")?;
    let item = NSURL::fileURLWithPath(&NSString::from_str(text));
    let app = NSApplication::sharedApplication(mtm);
    let window = app
        .mainWindow()
        .or_else(|| app.keyWindow())
        .or_else(|| app.windows().iter().find(|window| window.isVisible()))
        .ok_or("the app has no visible window for the panel to follow")?;
    let source = PreviewSource::new(mtm, item);
    let previous = SOURCE.with(|slot| slot.borrow_mut().take());
    if let Some(previous) = &previous {
        leave_responder_chain(previous);
    }
    unsafe {
        let panel: Option<Retained<AnyObject>> = msg_send![panel_class, sharedPreviewPanel];
        let panel = panel.ok_or("QLPreviewPanel has no shared panel")?;
        let next: Option<Retained<NSResponder>> = msg_send![&*window, nextResponder];
        let _: () = msg_send![&*source, setNextResponder: next.as_deref()];
        let _: () = msg_send![&*window, setNextResponder: &*source];
        let chained = ChainedSource { source, window: Weak::new(&*window) };
        let _: () = msg_send![&*panel, updateController];
        let controller: Option<Retained<AnyObject>> = msg_send![&*panel, currentController];
        let is_ours = controller
            .as_ref()
            .is_some_and(|controller| Retained::as_ptr(controller) == Retained::as_ptr(&chained.source).cast());
        if !is_ours {
            leave_responder_chain(&chained);
            return Err("another responder controls the Quick Look panel".to_owned());
        }
        let _: () = msg_send![&*panel, reloadData];
        let _: () = msg_send![&*panel, setCurrentPreviewItemIndex: 0isize];
        let _: () = msg_send![&*panel, makeKeyAndOrderFront: None::<&AnyObject>];
        SOURCE.with(|slot| *slot.borrow_mut() = Some(chained));
    }
    Ok(())
}

fn leave_responder_chain(chained: &ChainedSource) {
    let Some(window) = chained.window.load() else {
        return;
    };
    unsafe {
        let current: Option<Retained<NSResponder>> = msg_send![&*window, nextResponder];
        let is_linked = current
            .as_ref()
            .is_some_and(|current| Retained::as_ptr(current) == Retained::as_ptr(&chained.source).cast());
        if is_linked {
            let next: Option<Retained<NSResponder>> = msg_send![&*chained.source, nextResponder];
            let _: () = msg_send![&*window, setNextResponder: next.as_deref()];
        }
    }
}

pub(super) fn open_default(app: &gpui::App, path: &Path) {
    app.open_with_system(path);
}
