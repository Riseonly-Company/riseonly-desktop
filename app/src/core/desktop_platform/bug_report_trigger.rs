use gpui::{App, Global, actions};
use rise_canon::RiseBugReportSurface;

use crate::core::desktop_platform::rise_app_lifecycle::{RiseAppLifecycle, ScenePhase};
use crate::modules::diagnostics::pages::bug_report::bug_report_sheet_host::BugReportSurfaceProbe;
use crate::modules::diagnostics::stores::diagnostics::diagnostics_interactions::bug_report_interactions::BugReportInteractionsStore;

actions!(rise_bug_report, [ReportBug]);

pub const REPORT_BUG_KEYSTROKE: &str = "secondary-shift-b";

struct BugReportTriggerInstalled;

impl Global for BugReportTriggerInstalled {}

/// safe to call twice
/// the listener is global so the menu item works in every window
pub fn install_bug_report_trigger(cx: &mut App) {
    install_with(present_from_shake, cx);
}

fn install_with(present: impl Fn(RiseBugReportSurface, &mut App) + 'static, cx: &mut App) {
    if cx.has_global::<BugReportTriggerInstalled>() {
        return;
    }
    cx.set_global(BugReportTriggerInstalled);

    cx.on_action(move |_: &ReportBug, cx| {
        if RiseAppLifecycle::scene_phase(cx) != ScenePhase::Active {
            return;
        }
        present(BugReportSurfaceProbe::current(), cx);
    });
}

fn present_from_shake(surface: RiseBugReportSurface, cx: &mut App) {
    let reports = BugReportInteractionsStore::shared(cx);
    reports.update(cx, |reports, cx| reports.present_from_shake(surface, cx));
}

#[cfg(test)]
#[path = "bug_report_trigger_tests.rs"]
mod tests;
