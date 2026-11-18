use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{
    define_class, msg_send, sel, AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, Message,
};
use objc2_app_kit::{
    NSCellImagePosition, NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSStatusBar,
    NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSData, NSSize, NSString};

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;
use crate::tray::*;

const TEMPLATE_ICON_PNG: &[u8] = include_bytes!("../../../../../app/resources/macos/TrayIconTemplate.png");
const TEMPLATE_ICON_POINTS: f64 = 18.0;

pub(crate) fn open(config: TrayConfig) -> Result<Box<dyn Tray>, TrayError> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Err(TrayError::NotMainThread);
    };

    let bar = NSStatusBar::systemStatusBar();
    let item = bar.statusItemWithLength(NSVariableStatusItemLength);
    let target = TrayEventTarget::new(mtm);

    let mut has_icon = false;
    if let Some(button) = item.button(mtm) {
        let target_object: &AnyObject = &target;
        unsafe {
            button.setTarget(Some(target_object));
            button.setAction(Some(sel!(iconClicked:)));
        }

        let name = NSString::from_str(&config.label);
        let _: () = unsafe { msg_send![&*button, setAccessibilityLabel: &*name] };

        match template_icon() {
            Some(image) => {
                button.setImage(Some(&image));
                button.setImagePosition(NSCellImagePosition::ImageLeading);
                has_icon = true;
            }
            None => tracing::warn!("tray: the template icon did not decode, the status item shows its label"),
        }
    }

    let mut tray = StatusItemTray {
        mtm,
        bar,
        item,
        target,
        label: config.label,
        tooltip: config.tooltip,
        unread: 0,
        has_icon,
    };

    tray.refresh();
    if let Some(items) = config.menu_items {
        let _ = tray.set_menu(items);
    }
    Ok(Box::new(tray))
}

pub(super) fn template_icon() -> Option<Retained<NSImage>> {
    let data = NSData::with_bytes(TEMPLATE_ICON_PNG);
    let image = NSImage::initWithData(NSImage::alloc(), &data)?;
    image.setSize(NSSize::new(TEMPLATE_ICON_POINTS, TEMPLATE_ICON_POINTS));
    image.setTemplate(true);
    Some(image)
}

struct TargetIvars {
    handler: RefCell<Option<TrayEventHandler>>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "RiseTrayEventTarget"]
    #[ivars = TargetIvars]
    struct TrayEventTarget;

    impl TrayEventTarget {
        #[unsafe(method(iconClicked:))]
        fn icon_clicked(&self, _sender: &AnyObject) {
            self.deliver(TrayEvent::Activate);
        }

        #[unsafe(method(menuItemPicked:))]
        fn menu_item_picked(&self, sender: &NSMenuItem) {
            match u32::try_from(sender.tag()) {
                Ok(id) => self.deliver(TrayEvent::Menu(id)),
                Err(_) => tracing::warn!("tray: a menu item carries a tag no TrayMenuItem can have"),
            }
        }
    }
);

impl TrayEventTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars {
            handler: RefCell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn deliver(&self, event: TrayEvent) {
        let _alive = self.retain();

        let Some(mut handler) = self.ivars().handler.borrow_mut().take() else {
            return;
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(event)));
        if outcome.is_err() {
            tracing::error!(target: "riseonly", ?event, "tray event handler panicked");
        }
        let mut slot = self.ivars().handler.borrow_mut();
        if slot.is_none() {
            *slot = Some(handler);
        }
    }
}

fn build_menu(mtm: MainThreadMarker, target: &TrayEventTarget, items: &[TrayMenuItem]) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);

    let target_object: &AnyObject = target;
    let no_key = NSString::from_str("");
    for entry in items {
        let title = NSString::from_str(&entry.label);
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &title,
                Some(sel!(menuItemPicked:)),
                &no_key,
            )
        };
        unsafe { item.setTarget(Some(target_object)) };
        item.setTag(entry.id as isize);
        item.setEnabled(entry.enabled);
        if let Some(checked) = entry.checked {
            item.setState(if checked {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        menu.addItem(&item);
    }
    menu
}

struct StatusItemTray {
    mtm: MainThreadMarker,
    bar: Retained<NSStatusBar>,
    item: Retained<NSStatusItem>,
    target: Retained<TrayEventTarget>,
    label: String,
    tooltip: String,
    unread: u32,
    has_icon: bool,
}

impl StatusItemTray {
    fn refresh(&self) -> PlatformSupport {
        let Some(button) = self.item.button(self.mtm) else {
            return PlatformSupport::Unsupported;
        };

        let title = status_button_title(self.has_icon, &self.label, self.unread);
        button.setTitle(&NSString::from_str(&title));

        let tooltip = tray_tooltip(HostOs::MacOs, &self.tooltip, self.unread);
        button.setToolTip(Some(&NSString::from_str(&tooltip)));

        PlatformSupport::Performed
    }
}

impl Tray for StatusItemTray {
    fn set_tooltip(&mut self, text: &str) -> PlatformSupport {
        self.tooltip = text.to_owned();
        self.refresh()
    }

    fn set_badge(&mut self, count: u32) -> PlatformSupport {
        self.unread = count;
        self.refresh()
    }

    fn set_visible(&mut self, visible: bool) -> PlatformSupport {
        self.item.setVisible(visible);
        PlatformSupport::Performed
    }

    fn availability(&self) -> TrayAvailability {
        TrayAvailability::Available
    }

    fn set_menu(&mut self, items: Vec<TrayMenuItem>) -> PlatformSupport {
        if !tray_menu_is_valid(&items) {
            tracing::warn!("tray: refused a menu with a duplicate or out of range item id");
            return PlatformSupport::Unsupported;
        }
        if items.is_empty() {
            self.item.setMenu(None);
        } else {
            let menu = build_menu(self.mtm, &self.target, &items);
            self.item.setMenu(Some(&menu));
        }
        PlatformSupport::Performed
    }

    fn set_event_handler(&mut self, handler: TrayEventHandler) -> Result<(), TrayError> {
        *self.target.ivars().handler.borrow_mut() = Some(handler);
        Ok(())
    }
}

impl Drop for StatusItemTray {
    fn drop(&mut self) {
        self.item.setMenu(None);
        if let Some(button) = self.item.button(self.mtm) {
            unsafe {
                button.setTarget(None);
                button.setAction(None);
            }
        }
        self.bar.removeStatusItem(&self.item);
    }
}
