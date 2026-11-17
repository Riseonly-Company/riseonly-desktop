use std::sync::atomic::{AtomicBool, Ordering};

use gpui::App;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;

static ENABLED: AtomicBool = AtomicBool::new(cfg!(any(debug_assertions, feature = "sai_debug_tools")));

pub enum SaiWsScreenRefreshDebug {}

impl SaiWsScreenRefreshDebug {
    pub fn enabled() -> bool {
        cfg!(any(debug_assertions, feature = "sai_debug_tools")) && ENABLED.load(Ordering::Relaxed)
    }

    pub fn set_enabled(enabled: bool) {
        ENABLED.store(enabled, Ordering::Relaxed);
    }

    pub fn log(message: impl FnOnce() -> String, cx: &mut App) {
        if !Self::enabled() {
            return;
        }
        let message = message();
        Sai::log("SR", move || message, SaiLogKind::Websocket, cx);
    }
}
