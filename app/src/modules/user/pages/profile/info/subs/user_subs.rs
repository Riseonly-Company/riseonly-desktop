use gpui::{App, Window};
use rise_i18n::tr;
use rise_ui::swiftui::prelude::*;

use crate::modules::user::components::user_relationship_list::UserRelationshipList;
use crate::modules::user::stores::user::user_actions::user_types::UserListKind;

#[derive(IntoElement)]
pub struct UserSubs {
    pub user_id: Option<String>,
}

impl UserSubs {
    pub fn new(user_id: Option<String>) -> Self {
        Self { user_id }
    }
}

impl RenderOnce for UserSubs {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        UserRelationshipList::new(self.user_id, UserListKind::Subscriptions, tr("user_subs_text"))
    }
}
