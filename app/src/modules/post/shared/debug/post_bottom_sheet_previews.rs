#![cfg(any(debug_assertions, feature = "sai_debug_tools"))]

use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};
use crate::modules::post::components::collaborators::post_collaborators_list_sheet::PostCollaboratorsListSheet;
use crate::modules::post::engine::rise_post_engine_models::PostCollaboratorsSnapshot;
use crate::modules::post::stores::post_actions::post_types::{
    PostCollaboratorKind, PostCollaboratorRow, PostCollaboratorStatus, PostCollaboratorUser,
};

pub enum PostBottomSheetPreviews {}

impl PostBottomSheetPreviews {
    pub fn entries() -> Vec<BottomSheetDebugRegistration> {
        [PostCollaboratorKind::Coauthor, PostCollaboratorKind::Tag]
            .into_iter()
            .map(|kind| {
                let title =
                    if kind == PostCollaboratorKind::Coauthor { "post_coauthors_title" } else { "post_tagged_people_title" };
                BottomSheetDebugRegistration::new(
                    format!("posts.people.{}", kind.raw_value()),
                    BottomSheetDebugRegistrationModule::Posts,
                    title,
                    "PostCollaboratorsListSheet",
                    move |open: Binding<bool>, _, _| {
                        PostCollaboratorsListSheet::new(kind, tr(title))
                            .presentation_override(Some(open))
                            .snapshot_override(Some(Self::snapshot(kind)))
                    },
                )
            })
            .collect()
    }

    fn snapshot(kind: PostCollaboratorKind) -> PostCollaboratorsSnapshot {
        PostCollaboratorsSnapshot {
            items: vec![
                Self::row(
                    PostCollaboratorUser {
                        id: "debug-alex".to_owned(),
                        is_official: true,
                        name: "Alex Morgan".to_owned(),
                        tag: "alex".to_owned(),
                        ..Default::default()
                    },
                    kind,
                    PostCollaboratorStatus::Accepted,
                ),
                Self::row(
                    PostCollaboratorUser {
                        id: "debug-sam".to_owned(),
                        name: "Sam Rivera".to_owned(),
                        tag: "sam".to_owned(),
                        ..Default::default()
                    },
                    kind,
                    PostCollaboratorStatus::Accepted,
                ),
                Self::row(
                    PostCollaboratorUser {
                        id: "debug-jordan".to_owned(),
                        name: "Jordan Lee".to_owned(),
                        tag: "jordan".to_owned(),
                        ..Default::default()
                    },
                    kind,
                    PostCollaboratorStatus::Pending,
                ),
            ],
            has_more: false,
            is_loading: false,
            loaded_once: true,
        }
    }

    fn row(
        user: PostCollaboratorUser,
        kind: PostCollaboratorKind,
        status: PostCollaboratorStatus,
    ) -> PostCollaboratorRow {
        let mut row = PostCollaboratorRow::new(user);
        row.kind = kind.raw_value().to_owned();
        row.status = status.raw_value().to_owned();
        row
    }
}
