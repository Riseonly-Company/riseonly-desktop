use gpui::App;

use crate::modules::post::engine::rise_post_content_presentation::RisePostContentPresentation;
use crate::modules::post::engine::rise_post_engine_models::{PostCollaboratorsSnapshot, RisePostCollaboratorLimits};
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

impl PostServicesStore {
    pub fn post_collaborators_by_numeric_id(&self, post_id: i64, cx: &mut App) -> PostCollaboratorsSnapshot {
        self.post_collaborators(&post_id.to_string(), cx)
    }

    pub fn post_collaborators(&self, post_id: &str, cx: &mut App) -> PostCollaboratorsSnapshot {
        RisePostContentPresentation::shared(cx).read(cx).collaborators(post_id).clone()
    }

    pub fn collaborator_limits(&self) -> (i64, i64, i64) {
        (
            RisePostCollaboratorLimits::maximum_coauthors(),
            RisePostCollaboratorLimits::maximum_tagged_users(),
            RisePostCollaboratorLimits::maximum_tag_placements(),
        )
    }
}
