use gpui::App;

use crate::modules::post::stores::post_actions::post_actions::PostActionsStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::UserListRelationship;
use crate::modules::user::stores::user::user_services::user_services_hooks::{
    UserServicesHooks, UserServicesPostHooks,
};

pub fn install_user_services_hooks() {
    UserServicesHooks::install_post(UserServicesPostHooks {
        set_author_relationship_on_held_posts: Box::new(
            |author_id: &str, relationship: UserListRelationship, cx: &mut App| {
                PostServicesStore::shared(cx).update(cx, |services, cx| {
                    services.set_author_relationship_on_held_posts(author_id, relationship, cx)
                });
            },
        ),
        set_like_user_relationship: Box::new(|user_id: &str, relationship: UserListRelationship, cx: &mut App| {
            PostServicesStore::shared(cx)
                .update(cx, |services, cx| services.set_like_user_relationship(user_id, relationship, cx));
        }),
        set_author_relationship_action: Box::new(
            |author_id: &str, relationship: UserListRelationship, cx: &mut App| {
                PostActionsStore::shared(cx).update(cx, |actions, cx| {
                    actions.set_author_relationship_action(
                        author_id,
                        relationship.is_subbed,
                        relationship.is_subscriber,
                        relationship.is_friend,
                        cx,
                    )
                });
            },
        ),
        in_flight_own_publication_count: Box::new(|user_id: &str, cx: &mut App| {
            PostServicesStore::shared(cx).read(cx).in_flight_own_publication_count(user_id)
        }),
    });
}
