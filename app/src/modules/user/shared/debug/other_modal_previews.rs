#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::rc::Rc;

use gpui::{App, Empty, Window};
use rise_canon::rise_maintenance::{DEFAULT_MEDIA_REF, MediaKind};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::core::maintenance::maintenance_gate::MaintenanceSnapshot;
use crate::core::maintenance::maintenance_modal::MaintenanceModal;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationKind, BottomSheetDebugRegistrationModule,
};
use crate::core::widgets::overlay::achievement_earned_host::AchievementEarnedSheet;
use crate::core::widgets::overlay::premium_promo_host::PremiumPromoSheet;
use crate::core::widgets::overlay::premium_promo_store::PremiumPromoStore;
use crate::core::widgets::overlay::subscription_success_host::SubscriptionSuccessSheet;
use crate::modules::user::shared::debug::profile_bottom_sheet_previews::debug_achievement;
use crate::modules::user::shared::debug::shared_sheet_previews::debug_close;

pub enum OtherModalPreviews {}

impl OtherModalPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        let mut entries: Vec<BottomSheetDebugRegistration> = [false, true]
            .map(|known| {
                BottomSheetDebugRegistration::new(
                    format!("other.maintenance.{known}"),
                    BottomSheetDebugRegistrationModule::Other,
                    "maintenance_default_title",
                    "MaintenanceModal",
                    move |open: Binding<bool>, _: &mut Window, cx: &mut App| -> AnyElement {
                        if !open.get(cx) {
                            return Empty.into_any_element();
                        }
                        MaintenanceModal::new(
                            MaintenanceSnapshot::new(
                                true,
                                1,
                                tr("maintenance_default_title"),
                                tr("maintenance_default_body"),
                                MediaKind::Builtin,
                                DEFAULT_MEDIA_REF.to_owned(),
                                known.then(|| RiseClock::now().adding_time_interval(3600.0)),
                                0,
                            ),
                            Rc::new(debug_close(open)),
                            Rc::new(|_: &mut Window, _: &mut App| {}),
                        )
                        .into_any_element()
                    },
                )
                .with_variant(Some(
                    if known { "maintenance_modal_dismiss" } else { "maintenance_eta_unknown" }.to_owned(),
                ))
                .with_kind(BottomSheetDebugRegistrationKind::Modal)
            })
            .into_iter()
            .collect();
        entries.push(BottomSheetDebugRegistration::new(
            "premium.success",
            BottomSheetDebugRegistrationModule::Premium,
            "subscription_success_title",
            "SubscriptionSuccessSheet",
            |open: Binding<bool>, _: &mut Window, _: &mut App| {
                let close = debug_close(open.clone());
                SubscriptionSuccessSheet::new(open, close)
            },
        ));
        entries.push(BottomSheetDebugRegistration::new(
            "profile.achievement.earned",
            BottomSheetDebugRegistrationModule::Profile,
            "achievement_earned_title",
            "AchievementEarnedSheet",
            |open: Binding<bool>, _: &mut Window, _: &mut App| {
                let close = debug_close(open.clone());
                AchievementEarnedSheet::new(open, Some(debug_achievement()), close)
            },
        ));
        for verified in [false, true] {
            for lifetime in [false, true] {
                entries.push(Self::promo(verified, lifetime));
            }
        }
        entries
    }

    fn promo(verified: bool, lifetime: bool) -> BottomSheetDebugRegistration {
        BottomSheetDebugRegistration::new(
            format!("premium.promo.{verified}.{lifetime}"),
            BottomSheetDebugRegistrationModule::Premium,
            if verified { "verified_promo_title" } else { "premium_promo_title" },
            "PremiumPromoSheet",
            move |open: Binding<bool>, _: &mut Window, _: &mut App| {
                let period_text = if verified {
                    tr("verified_promo_badge")
                } else {
                    PremiumPromoStore::period_text_for(90, lifetime)
                };
                let on_redeem = debug_close(open.clone());
                let on_dismiss = debug_close(open.clone());
                PremiumPromoSheet::new(
                    open,
                    verified,
                    lifetime,
                    Some("RISEONLY".to_owned()),
                    period_text,
                    None,
                    true,
                    false,
                    on_redeem,
                    on_dismiss,
                )
            },
        )
        .with_variant(Some(
            if lifetime { "premium_promo_period_lifetime" } else { "premium_gift_title" }.to_owned(),
        ))
    }
}
