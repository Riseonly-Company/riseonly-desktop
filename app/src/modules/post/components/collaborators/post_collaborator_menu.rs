use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::App;
use rise_i18n::tr;

use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierSendOptions, NotifierType};
use crate::core::widgets::sheets::people_picker_sheet::{PeoplePickerHostContext, PeoplePickerMode, PeoplePickerStore};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::post::components::collaborators::post_collaborators_presenter::{
    PostCollaboratorConfirmKind, PostCollaboratorsPresenter,
};
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostCollaboratorKind, PostCollaboratorRejection, PostCollaboratorUser, PostTagInputDraft,
    PostTagPlacement,
};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;

#[derive(Clone)]
pub struct PostCollaboratorMenuItem {
    pub id: String,
    pub title: String,
    pub system_image: String,
    pub destructive: bool,
    pub action: Rc<dyn Fn(&mut App)>,
}

impl PostCollaboratorMenuItem {
    fn new(id: &str, title: String, system_image: &str, action: impl Fn(&mut App) + 'static) -> Self {
        Self {
            id: id.to_owned(),
            title,
            system_image: system_image.to_owned(),
            destructive: false,
            action: Rc::new(action),
        }
    }

    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}

/// coauthor and tag items for every surface
/// native post menu and the reel more sheet
/// one list so feed and reels cant drift apart
pub enum PostCollaboratorMenu {}

impl PostCollaboratorMenu {
    pub fn items(post: &GetPostFeedResponse, is_mine: bool, cx: &mut App) -> Vec<PostCollaboratorMenuItem> {
        let mut items: Vec<PostCollaboratorMenuItem> = Vec::new();
        let post_id = post.id.clone();

        let acts_as_author = is_mine
            || GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.viewer_acts_as_author(&post_id, cx));
        if acts_as_author {
            let shared_post = Rc::new(post.clone());
            let invite_post = shared_post.clone();
            items.push(PostCollaboratorMenuItem::new(
                "coauthor_invite",
                tr("post_coauthor_invite_action"),
                "person.badge.plus",
                move |cx| Self::present_coauthor_picker(&invite_post, cx),
            ));
            let manage_post_id = post_id.clone();
            items.push(PostCollaboratorMenuItem::new(
                "coauthors_manage",
                tr("post_coauthors_manage_action"),
                "person.2",
                move |cx| {
                    PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| {
                        presenter.present_coauthors(&manage_post_id, true, cx);
                    });
                },
            ));
            if Self::has_media(post) {
                let tag_post = shared_post;
                items.push(PostCollaboratorMenuItem::new(
                    "tag_people",
                    tr("post_tag_people_action"),
                    "person.crop.rectangle",
                    move |cx| Self::present_tag_picker(&tag_post, cx),
                ));
                let tagged_post_id = post_id;
                items.push(PostCollaboratorMenuItem::new(
                    "tagged_people_manage",
                    tr("post_tagged_people_manage_action"),
                    "person.crop.circle.badge.checkmark",
                    move |cx| {
                        PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| {
                            presenter.present_tagged_people(&tagged_post_id, true, cx);
                        });
                    },
                ));
            }
            return items;
        }

        if post.is_pending_coauthor_invite() {
            let accept_post_id = post_id.clone();
            items.push(PostCollaboratorMenuItem::new(
                "coauthor_accept",
                tr("post_coauthor_accept_button"),
                "checkmark.circle",
                move |cx| Self::ask(PostCollaboratorConfirmKind::AcceptInvite, &accept_post_id, cx),
            ));
            let decline_post_id = post_id.clone();
            items.push(PostCollaboratorMenuItem::new(
                "coauthor_decline",
                tr("post_coauthor_decline_action"),
                "xmark.circle",
                move |cx| {
                    PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.decline_coauthor_invite(&decline_post_id, cx);
                    });
                },
            ));
        }

        if post.is_accepted_coauthor() {
            let shown = post.coauthor_shows_in_grid();
            let grid_post_id = post_id.clone();
            items.push(PostCollaboratorMenuItem::new(
                "coauthor_grid_visibility",
                if shown { tr("post_coauthor_hide_from_grid_action") } else { tr("post_coauthor_show_in_grid_action") },
                if shown { "eye.slash" } else { "eye" },
                move |cx| {
                    PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.set_coauthor_grid_visibility(&grid_post_id, !shown, cx);
                    });
                },
            ));
            let leave_post_id = post_id.clone();
            items.push(
                PostCollaboratorMenuItem::new(
                    "coauthor_leave",
                    tr("post_coauthor_leave_action"),
                    "person.2.slash",
                    move |cx| Self::ask(PostCollaboratorConfirmKind::LeaveCoauthorship, &leave_post_id, cx),
                )
                .destructive(),
            );
        }

        if post.is_tagged_viewer() {
            let remove_post_id = post_id.clone();
            items.push(
                PostCollaboratorMenuItem::new(
                    "tag_remove_self",
                    tr("post_tag_remove_self_action"),
                    "person.crop.square.badge.xmark",
                    move |cx| Self::ask(PostCollaboratorConfirmKind::RemoveOwnTag, &remove_post_id, cx),
                )
                .destructive(),
            );
        }

        if post.coauthors_count > 0 {
            let view_post_id = post_id.clone();
            items.push(PostCollaboratorMenuItem::new(
                "coauthors_view",
                tr("post_coauthors_title"),
                "person.2",
                move |cx| {
                    PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| {
                        presenter.present_coauthors(&view_post_id, false, cx);
                    });
                },
            ));
        }
        if post.tagged_users_count > 0 || !post.tagged_users.is_empty() {
            items.push(PostCollaboratorMenuItem::new(
                "tagged_people_view",
                tr("post_tagged_people_title"),
                "person.crop.circle",
                move |cx| {
                    PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| {
                        presenter.present_tagged_people(&post_id, false, cx);
                    });
                },
            ));
        }

        items
    }

    pub fn has_media(post: &GetPostFeedResponse) -> bool {
        !post.images.is_empty()
            || post.video.as_ref().is_some_and(|video| !video.is_empty())
            || post.is_short
            || post.is_image_reel
    }

    fn ask(kind: PostCollaboratorConfirmKind, post_id: &str, cx: &mut App) {
        PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| presenter.ask(kind, post_id, cx));
    }

    fn present_coauthor_picker(post: &GetPostFeedResponse, cx: &mut App) {
        let post_id = post.id.clone();
        let interactions = PostInteractionsStore::shared(cx);
        let mut excluded: HashSet<String> = HashSet::new();
        excluded.insert(post.author.id.clone().unwrap_or_else(|| post.author_id.clone()));
        for user in &post.coauthors {
            excluded.insert(user.id.clone());
        }
        excluded.extend(interactions.update(cx, |interactions, cx| interactions.coauthor_row_ids(&post_id, cx)));
        let (coauthors_limit, _, _) = PostServicesStore::shared(cx).read(cx).collaborator_limits();
        let done_interactions = interactions.clone();
        let done_post_id = post_id.clone();
        let session = PeoplePickerStore::shared(cx).update(cx, |store, cx| {
            store.present(
                PeoplePickerMode::SearchAll,
                coauthors_limit,
                Vec::new(),
                excluded,
                tr("post_coauthor_invite_action"),
                None,
                PeoplePickerHostContext::Root,
                move |users: Vec<PostCollaboratorUser>, cx: &mut App| {
                    if users.is_empty() {
                        return;
                    }
                    done_interactions.update(cx, |interactions, cx| {
                        interactions.invite_coauthors(
                            &done_post_id,
                            users,
                            Some(Box::new(PostCollaboratorMutationFeedback::report_invite)),
                            cx,
                        );
                    });
                },
                cx,
            )
        });
        let Some(session) = session else {
            return;
        };
        let finished_interactions = interactions.clone();
        let finished_post_id = post_id.clone();
        interactions.update(cx, |interactions, cx| {
            interactions.load_post_collaborators(
                &post_id,
                Some(PostCollaboratorKind::Coauthor),
                None,
                true,
                Some(Box::new(move |cx: &mut App| {
                    let ids = finished_interactions
                        .update(cx, |interactions, cx| interactions.coauthor_row_ids(&finished_post_id, cx));
                    PeoplePickerStore::shared(cx).update(cx, |store, cx| store.exclude(ids, session, cx));
                })),
                cx,
            );
        });
    }

    fn present_tag_picker(post: &GetPostFeedResponse, cx: &mut App) {
        let post_id = post.id.clone();
        let mut placements_by_user: HashMap<String, Vec<PostTagPlacement>> = HashMap::new();
        for tagged in &post.tagged_users {
            let Some(user) = &tagged.user else {
                continue;
            };
            placements_by_user.insert(user.id.clone(), tagged.placements.clone());
        }
        let (_, tagged_users_limit, _) = PostServicesStore::shared(cx).read(cx).collaborator_limits();
        let preselected: Vec<PostCollaboratorUser> =
            post.tagged_users.iter().filter_map(|tagged| tagged.user.clone()).collect();
        let excluded: HashSet<String> =
            HashSet::from([post.author.id.clone().unwrap_or_else(|| post.author_id.clone())]);
        PeoplePickerStore::shared(cx).update(cx, |store, cx| {
            store.present(
                PeoplePickerMode::Friends,
                tagged_users_limit,
                preselected,
                excluded,
                tr("post_tag_people_action"),
                Some(tr("people_picker_friends_only_hint")),
                PeoplePickerHostContext::Root,
                move |users: Vec<PostCollaboratorUser>, cx: &mut App| {
                    let drafts: Vec<PostTagInputDraft> = users
                        .into_iter()
                        .map(|user| PostTagInputDraft {
                            user_id: user.id.clone(),
                            placements: placements_by_user.get(&user.id).cloned().unwrap_or_default(),
                            user: Some(user),
                        })
                        .collect();
                    PostInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                        interactions.set_post_tags(&post_id, drafts, None, cx);
                    });
                },
                cx,
            )
        });
    }
}

pub enum PostCollaboratorMutationFeedback {}

impl PostCollaboratorMutationFeedback {
    pub fn report_invite(result: anyhow::Result<Vec<PostCollaboratorRejection>>, cx: &mut App) {
        if !result.is_ok_and(|rejections| rejections.is_empty()) {
            return;
        }
        Notifier::send(
            tr("post_coauthor_invite_sent"),
            NotifierSendOptions { r#type: NotifierType::Success, ..Default::default() },
            cx,
        );
    }
}
