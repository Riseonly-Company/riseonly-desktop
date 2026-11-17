use gpui::{AnyWindowHandle, App};

use crate::app::root_view::RootView;
use crate::modules::share_extension::entry::share_item_loader::ShareExtensionItem;
use crate::modules::system_share::desktop_share_intake_hooks::{
    DesktopShareIntakeHooks, set_desktop_share_intake_hooks,
};

pub fn install_desktop_share_intake_hooks() {
    set_desktop_share_intake_hooks(DesktopShareIntakeHooks { present_shared_items });
}

fn present_shared_items(shell: AnyWindowHandle, items: Vec<ShareExtensionItem>, cx: &mut App) {
    let Some(handle) = shell.downcast::<RootView>() else {
        return;
    };
    let _ = handle.update(cx, |view, window, cx| {
        view.present_shared_items(items, cx);
        window.activate_window();
    });
}
