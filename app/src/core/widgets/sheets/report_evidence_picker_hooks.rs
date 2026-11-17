use gpui::{AnyElement, App, ElementId, Entity, Window};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::core::feature_hooks::FeatureHookSlot;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerResult;

pub type ReportEvidencePickerComplete = Box<dyn Fn(MediaPickerResult, &mut Window, &mut App)>;
pub type ReportEvidencePickerDismissed = Box<dyn Fn(&mut Window, &mut App)>;

pub struct ReportEvidencePickerHooks {
    pub media_picker_sheet: fn(
        id: ElementId,
        is_presented: Binding<bool>,
        configuration: MediaPickerConfiguration,
        theme_store: Entity<ThemeStore>,
        on_complete: ReportEvidencePickerComplete,
        on_dismissed: ReportEvidencePickerDismissed,
    ) -> AnyElement,
}

pub(crate) static REPORT_EVIDENCE_PICKER_HOOKS: FeatureHookSlot<ReportEvidencePickerHooks> =
    FeatureHookSlot::new("REPORT_EVIDENCE_PICKER_HOOKS");

pub fn set_report_evidence_picker_hooks(hooks: ReportEvidencePickerHooks) {
    let _ = REPORT_EVIDENCE_PICKER_HOOKS.set(hooks);
}

pub fn report_evidence_picker_hooks() -> Option<&'static ReportEvidencePickerHooks> {
    REPORT_EVIDENCE_PICKER_HOOKS.get()
}
