use std::fmt::Debug;
use std::panic::Location;

use gpui::{App, Entity};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::logger::sai_logger::SaiLogger;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;

/// sai entry point, every member runs on the main thread
pub enum Sai {}

impl Sai {
    pub fn ws(cx: &mut App) -> Entity<SaiWebSocketManager> {
        SaiWebSocketManager::shared(cx)
    }

    #[track_caller]
    pub fn log_unnamed<V: Debug + 'static>(value: impl FnOnce() -> V, cx: &mut App) {
        let caller = Location::caller();
        if Self::records_logs() {
            let payload = value();
            SaiLogger::shared(cx).update(cx, |logger, cx| {
                logger.record("Unnamed", &payload, SaiLogKind::Default, caller, cx);
            });
        }
    }

    #[track_caller]
    pub fn log<V: Debug + 'static>(
        name: &str,
        value: impl FnOnce() -> V,
        kind: SaiLogKind,
        cx: &mut App,
    ) {
        let caller = Location::caller();
        if Self::records_logs() {
            let payload = value();
            SaiLogger::shared(cx).update(cx, |logger, cx| {
                logger.record(name, &payload, kind, caller, cx);
            });
        }
    }

    #[track_caller]
    pub fn log_channel<V: Debug + 'static>(
        name: &str,
        value: impl FnOnce() -> V,
        channel: &str,
        cx: &mut App,
    ) {
        let caller = Location::caller();
        if Self::records_logs() {
            let payload = value();
            SaiLogger::shared(cx).update(cx, |logger, cx| {
                logger.record(name, &payload, SaiLogKind::Custom(channel.to_owned()), caller, cx);
            });
        }
    }

    /// true only in debug builds, no debug tools flag on desktop
    fn records_logs() -> bool {
        cfg!(debug_assertions)
    }
}
