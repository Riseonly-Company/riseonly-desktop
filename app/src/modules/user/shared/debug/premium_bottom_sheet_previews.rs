#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use gpui::{App, Window};
use rise_core::foundation::foundation_calendar::Calendar;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, Locale, format_printf, tr, tr_args};
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationKind, BottomSheetDebugRegistrationModule,
};
use crate::core::widgets::overlay::download_limit_help_modal::DownloadLimitHelpModal;
use crate::core::widgets::overlay::premium_upsell_host::PremiumUpsellSheet;
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellPrompt;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::user::components::account_limit_sheet::AccountLimitSheet;
use crate::modules::user::shared::debug::shared_sheet_previews::debug_close;

pub enum PremiumBottomSheetPreviews {}

impl PremiumBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        let mut entries = vec![
            Self::prompt("premium.upsell", "premium_upsell_title", false, true),
            Self::prompt("premium.download.free", "download_limit_title", true, true),
            Self::prompt("premium.download.premium", "download_limit_title", true, false),
            BottomSheetDebugRegistration::new(
                "limits.download.help",
                BottomSheetDebugRegistrationModule::Limits,
                "download_limit_help_title",
                "DownloadLimitHelpModal",
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    DownloadLimitHelpModal::new("limits.download.help.modal", open)
                },
            )
            .with_kind(BottomSheetDebugRegistrationKind::Modal),
        ];
        for premium in [false, true] {
            entries.push(
                BottomSheetDebugRegistration::new(
                    format!("limits.accounts.{premium}"),
                    BottomSheetDebugRegistrationModule::Limits,
                    "account_limit_title",
                    "AccountLimitSheet",
                    move |open: Binding<bool>, _: &mut Window, _: &mut App| {
                        let close = debug_close(open.clone());
                        AccountLimitSheet::new(open, premium, close)
                    },
                )
                .with_variant(Some(
                    if premium { "account_limit_premium_message" } else { "account_limit_standard_message" }
                        .to_owned(),
                )),
            );
        }
        for reason in [
            "premium_upsell_reason_sticker_pack",
            "premium_upsell_reason_story_stealth",
            "premium_upsell_reason_call_recording",
        ] {
            entries.push(Self::upsell(format!("limits.{reason}"), BottomSheetDebugRegistrationModule::Limits, reason, move |prompt_id| PremiumUpsellPrompt {
                is_feature: true,
                ..PremiumUpsellPrompt::new(prompt_id, tr(reason))
            }));
        }
        entries.push(Self::upsell("limits.pinned_chats".to_owned(), BottomSheetDebugRegistrationModule::Limits, "limit_pinned_chats", |prompt_id| {
            PremiumUpsellPrompt::new(
                prompt_id,
                tr_args("chat_pin_limit_title", &[FormatArg::Int(ChatServicesStore::max_pinned_chats(false))]),
            )
        }));
        entries
    }

    fn prompt(id: &str, title: &str, download: bool, upgrade: bool) -> BottomSheetDebugRegistration {
        let module = if download {
            BottomSheetDebugRegistrationModule::Limits
        } else {
            BottomSheetDebugRegistrationModule::Premium
        };
        Self::upsell(id.to_owned(), module, title, move |prompt_id| PremiumUpsellPrompt {
            is_download_limit: download,
            can_upgrade: upgrade,
            recovery_text: download.then(|| {
                let now = RiseClock::now();
                let label = RiseFormat::date_time_label(
                    &now.adding_time_interval(3600.0),
                    &now,
                    &Calendar::current(),
                    Locale::app_locale().identifier(),
                );
                format_printf(&tr("download_limit_retry"), &[FormatArg::Text(label.into())])
            }),
            ..PremiumUpsellPrompt::new(
                prompt_id,
                tr(if download { "download_limit_message" } else { "premium_upsell_reason_generic" }),
            )
        })
        .with_variant(
            download.then(|| if upgrade { "premium_upsell_cta" } else { "subscription_plan_active" }.to_owned()),
        )
    }

    fn upsell(
        id: String,
        module: BottomSheetDebugRegistrationModule,
        title: &str,
        make_prompt: impl Fn(Uuid) -> PremiumUpsellPrompt + 'static,
    ) -> BottomSheetDebugRegistration {
        let prompt_id = Uuid::new_v4();
        let sheet_id = format!("{id}.sheet");
        BottomSheetDebugRegistration::new(
            id,
            module,
            title,
            "PremiumUpsellSheet",
            move |open: Binding<bool>, _: &mut Window, _: &mut App| {
                let on_dismiss = debug_close(open.clone());
                let on_upgrade = debug_close(open.clone());
                PremiumUpsellSheet::new(sheet_id.clone(), open, Some(make_prompt(prompt_id)), on_dismiss, on_upgrade)
            },
        )
    }
}
