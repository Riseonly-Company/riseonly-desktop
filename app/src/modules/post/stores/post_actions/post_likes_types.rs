use crate::modules::post::stores::post_actions::post_types::{AuthorInfo, AuthorInfoMore};
use crate::modules::user::stores::user::user_actions::user_types::UserListRelationship;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostLikeUser {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub logo: String,
    pub is_premium: bool,
    pub is_official: bool,
    pub is_friend: bool,
    pub is_subbed: bool,
    pub is_subscriber: bool,
    pub liked_at_ms: i64,
    pub premium_badge_emoji: Option<String>,
    pub name_color: Option<String>,
}

impl PostLikeUser {
    pub fn author(&self) -> AuthorInfo {
        AuthorInfo {
            id: Some(self.id.clone()),
            name: self.name.clone(),
            tag: self.tag.clone(),
            more: AuthorInfoMore {
                logo: self.logo.clone(),
                is_premium: self.is_premium,
                is_official: self.is_official,
                premium_badge_emoji: self.premium_badge_emoji.clone(),
                name_color: self.name_color.clone(),
                ..AuthorInfoMore::default()
            },
            is_subscribed: self.is_subbed,
            is_friend: self.is_friend,
            ..AuthorInfo::default()
        }
    }

    pub fn relationship(&self) -> UserListRelationship {
        UserListRelationship {
            is_subbed: self.is_subbed,
            is_subscriber: self.is_subscriber,
            is_friend: self.is_friend,
        }
    }
}
