#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::modules::moderation::engine::rise_moderation_wire::{ReportRowDTO, RiseStoredCaseCapabilities};
use crate::modules::moderation::pages::moderation_case_sheets::{
    AdminSanctionSheet, AppealVerdictSheet, ReopenSheet, ResolveConfirmSheet,
};
use crate::modules::moderation::stores::moderation_actions::moderation_types::{
    AdminSanctionKind, AppealDecision, AppealDecisionExt, ResolveAction, ResolveActionExt,
};

const PREVIEW_REPORT_JSON: &str =
    r#"{"id":-1,"reportType":"post","status":"pending","reason":"spam","description":"Preview report"}"#;

pub enum ModerationBottomSheetPreviews {}

impl ModerationBottomSheetPreviews {
    fn report() -> Option<ReportRowDTO> {
        serde_json::from_str(PREVIEW_REPORT_JSON).ok()
    }

    fn capabilities() -> RiseStoredCaseCapabilities {
        RiseStoredCaseCapabilities {
            is_admin: true,
            can_note: true,
            can_dismiss: true,
            can_warn: true,
            can_delete_content: true,
            can_block_author: true,
            can_mute_author: true,
            ..RiseStoredCaseCapabilities::default()
        }
    }

    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        let mut entries = vec![BottomSheetDebugRegistration::native(
            "moderation.reopen",
            BottomSheetDebugRegistrationModule::Moderation,
            "case_reopen_title",
            "ReopenSheet",
            |open: Binding<bool>, _, _| {
                let done = open.clone();
                ReopenSheet::new(-1, move |window, cx| done.set(false, window, cx))
                    .on_submit(move |_, window, cx| open.set(false, window, cx))
            },
        )];

        entries.extend(AdminSanctionKind::ALL.into_iter().map(|kind| {
            let id = format!("moderation.admin.{}", kind.raw_value());
            BottomSheetDebugRegistration::native(
                id,
                BottomSheetDebugRegistrationModule::Moderation,
                kind.title_key(),
                "AdminSanctionSheet",
                move |open: Binding<bool>, _, _| {
                    let done = open.clone();
                    AdminSanctionSheet::new(
                        kind,
                        Some("debug-user".to_owned()),
                        Some("Alex Morgan".to_owned()),
                        Some(-1),
                        None,
                        move |window, cx| done.set(false, window, cx),
                    )
                    .ip(Some("192.0.2.1".to_owned()))
                    .on_submit(move |window, cx| open.set(false, window, cx))
                },
            )
        }));

        entries.extend([AppealDecision::Accept, AppealDecision::Reject].into_iter().map(|decision| {
            let id = format!("moderation.appeal.{}", decision.id());
            BottomSheetDebugRegistration::native(
                id,
                BottomSheetDebugRegistrationModule::Moderation,
                decision.title_key(),
                "AppealVerdictSheet",
                move |open: Binding<bool>, _, _| {
                    let done = open.clone();
                    AppealVerdictSheet::new(
                        -1,
                        decision,
                        move |window, cx| done.set(false, window, cx),
                    )
                    .on_submit(move |window, cx| open.set(false, window, cx))
                },
            )
        }));

        if let Some(report) = Self::report() {
            let capabilities = Self::capabilities();
            entries.extend(ResolveAction::ALL.into_iter().map(|action| {
                let id = format!("moderation.resolve.{}", action.id());
                let report = report.clone();
                let capabilities = capabilities.clone();
                BottomSheetDebugRegistration::native(
                    id,
                    BottomSheetDebugRegistrationModule::Moderation,
                    action.title_key(),
                    "ResolveConfirmSheet",
                    move |open: Binding<bool>, _, _| {
                        let done = open.clone();
                        ResolveConfirmSheet::new(
                            report.clone(),
                            capabilities.clone(),
                            action,
                            move |window, cx| done.set(false, window, cx),
                        )
                        .on_submit(move |window, cx| open.set(false, window, cx))
                    },
                )
            }));
        }

        entries
    }
}
