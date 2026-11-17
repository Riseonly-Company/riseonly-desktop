use std::collections::{HashMap, HashSet};

use gpui::{App, Context};
use rise_catalog::PostCollaboratorRejectionReason;
use rise_core::foundation::foundation_character::int_from_string;
use rise_core::foundation::foundation_text::trimming_whitespaces;
use rise_engine::CancellationError;
use rise_i18n::{FormatArg, tr, tr_args, tr_plural};

use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::post::engine::rise_post_engine_models::PostCollaboratorsSnapshot;
use crate::modules::post::engine::rise_post_repository::RisePostMutationError;
use crate::modules::post::shared::post_media_slides::PostMediaSlides;
use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_actions::post_types::{
    PostCollaboratorKind, PostCollaboratorRejection, PostCollaboratorStatus, PostCollaboratorUser, PostTagInputDraft,
};
use crate::modules::post::stores::post_actions::rise_post_publish_failure::RisePostPublishFailure;
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_guide_sandbox_hooks::post_guide_sandbox_hooks;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

type PostCollaborationOnResult = Box<dyn FnOnce(anyhow::Result<Vec<PostCollaboratorRejection>>, &mut App)>;

impl PostInteractionsStore {
    const COLLABORATOR_REJECTION_ORDER: [&'static str; 5] = [
        PostCollaboratorRejectionReason::Blocked.wire(),
        PostCollaboratorRejectionReason::Privacy.wire(),
        PostCollaboratorRejectionReason::Already.wire(),
        PostCollaboratorRejectionReason::NotFound.wire(),
        PostCollaboratorRejectionReason::Invalid.wire(),
    ];

    pub fn invite_coauthors(
        &mut self,
        post_id: &str,
        users: Vec<PostCollaboratorUser>,
        on_result: Option<PostCollaborationOnResult>,
        cx: &mut Context<Self>,
    ) {
        if users.is_empty() {
            return;
        }
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            if let Some(on_result) = on_result {
                on_result(Err(RisePostMutationError::Server("invalid_post_id".to_owned()).into()), cx);
            }
            return;
        };
        let (limit, _, _) = PostServicesStore::shared(cx).read(cx).collaborator_limits();
        if users.len() as i64 > limit {
            Self::announce_limit(limit, cx);
            if let Some(on_result) = on_result {
                on_result(Err(RisePostMutationError::InvalidDraft("too_many_coauthors".to_owned()).into()), cx);
            }
            return;
        }
        let this = cx.weak_entity();
        let people = users.clone();
        let finish: PostCollaborationOnResult = Box::new(move |result, cx: &mut App| {
            this.update(cx, |this, cx| this.finish_collaboration(result, &people, Some(limit), on_result, cx)).ok();
        });
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.invite_coauthors_action(numeric_post_id, users, Some(finish), cx);
        });
    }

    pub fn revoke_coauthor(&mut self, post_id: &str, user_id: &str, cx: &mut Context<Self>) {
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            return;
        };
        let finish = self.plain_finish(cx);
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.revoke_coauthor_action(numeric_post_id, user_id, Some(finish), cx);
        });
    }

    pub fn accept_coauthor_invite(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if let Some(sandbox) = post_guide_sandbox_hooks()
            && (sandbox.owns_post)(post_id, cx)
        {
            (sandbox.accept_coauthor_invite)(cx);
            return;
        }
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            return;
        };
        let finish = self.plain_finish(cx);
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.accept_coauthor_invite_action(numeric_post_id, Some(finish), cx);
        });
    }

    pub fn decline_coauthor_invite(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if let Some(sandbox) = post_guide_sandbox_hooks()
            && (sandbox.owns_post)(post_id, cx)
        {
            (sandbox.leave_coauthorship)(cx);
            return;
        }
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            return;
        };
        let finish = self.plain_finish(cx);
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.decline_coauthor_invite_action(numeric_post_id, Some(finish), cx);
        });
    }

    pub fn leave_coauthorship(&mut self, post_id: &str, cx: &mut Context<Self>) {
        if let Some(sandbox) = post_guide_sandbox_hooks()
            && (sandbox.owns_post)(post_id, cx)
        {
            (sandbox.leave_coauthorship)(cx);
            return;
        }
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            return;
        };
        let finish = self.plain_finish(cx);
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.leave_coauthorship_action(numeric_post_id, Some(finish), cx);
        });
    }

    pub fn set_coauthor_grid_visibility(&mut self, post_id: &str, show_in_grid: bool, cx: &mut Context<Self>) {
        if let Some(sandbox) = post_guide_sandbox_hooks()
            && (sandbox.owns_post)(post_id, cx)
        {
            (sandbox.set_coauthor_shows_in_grid)(show_in_grid, cx);
            return;
        }
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            return;
        };
        let finish = self.plain_finish(cx);
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.set_coauthor_grid_visibility_action(numeric_post_id, show_in_grid, Some(finish), cx);
        });
    }

    pub fn set_post_tags(
        &mut self,
        post_id: &str,
        tags: Vec<PostTagInputDraft>,
        on_result: Option<PostCollaborationOnResult>,
        cx: &mut Context<Self>,
    ) {
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            if let Some(on_result) = on_result {
                on_result(Err(RisePostMutationError::Server("invalid_post_id".to_owned()).into()), cx);
            }
            return;
        };
        let (_, tagged_users, tag_placements) = PostServicesStore::shared(cx).read(cx).collaborator_limits();
        if tags.len() as i64 > tagged_users {
            Self::announce_limit(tagged_users, cx);
            if let Some(on_result) = on_result {
                on_result(Err(RisePostMutationError::InvalidDraft("too_many_tagged_users".to_owned()).into()), cx);
            }
            return;
        }

        let deduped: Vec<PostTagInputDraft> = tags
            .iter()
            .map(|draft| PostTagInputDraft {
                user_id: draft.user_id.clone(),
                placements: PostMediaSlides::first_placement_per_slide(&draft.placements),
                user: draft.user.clone(),
            })
            .collect();
        if !deduped.iter().all(|draft| draft.placements.len() as i64 <= tag_placements) {
            Self::announce_limit(tag_placements, cx);
            if let Some(on_result) = on_result {
                on_result(Err(RisePostMutationError::InvalidDraft("too_many_tag_placements".to_owned()).into()), cx);
            }
            return;
        }
        let this = cx.weak_entity();
        let people: Vec<PostCollaboratorUser> = deduped.iter().filter_map(|draft| draft.user.clone()).collect();
        let finish: PostCollaborationOnResult = Box::new(move |result, cx: &mut App| {
            this.update(cx, |this, cx| this.finish_collaboration(result, &people, Some(tagged_users), on_result, cx))
                .ok();
        });
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.set_post_tags_action(numeric_post_id, deduped, Some(finish), cx);
        });
    }

    pub fn remove_post_tag(&mut self, post_id: &str, user_id: &str, cx: &mut Context<Self>) {
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            return;
        };
        let finish = self.plain_finish(cx);
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.remove_post_tag_action(numeric_post_id, user_id, Some(finish), cx);
        });
    }

    pub fn remove_own_tag(&mut self, post_id: &str, cx: &mut Context<Self>) {
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            return;
        };
        let finish = self.plain_finish(cx);
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.remove_own_tag_action(numeric_post_id, Some(finish), cx);
        });
    }

    pub fn load_post_collaborators(
        &mut self,
        post_id: &str,
        kind: Option<PostCollaboratorKind>,
        status: Option<PostCollaboratorStatus>,
        reset: bool,
        on_finished: Option<Box<dyn FnOnce(&mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let Some(numeric_post_id) = Self::numeric_post_id(post_id) else {
            return;
        };
        let snapshot = PostServicesStore::shared(cx).update(cx, |services, cx| services.post_collaborators(post_id, cx));
        if snapshot.is_loading {
            return;
        }
        PostActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.load_post_collaborators_action(numeric_post_id, kind, status, reset, on_finished, cx);
        });
    }

    pub fn post_collaborators(&self, post_id: &str, cx: &mut App) -> PostCollaboratorsSnapshot {
        PostServicesStore::shared(cx).update(cx, |services, cx| services.post_collaborators(post_id, cx))
    }

    pub fn coauthor_row_ids(&self, post_id: &str, cx: &mut App) -> HashSet<String> {
        PostServicesStore::shared(cx)
            .update(cx, |services, cx| services.post_collaborators(post_id, cx))
            .items
            .into_iter()
            .filter(|row| row.kind == PostCollaboratorKind::Coauthor.raw_value())
            .map(|row| row.user.id)
            .collect()
    }

    pub fn collaborator_limits(&self, cx: &mut App) -> (i64, i64, i64) {
        PostServicesStore::shared(cx).read(cx).collaborator_limits()
    }

    pub fn announce_collaborator_rejections(
        &self,
        rejections: &[PostCollaboratorRejection],
        people: &[PostCollaboratorUser],
        limit: Option<i64>,
        cx: &mut App,
    ) {
        let refused: Vec<PostCollaboratorRejection> = rejections
            .iter()
            .filter(|rejection| rejection.reason != PostCollaboratorRejectionReason::SelfInvite.wire())
            .cloned()
            .collect();
        if refused.is_empty() {
            return;
        }
        let limit = match limit {
            Some(limit) => limit,
            None => self.collaborator_limits(cx).0,
        };
        let (texts, unnamed) = Self::collaborator_rejection_lines(&refused, people, limit);
        let single = texts.len() == 1 && unnamed == 0;
        let quiet = single || texts.is_empty();
        let title = if single { texts[0].clone() } else { tr("post_collaborators_partially_rejected") };
        Notifier::send(
            title,
            NotifierSendOptions {
                content: if quiet { None } else { Some(texts.join("\n")) },
                r#type: NotifierType::Warning,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("person.crop.circle.badge.exclamationmark".to_owned()),
                duration: if quiet { 3.2 } else { 5.0 },
                ..Default::default()
            },
            cx,
        );
    }

    fn collaborator_rejection_lines(
        rejections: &[PostCollaboratorRejection],
        people: &[PostCollaboratorUser],
        limit: i64,
    ) -> (Vec<String>, i64) {
        let mut names_by_reason: HashMap<&str, Vec<String>> = HashMap::new();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut unnamed = 0;
        let mut hit_limit = false;
        for rejection in rejections {
            if !seen.insert(rejection.user_id.as_str()) {
                continue;
            }
            if rejection.reason == PostCollaboratorRejectionReason::Limit.wire() {
                hit_limit = true;
                continue;
            }
            let reason = Self::COLLABORATOR_REJECTION_ORDER
                .iter()
                .copied()
                .find(|known| *known == rejection.reason)
                .unwrap_or(PostCollaboratorRejectionReason::Invalid.wire());
            let Some(person) = people.iter().find(|person| person.id == rejection.user_id) else {
                unnamed += 1;
                continue;
            };
            let name = if person.name.is_empty() { format!("@{}", person.tag) } else { person.name.clone() };
            names_by_reason.entry(reason).or_default().push(name);
        }
        let mut texts: Vec<String> = Self::COLLABORATOR_REJECTION_ORDER
            .iter()
            .filter_map(|reason| {
                let names = names_by_reason.get(reason)?;
                Some(tr_args(
                    Self::collaborator_rejection_message_key(reason),
                    &[FormatArg::from(names.join(", "))],
                ))
            })
            .collect();
        if hit_limit {
            texts.push(tr_args("post_collaborator_rejected_limit", &[FormatArg::from(limit)]));
        }
        (texts, unnamed)
    }

    pub fn collaborator_rejection_message_key(reason: &str) -> &'static str {
        match PostCollaboratorRejectionReason::from_wire(reason) {
            Some(PostCollaboratorRejectionReason::Blocked) => "post_collaborator_rejected_blocked",
            Some(PostCollaboratorRejectionReason::Privacy) => "post_collaborator_rejected_privacy",
            Some(PostCollaboratorRejectionReason::Already) => "post_collaborator_rejected_already",
            Some(PostCollaboratorRejectionReason::Limit) => "post_collaborator_rejected_limit",
            Some(PostCollaboratorRejectionReason::NotFound) => "post_collaborator_rejected_not_found",
            _ => "post_collaborator_rejected_invalid",
        }
    }

    fn numeric_post_id(post_id: &str) -> Option<i64> {
        let value = int_from_string(trimming_whitespaces(post_id))?;
        (value > 0).then_some(value)
    }

    fn plain_finish(&self, cx: &mut Context<Self>) -> PostCollaborationOnResult {
        let this = cx.weak_entity();
        Box::new(move |result, cx: &mut App| {
            this.update(cx, |this, cx| this.finish_collaboration(result, &[], None, None, cx)).ok();
        })
    }

    fn finish_collaboration(
        &mut self,
        result: anyhow::Result<Vec<PostCollaboratorRejection>>,
        people: &[PostCollaboratorUser],
        limit: Option<i64>,
        on_result: Option<PostCollaborationOnResult>,
        cx: &mut Context<Self>,
    ) {
        match &result {
            Ok(rejections) => {
                self.announce_collaborator_rejections(rejections, people, limit, cx);
            }
            Err(error) => {
                if error.downcast_ref::<CancellationError>().is_none() {
                    Notifier::send(
                        tr(&RisePostPublishFailure::message(error)),
                        NotifierSendOptions {
                            r#type: NotifierType::Error,
                            position: NotifierPosition::Bottom,
                            icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                            ..Default::default()
                        },
                        cx,
                    );
                }
            }
        }
        if let Some(on_result) = on_result {
            on_result(result, cx);
        }
    }

    fn announce_limit(limit: i64, cx: &mut App) {
        Notifier::send(
            tr_plural("people_picker_limit_reached", limit),
            NotifierSendOptions {
                r#type: NotifierType::Warning,
                position: NotifierPosition::Bottom,
                icon: NotifierIcon::System("person.2.slash".to_owned()),
                ..Default::default()
            },
            cx,
        );
    }
}
