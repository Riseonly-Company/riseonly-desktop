use std::cell::Cell;

use gpui::TestAppContext;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::logger::sai_logger::SaiLogger;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::stores::message::message_services::message_pagination_trace::MessagePaginationTrace;

#[gpui::test]
fn payload_evaluation_matches_logging_build_mode(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let name = "SaiLoggingPerformanceContractTests";
        let evaluations = Cell::new(0);

        Sai::log(
            name,
            || {
                evaluations.set(evaluations.get() + 1);
                "payload"
            },
            SaiLogKind::Debug,
            cx,
        );

        if cfg!(debug_assertions) {
            assert_eq!(evaluations.get(), 1);
            let logger = SaiLogger::shared(cx);
            let entry = logger.read(cx).entries().into_iter().find(|entry| entry.name == name);
            if let Some(entry) = entry {
                logger.update(cx, |logger, cx| logger.remove_entry(entry.id, cx));
            }
        } else {
            assert_eq!(evaluations.get(), 0);
        }
    });
}

#[gpui::test]
fn disabled_pagination_trace_does_not_evaluate_payloads(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let logger = SaiLogger::shared(cx);
        let before = logger.read(cx).entries().len();

        MessagePaginationTrace::record(
            "event",
            "scope",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some("detail"),
            cx,
        );

        let after = logger.read(cx).entries().len();
        if MessagePaginationTrace::IS_ENABLED {
            assert_eq!(after, before + 1);
        } else {
            assert_eq!(after, before);
        }
    });
}
