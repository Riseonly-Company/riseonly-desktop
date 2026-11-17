use gpui::AppContext;

use crate::app::shell::window_title_bar::{WindowTitleBar, window_frame};
use crate::modules::rtc::pages::rise_rtc_call_window_chrome_hooks::{
    RiseRTCCallWindowChromeHooks, set_rise_rtc_call_window_chrome_hooks,
};

pub fn install_rise_rtc_call_window_chrome_hooks() {
    set_rise_rtc_call_window_chrome_hooks(RiseRTCCallWindowChromeHooks {
        title_bar: |on_close, window, cx| {
            cx.new(|cx| WindowTitleBar::without_app_menu(window, cx).on_close(on_close)).into()
        },
        set_title_bar_controls_theme: |title_bar, theme, cx| {
            if let Ok(bar) = title_bar.clone().downcast::<WindowTitleBar>() {
                bar.update(cx, |bar, cx| bar.set_controls_theme(theme, cx));
            }
        },
        window_frame: |content, window, cx| window_frame(content, window, cx),
    });
}
