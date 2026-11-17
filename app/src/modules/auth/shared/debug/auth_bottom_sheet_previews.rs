#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::modules::auth::components::web_login_approval::web_login_approval_sheet::WebLoginApprovalContent;
use crate::modules::auth::widgets::modals::password_reset_intro_sheet::PasswordResetIntroSheet;
use crate::modules::user::stores::user::user_actions::user_types::ProfileModel;

pub enum AuthBottomSheetPreviews {}

impl AuthBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        let mut entries = vec![BottomSheetUi::debug_registration(
            "auth.reset",
            BottomSheetDebugRegistrationModule::Auth,
            "auth_reset_password_title",
            "PasswordResetIntroSheet",
            |open: Binding<bool>, _, _| {
                let content_open = open.clone();
                BottomSheetUi::new(
                    "auth.reset",
                    open,
                    move |_, _| {
                        let done = content_open.clone();
                        PasswordResetIntroSheet::new("+1 202 555 0142")
                            .on_continue(move |window, cx| done.set(false, window, cx))
                    },
                    |_, _| div(),
                )
                .snap_fractions(vec![0.5])
                .footer_shows_top_border(false)
            },
        )];

        entries.extend([false, true].map(|failed| {
            BottomSheetDebugRegistration::new(
                format!("auth.web_login.{failed}"),
                BottomSheetDebugRegistrationModule::Auth,
                "weblogin_title",
                "WebLoginApprovalContent",
                move |open: Binding<bool>, _, _| {
                    let approved = open.clone();
                    let denied = open.clone();
                    WebLoginApprovalContent::new(
                        open,
                        Some(ProfileModel {
                            id: "debug-profile".to_owned(),
                            name: "Alex Morgan".to_owned(),
                            tag: "alex".to_owned(),
                            ..ProfileModel::default()
                        }),
                        failed.then(|| tr("server_error_rate_limited_seconds").replace("%d", "30")),
                        false,
                        false,
                        move |window, cx| approved.set(false, window, cx),
                        move |window, cx| denied.set(false, window, cx),
                    )
                },
            )
            .with_variant(failed.then(|| "error".to_owned()))
        }));

        entries
    }
}
