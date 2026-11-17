use gpui::{AnyElement, ElementId, Entity, IntoElement};
use rise_theme::ThemeStore;
use rise_ui::swiftui::binding::Binding;

use crate::core::widgets::sheets::report_evidence_picker_hooks::{
    ReportEvidencePickerComplete, ReportEvidencePickerDismissed, ReportEvidencePickerHooks,
    set_report_evidence_picker_hooks,
};
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::ui::media_picker_sheet::MediaPickerSheet;

fn media_picker_sheet(
    id: ElementId,
    is_presented: Binding<bool>,
    configuration: MediaPickerConfiguration,
    theme_store: Entity<ThemeStore>,
    on_complete: ReportEvidencePickerComplete,
    on_dismissed: ReportEvidencePickerDismissed,
) -> AnyElement {
    MediaPickerSheet::new(id, is_presented, configuration, theme_store, on_complete)
        .experimental_bottom_sheet(true)
        .on_dismissed(on_dismissed)
        .into_any_element()
}

pub fn install_report_evidence_picker_hooks() {
    set_report_evidence_picker_hooks(ReportEvidencePickerHooks { media_picker_sheet });
}
