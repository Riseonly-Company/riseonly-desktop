use crate::core::ui::hold_context_menu_ui::hold_context_menu_host::HoldContextMenuWindowPresenter;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presenter_hooks::{
    HoldContextMenuPresenterHooks, set_hold_context_menu_presenter_hooks,
};

pub fn install_hold_context_menu_presenter_hooks() {
    set_hold_context_menu_presenter_hooks(HoldContextMenuPresenterHooks {
        show: HoldContextMenuWindowPresenter::show,
        restore_source_key_window: HoldContextMenuWindowPresenter::restore_source_key_window,
        source_window: |cx| HoldContextMenuWindowPresenter::shared(cx).read(cx).source_window(),
        hide: HoldContextMenuWindowPresenter::hide,
    });
}
