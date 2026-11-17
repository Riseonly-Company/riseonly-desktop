#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::modules::map::components::map_invite_sheet::MapInviteSheet;
use crate::modules::map::components::map_onboarding_sheet::MapOnboardingSheet;
use crate::modules::map::stores::map_actions::map_types::{
    MapInvitePreview, MapInviteRelation, MapInviteSheetState, MapInviteSheetStatePhase, MapUser,
};
use crate::modules::map::stores::map_interactions::map_interactions::MapOnboardingStep;

pub enum MapBottomSheetPreviews {}

impl MapBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        let variants = [
            (MapInviteRelation::None, "map_invite_subtitle"),
            (MapInviteRelation::Requested, "map_invite_requested_subtitle"),
            (MapInviteRelation::Incoming, "map_invite_incoming_subtitle"),
            (MapInviteRelation::Friends, "map_invite_friends_subtitle"),
        ];
        variants
            .into_iter()
            .map(|(relation, title)| {
                BottomSheetDebugRegistration::new(
                    format!("map.invite.{}", relation.raw_value()),
                    BottomSheetDebugRegistrationModule::Map,
                    title,
                    "MapInviteSheet",
                    move |open: Binding<bool>, _, _| {
                        let state = MapInviteSheetState::new(
                            "debug-preview".to_owned(),
                            MapInviteSheetStatePhase::Preview(MapInvitePreview {
                                owner: Self::preview_owner(),
                                relation,
                                viewer_is_member: true,
                                request_id: None,
                            }),
                        );
                        let retry = open.clone();
                        let submit = open.clone();
                        MapInviteSheet::new(
                            open,
                            Some(state),
                            move |window, cx| retry.set(false, window, cx),
                            move |window, cx| submit.set(false, window, cx),
                        )
                        .full_page(false)
                    },
                )
            })
            .chain(Self::onboarding())
            .collect()
    }

    fn preview_owner() -> MapUser {
        MapUser {
            id: "debug-preview-map-user".to_owned(),
            tag: "alex".to_owned(),
            name: "Alex Morgan".to_owned(),
            logo: String::new(),
            is_premium: false,
            is_official: false,
            name_color: None,
            premium_badge_emoji: None,
            is_deleted: false,
        }
    }

    fn onboarding() -> Vec<BottomSheetDebugRegistration> {
        let steps = [
            (MapOnboardingStep::Intro, "map_onboarding_intro_title"),
            (MapOnboardingStep::Photos, "map_onboarding_photos_title"),
            (MapOnboardingStep::Ready, "map_onboarding_ready_title"),
        ];
        steps
            .into_iter()
            .enumerate()
            .map(|(index, (step, title))| {
                BottomSheetDebugRegistration::new(
                    format!("map.onboarding.{index}"),
                    BottomSheetDebugRegistrationModule::Map,
                    title,
                    "MapOnboardingSheet",
                    move |open: Binding<bool>, _, _| {
                        let primary = open.clone();
                        let skip = open.clone();
                        MapOnboardingSheet::new(
                            open,
                            step,
                            false,
                            move |window, cx| primary.set(false, window, cx),
                            move |window, cx| skip.set(false, window, cx),
                        )
                        .full_page(false)
                    },
                )
            })
            .collect()
    }
}
