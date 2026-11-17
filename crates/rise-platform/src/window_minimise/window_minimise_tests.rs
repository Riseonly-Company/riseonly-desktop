use std::sync::Arc;

use parking_lot::Mutex;
use raw_window_handle::{HandleError, HasWindowHandle, WindowHandle};

use super::window_minimise::*;
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

struct NoHandle;

impl HasWindowHandle for NoHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Err(HandleError::Unavailable)
    }
}

/// replays a fixed series of raw reports into the sink like a branch would
struct Replay(Vec<bool>);

impl WindowMinimiseSource for Replay {
    fn watch(
        &self,
        _window: &dyn HasWindowHandle,
        sink: MinimisedSink,
    ) -> Result<MinimiseWatch, WindowMinimiseUnavailable> {
        for minimised in &self.0 {
            sink(*minimised);
        }
        Ok(MinimiseWatch::new(()))
    }
}

struct Unwritten;

impl WindowMinimiseSource for Unwritten {}

fn collected(source: &impl WindowMinimiseSource) -> (Result<MinimiseWatch, WindowMinimiseUnavailable>, Vec<bool>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let watch = watch_on(source, &NoHandle, move |minimised| sink.lock().push(minimised));
    let seen = seen.lock().clone();
    (watch, seen)
}

#[test]
fn the_start_state_is_reported_once() {
    let (watch, seen) = collected(&Replay(vec![false]));
    assert!(watch.is_ok());
    assert_eq!(seen, vec![false]);
}

#[test]
fn a_miniaturize_and_an_app_hide_of_one_trip_report_once() {
    let (_watch, seen) = collected(&Replay(vec![false, true, true, false, false]));
    assert_eq!(seen, vec![false, true, false]);
}

#[test]
fn a_window_minimised_at_start_reports_true_first() {
    let (_watch, seen) = collected(&Replay(vec![true, false]));
    assert_eq!(seen, vec![true, false]);
}

#[test]
fn an_unwritten_branch_names_the_host_and_reports_nothing() {
    let (watch, seen) = collected(&Unwritten);
    assert_eq!(
        watch.err(),
        Some(WindowMinimiseUnavailable::BranchNotWritten { host: HostOs::current() })
    );
    assert!(seen.is_empty());
}

#[test]
fn the_deduplicator_passes_only_changes() {
    let deduplicator = MinimisedDeduplicator::default();
    assert_eq!(deduplicator.report(true), Some(true));
    assert_eq!(deduplicator.report(true), None);
    assert_eq!(deduplicator.report(false), Some(false));
    assert_eq!(deduplicator.report(false), None);
}

#[test]
fn a_branch_without_hiding_answers_unsupported() {
    let source = Replay(Vec::new());
    assert_eq!(source.hide_window(&NoHandle), PlatformSupport::Unsupported);
    assert_eq!(source.show_window(&NoHandle), PlatformSupport::Unsupported);
}
