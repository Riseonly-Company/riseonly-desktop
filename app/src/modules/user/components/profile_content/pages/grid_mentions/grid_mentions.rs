use std::rc::Rc;

use gpui::{App, ElementId, Window, black, white};
use rise_i18n::tr;
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;

use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::post::components::collaborators::post_collaborators_presenter::{
    PostCollaboratorConfirmKind, PostCollaboratorsPresenter,
};
use crate::modules::post::engine::rise_post_engine_models::RisePostProfileKind;
use crate::modules::post::stores::post_actions::post_types::{GetPostFeedResponse, PostTaggedUserIdentifiable};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::user::components::profile_content::pages::grid_favorites::grid_favorites::ProfilePostGrid;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::ProfileGridMenuAsOwnerKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProfileMentionRole {
    Coauthor,
    Tagged,
}

impl ProfileMentionRole {
    fn symbol(self) -> &'static str {
        match self {
            Self::Coauthor => "person.2.fill",
            Self::Tagged => "person.crop.square",
        }
    }

    fn is_filled(self) -> bool {
        self == Self::Coauthor
    }
}

#[derive(IntoElement)]
pub struct GridMentions {
    id: ElementId,
    pub profile_tag: String,
    pub profile_user_id: String,
}

impl GridMentions {
    pub fn new(id: impl Into<ElementId>, profile_tag: impl Into<String>, profile_user_id: impl Into<String>) -> Self {
        Self { id: id.into(), profile_tag: profile_tag.into(), profile_user_id: profile_user_id.into() }
    }

    fn role_badge(role: ProfileMentionRole) -> AnyElement {
        div()
            .absolute()
            .left(px(6.0))
            .bottom(px(6.0))
            .px(px(6.0))
            .py(px(4.0))
            .rounded_full()
            .bg(black().opacity(0.5))
            .children(IconUi::sized(role.symbol(), px(9.0), white(), role.is_filled()))
            .into_any_element()
    }

    fn role(post: &GetPostFeedResponse, is_owner: bool, profile_user_id: &str) -> Option<ProfileMentionRole> {
        if is_owner {
            if post.is_accepted_coauthor() {
                return Some(ProfileMentionRole::Coauthor);
            }
            return post.is_tagged_viewer().then_some(ProfileMentionRole::Tagged);
        }
        if post.coauthors.iter().any(|coauthor| coauthor.id == profile_user_id) {
            return Some(ProfileMentionRole::Coauthor);
        }
        if post.tagged_users.iter().any(|user| user.id() == profile_user_id) {
            return Some(ProfileMentionRole::Tagged);
        }
        None
    }

    fn mention_actions(post: &GetPostFeedResponse, is_owner: bool) -> Vec<HoldContextMenuAction> {
        let mut items = vec![
            HoldContextMenuAction::new("open", tr("ctx_open")).system_image("arrow.up.right.square"),
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
        ];
        if !is_owner {
            return items;
        }
        if post.is_accepted_coauthor() {
            let shown = post.coauthor_shows_in_grid();
            let title =
                if shown { tr("post_coauthor_hide_from_grid_action") } else { tr("post_coauthor_show_in_grid_action") };
            items.push(
                HoldContextMenuAction::new("coauthor_grid_visibility", title)
                    .system_image(if shown { "eye.slash" } else { "eye" }),
            );
            items.push(
                HoldContextMenuAction::new("coauthor_leave", tr("post_coauthor_leave_action"))
                    .system_image("person.badge.minus")
                    .destructive(true),
            );
        }
        if post.is_tagged_viewer() {
            items.push(
                HoldContextMenuAction::new("tag_remove_self", tr("post_tag_remove_self_action"))
                    .system_image("person.crop.circle.badge.xmark")
                    .destructive(true),
            );
        }
        items
    }

    fn handle_action(id: &str, post: &GetPostFeedResponse, cx: &mut App) {
        match id {
            "coauthor_grid_visibility" => {
                let show_in_grid = !post.coauthor_shows_in_grid();
                PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.set_coauthor_grid_visibility(&post.id, show_in_grid, cx);
                });
            }
            "coauthor_leave" => Self::ask(PostCollaboratorConfirmKind::LeaveCoauthorship, &post.id, cx),
            "tag_remove_self" => Self::ask(PostCollaboratorConfirmKind::RemoveOwnTag, &post.id, cx),
            _ => {}
        }
    }

    fn ask(kind: PostCollaboratorConfirmKind, post_id: &str, cx: &mut App) {
        PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| presenter.ask(kind, post_id, cx));
    }
}

impl RenderOnce for GridMentions {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_owner = env::<ProfileGridMenuAsOwnerKey>(cx)
            || AuthServicesStore::shared(cx).read(cx).is_current_user(&self.profile_user_id);
        let badge_user_id = self.profile_user_id.clone();
        ProfilePostGrid {
            id: self.id,
            profile_tag: self.profile_tag,
            profile_user_id: self.profile_user_id,
            kind: RisePostProfileKind::Mentions,
            empty_icon: "person.crop.square".to_owned(),
            empty_text: tr("profile_mentions_empty"),
            actions: Rc::new(move |post: &GetPostFeedResponse| Self::mention_actions(post, is_owner)),
            on_extra_action: Rc::new(Self::handle_action),
            cell_overlay: Some(Rc::new(move |post: &GetPostFeedResponse| {
                Self::role(post, is_owner, &badge_user_id).map(Self::role_badge)
            })),
        }
    }
}
