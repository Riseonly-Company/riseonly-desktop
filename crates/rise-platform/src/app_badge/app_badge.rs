use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::gpui_shim::PlatformSupport;

pub const BADGE_CAP: u32 = 999;

/// text the badge should show, none when nothing should be drawn and zero is none
pub fn badge_text(count: u32) -> Option<String> {
    match count {
        0 => None,
        1..=BADGE_CAP => Some(count.to_string()),
        _ => Some(format!("{BADGE_CAP}+")),
    }
}

type TrayListener = Rc<dyn Fn(u32)>;

thread_local! {
    static CURRENT_COUNT: Cell<u32> = const { Cell::new(0) };
    static TRAY_LISTENER: RefCell<Option<TrayListener>> = const { RefCell::new(None) };
}

/// shows count everywhere, the answer comes from the os layer
/// unsupported means no launcher or tile took it, the tray still has the number (told first for that)
pub fn set_count(count: u32) -> PlatformSupport {
    CURRENT_COUNT.set(count);
    let listener = TRAY_LISTENER.with_borrow(Clone::clone);
    if let Some(listener) = listener {
        listener(count);
    }
    super::os::apply_count(count)
}

/// removes the badge everywhere, used on sign out
pub fn clear() -> PlatformSupport {
    set_count(0)
}

/// last number shown, an os layer whose window appears later applies it again from here
pub fn current_count() -> u32 {
    CURRENT_COUNT.get()
}

/// gives the tray its number from one place, listener calls set_badge
/// the tray opens after the first window so it gets the current count at once
pub fn set_tray_listener(listener: impl Fn(u32) + 'static) {
    let listener: TrayListener = Rc::new(listener);
    drop(TRAY_LISTENER.replace(Some(Rc::clone(&listener))));
    listener(current_count());
}

/// stops telling the tray, for a tray that has gone away
pub fn clear_tray_listener() {
    drop(TRAY_LISTENER.replace(None));
}
