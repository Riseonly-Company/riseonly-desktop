#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use gpui::{App, Window};
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::modules::user::components::achievements::achievement_detail_sheet::AchievementDetailSheet;
use crate::modules::user::components::contact_experience_sheets::ContactSyncOnboardingSheet;
use crate::modules::user::engine::core::rise_user_achievement_wire_models::RiseAchievement;
use crate::modules::user::shared::debug::shared_sheet_previews::debug_close;

pub enum ProfileBottomSheetPreviews {}

impl ProfileBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        vec![
            BottomSheetDebugRegistration::sheet(
                "profile.achievement",
                BottomSheetDebugRegistrationModule::Profile,
                "profile_achievements_row",
                "AchievementDetailSheet",
                vec![0.7, 0.93],
                None,
                |open: Binding<bool>, _: &mut Window, _: &mut App| {
                    AchievementDetailSheet::new(debug_achievement(), false).on_dismiss(debug_close(open))
                },
            ),
            BottomSheetDebugRegistration::new(
                "profile.contacts.onboarding",
                BottomSheetDebugRegistrationModule::Profile,
                "contact_onboarding_title",
                "ContactSyncOnboardingSheet",
                |open: Binding<bool>, _: &mut Window, cx: &mut App| {
                    let close = open.clone();
                    ContactSyncOnboardingSheet::new()
                        .on_decision(move |_, window: &mut Window, cx: &mut App| close.set(false, window, cx))
                        .presented("profile.contacts.onboarding.sheet", open, cx)
                },
            ),
        ]
    }
}

pub(super) fn debug_achievement() -> RiseAchievement {
    RiseAchievement {
        id: 1,
        code_key: "debug-preview".to_owned(),
        title: "First steps".to_owned(),
        description: "Every great story begins with a first step.".to_owned(),
        how_to_earn: "Create your profile and join the conversation.".to_owned(),
        granted_at_ms: 1_750_000_000_000,
        granted_count: 128,
        rarity: "rare".to_owned(),
        ..RiseAchievement::default()
    }
}
