use gpui::App;

use crate::modules::diagnostics::stores::diagnostics_guide_sandbox_hooks::{
    DiagnosticsGuideSandboxHooks, set_diagnostics_guide_sandbox_hooks,
};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

pub fn install_diagnostics_guide_sandbox_hooks() {
    let _ = set_diagnostics_guide_sandbox_hooks(DiagnosticsGuideSandboxHooks { owns_bug_report_scene });
}

fn owns_bug_report_scene(cx: &App) -> bool {
    GuideSandbox::existing(cx).is_some_and(|sandbox| sandbox.read(cx).owns_bug_report_scene())
}
