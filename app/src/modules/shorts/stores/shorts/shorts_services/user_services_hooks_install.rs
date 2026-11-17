
use gpui::App;

use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;
use crate::modules::user::stores::user::user_actions::user_types::UserListRelationship;
use crate::modules::user::stores::user::user_services::user_services_hooks::{
    UserServicesHooks, UserServicesShortsHooks,
};

pub fn install_user_services_hooks() {
    UserServicesHooks::install_shorts(UserServicesShortsHooks {
        set_author_relationship: Box::new(|author_id: &str, relationship: UserListRelationship, cx: &mut App| {
            ShortsServicesStore::shared(cx).update(cx, |shorts, cx| {
                shorts.set_author_relationship(author_id, relationship, cx);
            });
        }),
    });
}
