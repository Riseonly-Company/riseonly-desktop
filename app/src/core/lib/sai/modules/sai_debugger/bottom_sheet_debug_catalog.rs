#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::modules::auth::shared::debug::auth_bottom_sheet_previews::AuthBottomSheetPreviews;
use crate::modules::bot::shared::debug::bot_bottom_sheet_previews::BotBottomSheetPreviews;
use crate::modules::chat::shared::debug::chat_bottom_sheet_previews::ChatBottomSheetPreviews;
use crate::modules::diagnostics::shared::debug::diagnostics_bottom_sheet_previews::DiagnosticsBottomSheetPreviews;
use crate::modules::map::shared::debug::map_bottom_sheet_previews::MapBottomSheetPreviews;
use crate::modules::media_picker::shared::debug::media_bottom_sheet_previews::MediaBottomSheetPreviews;
use crate::modules::moderation::shared::debug::moderation_bottom_sheet_previews::ModerationBottomSheetPreviews;
use crate::modules::music::shared::debug::music_bottom_sheet_previews::MusicBottomSheetPreviews;
use crate::modules::notify::shared::debug::notification_bottom_sheet_previews::NotificationBottomSheetPreviews;
use crate::modules::post::shared::debug::post_bottom_sheet_previews::PostBottomSheetPreviews;
use crate::modules::rtc::shared::debug::call_bottom_sheet_previews::CallBottomSheetPreviews;
use crate::modules::shorts::shared::debug::shorts_bottom_sheet_previews::ShortsBottomSheetPreviews;
use crate::modules::story::shared::debug::story_bottom_sheet_previews::StoryBottomSheetPreviews;
use crate::modules::user::shared::debug::debug_native_alert_previews::DebugNativeAlertPreviews;
use crate::modules::user::shared::debug::other_modal_previews::OtherModalPreviews;
use crate::modules::user::shared::debug::premium_bottom_sheet_previews::PremiumBottomSheetPreviews;
use crate::modules::user::shared::debug::profile_bottom_sheet_previews::ProfileBottomSheetPreviews;
use crate::modules::user::shared::debug::shared_sheet_previews::SharedSheetPreviews;
use crate::modules::vacancy::shared::debug::vacancy_bottom_sheet_previews::VacancyBottomSheetPreviews;

thread_local! {
    static ENTRIES: Rc<Vec<BottomSheetDebugRegistration>> = Rc::new(BottomSheetDebugCatalog::make_entries());
    static COUNTS: Rc<HashMap<BottomSheetDebugRegistrationModule, i64>> =
        Rc::new(BottomSheetDebugCatalog::make_counts());
}

pub enum BottomSheetDebugCatalog {}

impl BottomSheetDebugCatalog {
    /// feature owned factories are listed once so unopened screens show up too
    pub fn entries() -> Rc<Vec<BottomSheetDebugRegistration>> {
        ENTRIES.with(Rc::clone)
    }

    pub fn counts() -> Rc<HashMap<BottomSheetDebugRegistrationModule, i64>> {
        COUNTS.with(Rc::clone)
    }

    pub fn modules() -> Vec<BottomSheetDebugRegistrationModule> {
        let counts = Self::counts();
        BottomSheetDebugRegistrationModule::ALL_CASES
            .into_iter()
            .filter(|module| counts.contains_key(module))
            .collect()
    }

    fn make_entries() -> Vec<BottomSheetDebugRegistration> {
        let mut entries = ChatBottomSheetPreviews::entries();
        entries.extend(MusicBottomSheetPreviews::entries());
        entries.extend(ProfileBottomSheetPreviews::entries());
        entries.extend(PremiumBottomSheetPreviews::entries());
        entries.extend(AuthBottomSheetPreviews::entries());
        entries.extend(BotBottomSheetPreviews::entries());
        entries.extend(MapBottomSheetPreviews::entries());
        entries.extend(NotificationBottomSheetPreviews::entries());
        entries.extend(MediaBottomSheetPreviews::entries());
        entries.extend(ModerationBottomSheetPreviews::entries());
        entries.extend(DiagnosticsBottomSheetPreviews::entries());
        entries.extend(PostBottomSheetPreviews::entries());
        entries.extend(StoryBottomSheetPreviews::entries());
        entries.extend(CallBottomSheetPreviews::entries());
        entries.extend(ShortsBottomSheetPreviews::entries());
        entries.extend(VacancyBottomSheetPreviews::entries());
        entries.extend(SharedSheetPreviews::entries());
        entries.extend(OtherModalPreviews::entries());
        entries.extend(DebugNativeAlertPreviews::entries());
        let unique_count = entries.iter().map(|entry| entry.id.as_str()).collect::<HashSet<_>>().len();
        assert!(unique_count == entries.len(), "Duplicate bottom sheet preview ID");
        entries
    }

    fn make_counts() -> HashMap<BottomSheetDebugRegistrationModule, i64> {
        let mut counts = HashMap::new();
        for entry in Self::entries().iter() {
            *counts.entry(entry.module).or_insert(0) += 1;
        }
        counts
    }
}
