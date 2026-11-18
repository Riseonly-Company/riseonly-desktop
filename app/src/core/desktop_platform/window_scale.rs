use gpui::{App, Context, Global, KeyBinding, actions};
use rise_i18n::tr_named;

use crate::core::desktop_platform::window_zoom_preferences::WindowZoomPreferences;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions};

actions!(rise_window_scale, [EnlargeInterface, ShrinkInterface, ResetInterfaceScale]);

pub const ENLARGE_KEYSTROKES: [&str; 3] = ["secondary-=", "secondary-shift-=", "secondary-+"];

pub const SHRINK_KEYSTROKES: [&str; 1] = ["secondary--"];

pub const RESET_KEYSTROKES: [&str; 1] = ["secondary-0"];

const ZOOM_TOAST_REPLACE_KEY: &str = "window-zoom";

struct WindowScaleInstalled;

impl Global for WindowScaleInstalled {}

/// stored zoom on every window and the three zoom keys, safe to call twice
pub fn install_window_scale(cx: &mut App) {
    if cx.has_global::<WindowScaleInstalled>() {
        return;
    }
    cx.set_global(WindowScaleInstalled);
    let factor = WindowZoomPreferences::shared(cx).read(cx).factor();
    cx.set_window_zoom(factor);
    cx.bind_keys(window_scale_key_bindings());
    cx.on_action(|_: &EnlargeInterface, cx| change_scale(cx, WindowZoomPreferences::zoom_in));
    cx.on_action(|_: &ShrinkInterface, cx| change_scale(cx, WindowZoomPreferences::zoom_out));
    cx.on_action(|_: &ResetInterfaceScale, cx| change_scale(cx, WindowZoomPreferences::reset));
}

pub fn window_scale_key_bindings() -> Vec<KeyBinding> {
    let enlarge = ENLARGE_KEYSTROKES.iter().map(|keystroke| KeyBinding::new(keystroke, EnlargeInterface, None));
    let shrink = SHRINK_KEYSTROKES.iter().map(|keystroke| KeyBinding::new(keystroke, ShrinkInterface, None));
    let reset = RESET_KEYSTROKES.iter().map(|keystroke| KeyBinding::new(keystroke, ResetInterfaceScale, None));
    enlarge.chain(shrink).chain(reset).collect()
}

fn change_scale(cx: &mut App, step: fn(&mut WindowZoomPreferences, &mut Context<WindowZoomPreferences>) -> bool) {
    let preferences = WindowZoomPreferences::shared(cx);
    preferences.update(cx, |preferences, cx| {
        step(preferences, cx);
    });
    let (factor, percent) = {
        let preferences = preferences.read(cx);
        (preferences.factor(), preferences.percent())
    };
    cx.set_window_zoom(factor);
    Notifier::send(
        tr_named("desktop_zoom_toast", &[("percent", &percent.to_string())]),
        NotifierSendOptions {
            replace_key: Some(ZOOM_TOAST_REPLACE_KEY.to_owned()),
            ..Default::default()
        },
        cx,
    );
}
