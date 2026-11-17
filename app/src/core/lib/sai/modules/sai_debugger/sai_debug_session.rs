use gpui::App;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;

pub struct SaiDebugSession;

impl SaiDebugSession {
    pub fn shared() -> Self {
        Self
    }

    pub fn append_fetch_line(&self, line: String, cx: &mut App) {
        Sai::log("Fetch", move || line, SaiLogKind::Websocket, cx);
    }

    pub fn append_cache_line(&self, line: String, cx: &mut App) {
        Sai::log("Cache", move || line, SaiLogKind::Cache, cx);
    }

    pub fn append_event_line(&self, line: String, cx: &mut App) {
        Sai::log("Event", move || line, SaiLogKind::Event, cx);
    }
}
