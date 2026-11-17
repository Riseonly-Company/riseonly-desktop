#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::modules::vacancy::components::resume_form_ui::SearchablePickerSheet;
use crate::modules::vacancy::shared::picker_option::PickerOption;

pub enum VacancyBottomSheetPreviews {}

impl VacancyBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        vec![BottomSheetDebugRegistration::new(
            "jobs.experience",
            BottomSheetDebugRegistrationModule::Jobs,
            "vacancy_resume_exp_level",
            "SearchablePickerSheet",
            |open: Binding<bool>, _, cx| {
                let close = open.clone();
                SearchablePickerSheet::new(
                    "jobs.experience.sheet",
                    tr("vacancy_resume_exp_level"),
                    vec![
                        PickerOption::new("none", tr("vacancy_exp_no")),
                        PickerOption::new("1-3", tr("vacancy_exp_1_3")),
                        PickerOption::new("3-6", tr("vacancy_exp_3_6")),
                        PickerOption::new("6-12", tr("vacancy_exp_6_12")),
                    ],
                    move |_, window, cx| close.set(false, window, cx),
                )
                .presented("jobs.experience", open, cx)
            },
        )]
    }
}
