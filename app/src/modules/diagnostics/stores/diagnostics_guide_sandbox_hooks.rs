use gpui::App;

use crate::core::feature_hooks::FeatureHookSlot;

pub struct DiagnosticsGuideSandboxHooks {
    pub owns_bug_report_scene: fn(cx: &App) -> bool,
}

pub(crate) static HOOKS: FeatureHookSlot<DiagnosticsGuideSandboxHooks> = FeatureHookSlot::new("HOOKS");

static UNINSTALLED: DiagnosticsGuideSandboxHooks = DiagnosticsGuideSandboxHooks { owns_bug_report_scene: |_| false };

pub fn set_diagnostics_guide_sandbox_hooks(
    hooks: DiagnosticsGuideSandboxHooks,
) -> Result<(), DiagnosticsGuideSandboxHooks> {
    HOOKS.set(hooks)
}

pub(crate) fn diagnostics_guide_sandbox_hooks() -> &'static DiagnosticsGuideSandboxHooks {
    HOOKS.get().unwrap_or(&UNINSTALLED)
}
