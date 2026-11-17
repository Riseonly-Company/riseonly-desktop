use gpui::{App, Window};
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::modules::user::components::user_relationship_list::UserRelationshipList;
use crate::modules::user::stores::user::user_actions::user_types::UserListKind;

#[derive(IntoElement)]
pub struct UserFriends {
    pub user_id: Option<String>,
}

impl UserFriends {
    pub fn new(user_id: Option<String>) -> Self {
        Self { user_id }
    }
}

impl RenderOnce for UserFriends {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        UserRelationshipList::new(self.user_id, UserListKind::Friends, tr("user_friends_text"))
    }
}
